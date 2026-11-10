//! 独立、可关闭的桌面 HUD。位置以物理像素保存，恢复时按当前显示器工作区修正。
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalRect, PhysicalSize, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder,
};

pub const LABEL: &str = "agent-hud";
const WIDTH: f64 = 400.0;
const COMPACT_HEIGHT: f64 = 72.0;
const EXPANDED_HEIGHT: f64 = 520.0;
const MAX_TOOLS: u8 = crate::config::AI_TOOL_KEYS.len() as u8;

// 固定预留拖动、浏览器与代理入口；AI 工具按 44px 圆环 + 间隔累加。
fn compact_width(tool_count: u8) -> f64 {
    if tool_count == 0 {
        312.0
    } else {
        176.0 + 48.0 * tool_count.min(MAX_TOOLS) as f64
    }
}
fn side_height(tool_count: u8) -> f64 {
    if tool_count == 0 {
        320.0
    } else {
        184.0 + 56.0 * tool_count.min(MAX_TOOLS) as f64
    }
}

#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum DockEdge {
    #[default]
    Top,
    Left,
    Right,
    Free,
}
impl DockEdge {
    fn side(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Preferences {
    enabled: bool,
    always_on_top: bool,
    auto_dock_top: bool,
    dock_edge: DockEdge,
    tool_scope: ToolScope,
    position: Option<PhysicalPosition<i32>>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            enabled: true,
            always_on_top: true,
            auto_dock_top: true,
            dock_edge: DockEdge::Top,
            tool_scope: ToolScope::Installed,
            position: None,
        }
    }
}
impl Preferences {
    fn edge(&self) -> DockEdge {
        if self.auto_dock_top {
            self.dock_edge
        } else {
            DockEdge::Free
        }
    }
}
#[derive(Clone, Copy, Default, Serialize, Deserialize, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum ToolScope {
    Online,
    #[default]
    Installed,
}
#[derive(Default)]
pub struct Runtime {
    preferences: Preferences,
    expanded: bool,
    ready: bool,
    destroying: bool,
    tool_count: u8,
    // 窗口是否已经由 layout 摆过。没摆过才用保存的锚点做首次定位。
    placed: bool,
    strip_offset: PhysicalPosition<i32>,
    scale: f64,
}
pub type State = Mutex<Runtime>;
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    enabled: bool,
    always_on_top: bool,
    auto_dock_top: bool,
    dock_edge: DockEdge,
    strip_offset_x: f64,
    strip_offset_y: f64,
    tool_scope: ToolScope,
    expanded: bool,
    compact_width: f64,
    side_height: f64,
}

fn status(app: &AppHandle) -> Status {
    let state = app.state::<State>();
    let state = state.lock();
    let scale = if state.scale > 0.0 { state.scale } else { 1.0 };
    Status {
        enabled: state.preferences.enabled,
        always_on_top: state.preferences.always_on_top,
        auto_dock_top: state.preferences.auto_dock_top,
        dock_edge: state.preferences.edge(),
        strip_offset_x: state.strip_offset.x as f64 / scale,
        strip_offset_y: state.strip_offset.y as f64 / scale,
        tool_scope: state.preferences.tool_scope,
        expanded: state.expanded,
        compact_width: compact_width(state.tool_count),
        side_height: side_height(state.tool_count),
    }
}
fn allowed(window: &WebviewWindow) -> Result<(), String> {
    if ["main", LABEL, "tray-menu"].contains(&window.label()) {
        Ok(())
    } else {
        Err("此窗口不能操作悬浮条".into())
    }
}
fn hud_only(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == LABEL {
        Ok(())
    } else {
        Err("此操作仅用于悬浮条".into())
    }
}
fn save(app: &AppHandle, preferences: &Preferences) -> Result<(), String> {
    let directory = crate::data_dir(app).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    crate::config::atomic_write(
        &directory.join("agent-hud.json"),
        &serde_json::to_vec(preferences).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}
fn update_preferences(
    app: &AppHandle,
    update: impl FnOnce(&mut Preferences),
) -> Result<(), String> {
    let state = app.state::<State>();
    let mut state = state.lock();
    let mut preferences = state.preferences.clone();
    update(&mut preferences);
    // 拖动保存与主界面开关可能同时发生，持锁完成读改写，避免旧位置覆盖新开关。
    save(app, &preferences)?;
    state.preferences = preferences;
    Ok(())
}
fn changed(app: &AppHandle) {
    let _ = app.emit("agent-hud-changed", status(app));
}

/// 支持负坐标副屏、任务栏避让和屏幕尺寸改变。贴边阈值按显示器 DPI 缩放。
fn constrain(
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    area: &PhysicalRect<i32, u32>,
    scale: f64,
    snap: bool,
) -> PhysicalPosition<i32> {
    let gap = (6.0 * scale).round() as i32;
    let left = area.position.x + gap;
    let top = area.position.y + gap;
    let right = (area.position.x + area.size.width as i32 - size.width as i32 - gap).max(left);
    let bottom = (area.position.y + area.size.height as i32 - size.height as i32 - gap).max(top);
    let mut x = position.x.clamp(left, right);
    let mut y = position.y.clamp(top, bottom);
    let threshold = (24.0 * scale).round() as i32;
    if snap {
        if x - left <= threshold {
            x = left;
        } else if right - x <= threshold {
            x = right;
        }
        if y - top <= threshold {
            y = top;
        } else if bottom - y <= threshold {
            y = bottom;
        }
    }
    PhysicalPosition::new(x, y)
}
fn size(
    area: &PhysicalRect<i32, u32>,
    scale: f64,
    expanded: bool,
    tool_count: u8,
    edge: DockEdge,
) -> PhysicalSize<u32> {
    let margin = (12.0 * scale).round() as u32;
    let (width, height) = if edge.side() {
        (
            if expanded {
                WIDTH + COMPACT_HEIGHT + 8.0
            } else {
                COMPACT_HEIGHT
            },
            if expanded {
                EXPANDED_HEIGHT.max(side_height(tool_count))
            } else {
                side_height(tool_count)
            },
        )
    } else {
        (
            if expanded {
                WIDTH.max(compact_width(tool_count))
            } else {
                compact_width(tool_count)
            },
            if expanded {
                EXPANDED_HEIGHT
            } else {
                COMPACT_HEIGHT
            },
        )
    };
    // 虚拟屏或已断开的显示器可能给出 0 尺寸工作区；此时至少给一个可用下限，
    // 否则会得到一个 1×1 的悬浮条窗口（WebView 不再分配渲染进程，看起来像“消失”）。
    let floor_width = ((120.0 * scale).round() as u32).max(120);
    let floor_height = ((40.0 * scale).round() as u32).max(40);
    let w = ((width * scale).round() as u32).min(area.size.width.saturating_sub(margin));
    let h = ((height * scale).round() as u32).min(area.size.height.saturating_sub(margin));
    // 下限只兜住退化工作区或钳位后归零，不能把正常设计尺寸（如侧边收起的 72）抬到 120。
    PhysicalSize::new(
        if w == 0 || area.size.width < floor_width {
            w.max(floor_width)
        } else {
            w
        },
        if h == 0 || area.size.height < floor_height {
            h.max(floor_height)
        } else {
            h
        },
    )
}

/// 显示器工作区要真的放得下悬浮条：断开的显示器/虚拟屏会返回 0 尺寸。
fn usable_monitor(area: &PhysicalRect<i32, u32>) -> bool {
    area.size.width >= 320 && area.size.height >= 200
}
fn anchor_position(
    position: PhysicalPosition<i32>,
    window: PhysicalSize<u32>,
    area: &PhysicalRect<i32, u32>,
    scale: f64,
    edge: DockEdge,
    snap: bool,
) -> PhysicalPosition<i32> {
    let mut position = constrain(position, window, area, scale, snap);
    let gap = (6.0 * scale).round() as i32;
    match edge {
        DockEdge::Top => position.y = area.position.y + gap,
        DockEdge::Left => position.x = area.position.x + gap,
        DockEdge::Right => {
            position.x = (area.position.x + area.size.width as i32 - window.width as i32 - gap)
                .max(area.position.x + gap)
        }
        DockEdge::Free => {}
    }
    position
}

fn drop_edge(
    position: PhysicalPosition<i32>,
    window: PhysicalSize<u32>,
    area: &PhysicalRect<i32, u32>,
    scale: f64,
    current: DockEdge,
) -> DockEdge {
    let position = constrain(position, window, area, scale, false);
    let gap = (6.0 * scale).round() as i32;
    let threshold = (24.0 * scale).round() as i32;
    let distances = [
        (DockEdge::Top, (position.y - area.position.y - gap).abs()),
        (DockEdge::Left, (position.x - area.position.x - gap).abs()),
        (
            DockEdge::Right,
            (area.position.x + area.size.width as i32 - gap - position.x - window.width as i32)
                .abs(),
        ),
    ];
    distances
        .into_iter()
        .filter(|(_, distance)| *distance <= threshold)
        .min_by_key(|(edge, distance)| (*distance, if *edge == current { 0 } else { 1 }))
        .map(|(edge, _)| edge)
        // 离上/左/右都超过阈值就保持落点：不认识的落点不该被改写成"回到顶部"。
        .unwrap_or(DockEdge::Free)
}

fn geometry(
    anchor: PhysicalPosition<i32>,
    area: &PhysicalRect<i32, u32>,
    scale: f64,
    expanded: bool,
    tool_count: u8,
    edge: DockEdge,
) -> (
    PhysicalPosition<i32>,
    PhysicalSize<u32>,
    PhysicalPosition<i32>,
) {
    let compact = size(area, scale, false, tool_count, edge);
    let target = size(area, scale, expanded, tool_count, edge);
    let compact_anchor = anchor_position(anchor, compact, area, scale, edge, false);
    let position = anchor_position(compact_anchor, target, area, scale, edge, false);
    (
        position,
        target,
        PhysicalPosition::new(compact_anchor.x - position.x, compact_anchor.y - position.y),
    )
}
/// 窗口被 layout 摆过之后，真实外框位置就是唯一的位置来源；
/// `Preferences::position` 只是它落盘后的副本，供下次启动首次摆放。
struct Placement {
    anchor: PhysicalPosition<i32>,
    edge: DockEdge,
    area: PhysicalRect<i32, u32>,
    scale: f64,
}

fn placement_for_strip(
    strip: PhysicalPosition<i32>,
    preferences: &Preferences,
    tool_count: u8,
    area: PhysicalRect<i32, u32>,
    scale: f64,
    redock: bool,
) -> Placement {
    // SetWindowPos 也会产生 onMoved。排版已保存同一锚点时，这次 snap 不是用户拖动。
    let redock = redock && preferences.position != Some(strip);
    let current = preferences.edge();
    let compact = size(&area, scale, false, tool_count, current);
    let edge = if redock && preferences.auto_dock_top {
        drop_edge(strip, compact, &area, scale, current)
    } else {
        current
    };
    Placement {
        anchor: anchor_position(
            strip,
            size(&area, scale, false, tool_count, edge),
            &area,
            scale,
            edge,
            redock,
        ),
        edge,
        area,
        scale,
    }
}

/// 由窗口当前外框位置反算紧凑栏锚点与停靠边。
/// 展开时窗口比紧凑栏大，`strip_offset` 把外框换算回用户抓住的那条紧凑栏。
fn placement_from_window(
    app: &AppHandle,
    window: &WebviewWindow,
    preferences: &Preferences,
    tool_count: u8,
    strip_offset: PhysicalPosition<i32>,
    redock: bool,
) -> Result<Placement, String> {
    let position = window.outer_position().map_err(|e| e.to_string())?;
    let strip = PhysicalPosition::new(position.x + strip_offset.x, position.y + strip_offset.y);
    let monitor = app
        .monitor_from_point((strip.x + 20) as f64, (strip.y + 20) as f64)
        .map_err(|e| e.to_string())?
        .or(window.current_monitor().map_err(|e| e.to_string())?)
        .or(app.primary_monitor().map_err(|e| e.to_string())?)
        .ok_or("未找到可用显示器")?;
    let area = *monitor.work_area();
    let scale = monitor.scale_factor();
    // 停靠方向只由实际移动后的 snap 决定。展开或筛选变更沿用现有方向与紧凑栏锚点。
    // strip_offset 已能还原底部向上展开的锚点，无需再按窗口底边推算一次。
    Ok(placement_for_strip(
        strip,
        preferences,
        tool_count,
        area,
        scale,
        redock,
    ))
}

/// 首次摆放（新建窗口 / 恢复默认位置）：沿用保存的锚点，没有就落在主屏顶部居中。
fn initial_placement(
    app: &AppHandle,
    preferences: &Preferences,
    tool_count: u8,
) -> Result<Placement, String> {
    // 保存的位置可能指向已经断开的显示器：只有坐标所在显示器仍可用时才沿用。
    let saved_monitor = preferences.position.and_then(|p| {
        app.monitor_from_point(p.x as f64 + 20.0, p.y as f64 + 20.0)
            .ok()
            .flatten()
    });
    let saved = preferences.position.filter(|_| {
        saved_monitor
            .as_ref()
            .map(|monitor| usable_monitor(&monitor.work_area()))
            .unwrap_or(false)
    });
    // 没有保存位置时落在**主显示器**顶部居中；主屏不可用才退到任意可用显示器。
    let monitor = saved_monitor
        .filter(|monitor| usable_monitor(&monitor.work_area()))
        .or_else(|| {
            app.primary_monitor()
                .ok()
                .flatten()
                .filter(|monitor| usable_monitor(&monitor.work_area()))
        })
        .or_else(|| {
            app.available_monitors().ok().and_then(|monitors| {
                monitors
                    .into_iter()
                    .find(|m| usable_monitor(&m.work_area()))
            })
        })
        .ok_or("未找到可用显示器")?;
    let area = *monitor.work_area();
    let scale = monitor.scale_factor();
    let edge = preferences.edge();
    let compact = size(&area, scale, false, tool_count, edge);
    let default = PhysicalPosition::new(
        area.position.x + (area.size.width as i32 - compact.width as i32) / 2,
        area.position.y + (18.0 * scale) as i32,
    );
    Ok(Placement {
        anchor: anchor_position(saved.unwrap_or(default), compact, &area, scale, edge, false),
        edge,
        area,
        scale,
    })
}

/// 把一次定位结果落到窗口与配置上。锚点先落盘再摆窗口，
/// 这样紧随其后的移动事件读到的锚点已经是新的，不会来回改。
fn apply_placement(
    app: &AppHandle,
    window: &WebviewWindow,
    preferences: &Preferences,
    placement: Placement,
    expanded: bool,
    tool_count: u8,
) -> Result<(), String> {
    // 位置只有一份：窗口在哪，锚点就在哪，展开/收起不会把拖动结果覆盖回旧坐标。
    if preferences.position != Some(placement.anchor) || preferences.dock_edge != placement.edge {
        update_preferences(app, |saved| {
            saved.position = Some(placement.anchor);
            saved.dock_edge = placement.edge;
        })?;
    }
    let (target_position, target, strip_offset) = geometry(
        placement.anchor,
        &placement.area,
        placement.scale,
        expanded,
        tool_count,
        placement.edge,
    );
    window.set_size(target).map_err(|e| e.to_string())?;
    window
        .set_position(target_position)
        .map_err(|e| e.to_string())?;
    {
        let state = app.state::<State>();
        let mut state = state.lock();
        state.placed = true;
        state.strip_offset = strip_offset;
        state.scale = placement.scale;
        state.expanded = expanded;
    }
    Ok(())
}

fn layout(app: &AppHandle, expanded: bool) -> Result<(), String> {
    let window = app.get_webview_window(LABEL).ok_or("悬浮条窗口尚未就绪")?;
    let (preferences, tool_count, placed, strip_offset) = {
        let state = app.state::<State>();
        let state = state.lock();
        (
            state.preferences.clone(),
            state.tool_count,
            state.placed,
            state.strip_offset,
        )
    };
    let placement = if placed {
        placement_from_window(app, &window, &preferences, tool_count, strip_offset, false)?
    } else {
        initial_placement(app, &preferences, tool_count)?
    };
    apply_placement(app, &window, &preferences, placement, expanded, tool_count)
}

pub fn start(app: &AppHandle) -> Result<(), String> {
    let path = crate::data_dir(app)
        .map_err(|e| e.to_string())?
        .join("agent-hud.json");
    if path.exists() {
        // 设置文件被外部编辑器保存成带 BOM 或坏 JSON 时，退回默认值继续启动，
        // 不能因为一份设置读不出来就让悬浮条整窗消失。
        let parsed = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                let body = bytes
                    .strip_prefix(&[0xEF, 0xBB, 0xBF][..])
                    .unwrap_or(&bytes);
                serde_json::from_slice::<Preferences>(body)
                    .map_err(|e| format!("悬浮条设置无法读取：{e}"))
            });
        match parsed {
            Ok(preferences) => app.state::<State>().lock().preferences = preferences,
            Err(error) => crate::platform::notifications::report(app, error),
        }
    }
    if app.state::<State>().lock().preferences.enabled {
        create(app)?;
    }
    Ok(())
}
fn create(app: &AppHandle) -> Result<(), String> {
    if app.get_webview_window(LABEL).is_some() {
        return Ok(());
    }
    {
        let state = app.state::<State>();
        let mut state = state.lock();
        state.ready = false;
        state.tool_count = 0;
        // 新窗口还没摆过，首次 layout 必须回到保存的锚点，而不是以系统给的位置为准。
        state.placed = false;
    }
    let top = app.state::<State>().lock().preferences.always_on_top;
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html?view=hud".into()))
        .title("云阔 · Agent 悬浮条")
        .inner_size(compact_width(0), COMPACT_HEIGHT)
        .visible(false)
        .focused(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(top)
        .skip_taskbar(true)
        .additional_browser_args(crate::app::WEBVIEW_BROWSER_ARGS)
        .build()
        .map_err(|e| e.to_string())?;
    layout(app, false)
}

#[tauri::command]
pub fn agent_hud_status(app: AppHandle, window: WebviewWindow) -> Result<Status, String> {
    allowed(&window)?;
    Ok(status(&app))
}
#[tauri::command]
pub fn agent_hud_ready(app: AppHandle, window: WebviewWindow) -> Result<Status, String> {
    hud_only(&window)?;
    let enabled = {
        let state = app.state::<State>();
        let mut state = state.lock();
        state.ready = true;
        state.preferences.enabled
    };
    if enabled {
        window.show().map_err(|e| e.to_string())?;
    }
    let _ = window.emit("agent-hud-visibility", enabled);
    Ok(status(&app))
}
pub fn set_enabled(app: &AppHandle, enabled: bool) -> Result<Status, String> {
    update_preferences(app, |preferences| preferences.enabled = enabled)?;
    if enabled && app.state::<State>().lock().destroying {
        changed(app);
        return Ok(status(app));
    }
    if enabled {
        if let Err(error) = create(app) {
            update_preferences(app, |preferences| preferences.enabled = false)?;
            changed(app);
            return Err(error);
        }
    }
    if !enabled {
        let state = app.state::<State>();
        let mut state = state.lock();
        state.expanded = false;
        state.ready = false;
    }
    if let Some(window) = app.get_webview_window(LABEL) {
        if enabled {
            layout(app, false)?;
            if app.state::<State>().lock().ready {
                window.show().map_err(|e| e.to_string())?;
            }
        } else {
            let _ = window.emit("agent-hud-visibility", false);
            app.state::<State>().lock().destroying = true;
            if let Err(error) = window.destroy() {
                app.state::<State>().lock().destroying = false;
                return Err(error.to_string());
            }
        }
        let _ = window.emit("agent-hud-visibility", enabled);
    }
    changed(app);
    Ok(status(app))
}

// destroy 是异步事件；快速重新开启时，等旧窗口移出 Tauri 管理器再创建。
pub fn destroyed(app: &AppHandle) {
    let recreate = {
        let state = app.state::<State>();
        let mut state = state.lock();
        let recreate = state.destroying && state.preferences.enabled;
        if !recreate {
            state.destroying = false;
        }
        state.ready = false;
        recreate
    };
    if recreate {
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let enabled = handle.state::<State>().lock().preferences.enabled;
            if enabled {
                if let Err(error) = create(&handle) {
                    crate::platform::notifications::report(
                        &handle,
                        format!("悬浮条未能重新打开：{error}"),
                    );
                }
            }
            handle.state::<State>().lock().destroying = false;
            // 创建过程中再次关闭时，也释放刚创建完成的窗口。
            let enabled = handle.state::<State>().lock().preferences.enabled;
            if !enabled {
                if let Some(window) = handle.get_webview_window(LABEL) {
                    handle.state::<State>().lock().destroying = true;
                    let _ = window.destroy();
                }
            }
        });
    }
}
#[tauri::command]
pub async fn agent_hud_set_enabled(
    app: AppHandle,
    window: WebviewWindow,
    enabled: bool,
) -> Result<Status, String> {
    allowed(&window)?;
    set_enabled(&app, enabled)
}
#[tauri::command]
pub fn agent_hud_set_top(
    app: AppHandle,
    window: WebviewWindow,
    always_on_top: bool,
) -> Result<Status, String> {
    allowed(&window)?;
    update_preferences(&app, |preferences| {
        preferences.always_on_top = always_on_top
    })?;
    if let Some(window) = app.get_webview_window(LABEL) {
        window
            .set_always_on_top(always_on_top)
            .map_err(|e| e.to_string())?;
    }
    changed(&app);
    Ok(status(&app))
}
#[tauri::command]
pub fn agent_hud_set_options(
    app: AppHandle,
    window: WebviewWindow,
    auto_dock_top: Option<bool>,
    tool_scope: Option<ToolScope>,
) -> Result<Status, String> {
    allowed(&window)?;
    update_preferences(&app, |preferences| {
        if let Some(value) = auto_dock_top {
            preferences.auto_dock_top = value;
        }
        if let Some(value) = tool_scope {
            preferences.tool_scope = value;
        }
    })?;
    if auto_dock_top.is_some() && app.get_webview_window(LABEL).is_some() {
        let expanded = app.state::<State>().lock().expanded;
        layout(&app, expanded)?;
    }
    changed(&app);
    Ok(status(&app))
}
#[tauri::command]
pub async fn agent_hud_tools(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Vec<crate::ai::agent_tools::ToolStatus>, String> {
    allowed(&window)?;
    let config = app.state::<crate::config::ConfigMutex>().lock().clone();
    tokio::task::spawn_blocking(move || {
        crate::ai::agent_tools::read(&config)
            .map(|tools| crate::ai::agent_tools::for_cards(tools, &config))
    })
    .await
    .map_err(|_| "本机工具识别未完成".to_string())?
}
#[tauri::command]
pub fn agent_hud_expand(
    app: AppHandle,
    window: WebviewWindow,
    expanded: bool,
    tool_count: Option<u8>,
) -> Result<Status, String> {
    hud_only(&window)?;
    if let Some(count) = tool_count {
        app.state::<State>().lock().tool_count = count.min(MAX_TOOLS);
    }
    layout(&app, expanded)?;
    Ok(status(&app))
}
#[tauri::command]
pub fn agent_hud_snap(app: AppHandle, window: WebviewWindow) -> Result<bool, String> {
    hud_only(&window)?;
    if !app.state::<State>().lock().preferences.enabled {
        return Ok(true);
    }
    // 原生拖动还按着鼠标：这时排版会和系统抢窗口，等松手后的那次调用。
    if unsafe {
        windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(
            windows::Win32::UI::Input::KeyboardAndMouse::VK_LBUTTON.0 as i32,
        )
    } < 0
    {
        return Ok(false);
    }
    let (preferences, expanded, tool_count, strip_offset) = {
        let state = app.state::<State>();
        let state = state.lock();
        (
            state.preferences.clone(),
            state.expanded,
            state.tool_count,
            state.strip_offset,
        )
    };
    let placement =
        placement_from_window(&app, &window, &preferences, tool_count, strip_offset, true)?;
    // 锚点和停靠边都没变：窗口已经在该在的位置，别再 SetWindowPos——
    // 那会再发一次移动事件，400ms 后的 snap 等于被自己叫醒。
    if preferences.position == Some(placement.anchor) && preferences.dock_edge == placement.edge {
        return Ok(true);
    }
    apply_placement(&app, &window, &preferences, placement, expanded, tool_count)?;
    changed(&app);
    Ok(true)
}
#[tauri::command]
pub fn agent_hud_reset(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    allowed(&window)?;
    update_preferences(&app, |preferences| {
        preferences.position = None;
        preferences.dock_edge = DockEdge::Top;
    })?;
    // 恢复默认位置：清掉"已摆放"，下一次 layout 才会回到主屏顶部居中。
    app.state::<State>().lock().placed = false;
    layout(&app, false)?;
    changed(&app);
    Ok(())
}
#[tauri::command]
pub fn agent_hud_open_main(
    app: AppHandle,
    window: WebviewWindow,
    page: String,
) -> Result<(), String> {
    hud_only(&window)?;
    let script = match page.as_str() {
        "stats" => "location.hash = '/ai-stats'",
        "settings" => "location.hash = '/ai-stats?tab=hud'",
        "environment" => "location.hash = '/environment?tab=browser'",
        "tools" => "location.hash = '/ai'",
        "detect" => "location.hash = '/detect'",
        _ => return Err("不支持的页面".into()),
    };
    if let Some(window) = app.get_webview("main") {
        window.eval(script).map_err(|e| e.to_string())?;
    }
    crate::show_main(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changing_tool_scope_keeps_free_strip_orientation() {
        let area = PhysicalRect {
            position: PhysicalPosition::new(0, 0),
            size: PhysicalSize::new(1920, 1080),
        };
        let preferences = Preferences {
            dock_edge: DockEdge::Free,
            ..Default::default()
        };
        let strip = PhysicalPosition::new(1550, 220);
        let before = placement_for_strip(strip, &preferences, 1, area, 1.0, false);
        assert_eq!(before.edge, DockEdge::Free);
        let after = placement_for_strip(strip, &preferences, 10, area, 1.0, false);
        assert_eq!(
            after.edge,
            DockEdge::Free,
            "工具变多只能约束窗口，不能当成拖动改为侧边停靠"
        );
    }
    #[test]
    fn layout_generated_move_event_does_not_redock_free_strip() {
        let area = PhysicalRect {
            position: PhysicalPosition::new(0, 0),
            size: PhysicalSize::new(1920, 1080),
        };
        let mut preferences = Preferences {
            dock_edge: DockEdge::Free,
            ..Default::default()
        };
        let layout = placement_for_strip(
            PhysicalPosition::new(1550, 220),
            &preferences,
            10,
            area,
            1.0,
            false,
        );
        // apply_placement 落盘后 SetWindowPos 也会发送 onMoved，随后触发 snap。
        preferences.position = Some(layout.anchor);
        let snap = placement_for_strip(layout.anchor, &preferences, 10, area, 1.0, true);
        assert_eq!(
            snap.edge,
            DockEdge::Free,
            "排版自己的移动事件不能再次触发吸边"
        );
        let dragged = placement_for_strip(
            PhysicalPosition::new(6, 220),
            &preferences,
            10,
            area,
            1.0,
            true,
        );
        assert_eq!(dragged.edge, DockEdge::Left, "实际拖动仍应吸边");
    }
    #[test]
    fn snap_respects_taskbar_negative_coordinates_and_dpi() {
        let area = PhysicalRect {
            position: PhysicalPosition::new(-1920, 48),
            size: PhysicalSize::new(1920, 1032),
        };
        let window = PhysicalSize::new(594, 120);
        assert_eq!(
            constrain(PhysicalPosition::new(-1900, 65), window, &area, 1.5, true),
            PhysicalPosition::new(-1911, 57)
        );
        let position = constrain(PhysicalPosition::new(2000, 2000), window, &area, 1.5, true);
        assert_eq!(position, PhysicalPosition::new(-603, 951));
    }
    #[test]
    fn expanded_window_and_disconnected_screen_remain_accessible() {
        let area = PhysicalRect {
            position: PhysicalPosition::new(0, 0),
            size: PhysicalSize::new(960, 600),
        };
        let expanded = size(&area, 1.25, true, 3, DockEdge::Top);
        assert_eq!(expanded, PhysicalSize::new(500, 585));
        let position = constrain(
            PhysicalPosition::new(-5000, 9000),
            expanded,
            &area,
            1.25,
            false,
        );
        assert!(
            position.x >= 0
                && position.y >= 0
                && position.x + expanded.width as i32 <= 960
                && position.y + expanded.height as i32 <= 600
        );
    }
    #[test]
    fn compact_layout_tracks_tools_and_keeps_room_for_status_dots_at_each_dpi() {
        let area = PhysicalRect {
            position: PhysicalPosition::new(0, 0),
            size: PhysicalSize::new(1920, 1080),
        };
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for (count, width) in [
                (0, 312.0),
                (1, 224.0),
                (3, 320.0),
                (7, 512.0),
                (8, 560.0),
                (9, 608.0),
                (255, 608.0),
            ] {
                assert_eq!(
                    size(&area, scale, false, count, DockEdge::Top),
                    PhysicalSize::new(
                        (width * scale).round() as u32,
                        (72.0 * scale).round() as u32
                    )
                );
                assert_eq!(
                    size(&area, scale, true, count, DockEdge::Top).width,
                    (400.0_f64.max(width) * scale).round() as u32
                );
            }
        }
    }
    #[test]
    fn preferences_preserve_disabled_and_negative_position() {
        let prefs = Preferences {
            enabled: false,
            always_on_top: false,
            auto_dock_top: false,
            dock_edge: DockEdge::Right,
            tool_scope: ToolScope::Online,
            position: Some(PhysicalPosition::new(-1600, 20)),
        };
        let restored: Preferences =
            serde_json::from_slice(&serde_json::to_vec(&prefs).unwrap()).unwrap();
        assert!(!restored.enabled && !restored.always_on_top);
        assert_eq!(restored.position, prefs.position);
        assert!(!restored.auto_dock_top);
        assert_eq!(restored.tool_scope, ToolScope::Online);
        assert_eq!(restored.dock_edge, DockEdge::Right);
        assert_eq!(restored.edge(), DockEdge::Free);
        let legacy: Preferences = serde_json::from_str(
            r#"{"enabled":true,"alwaysOnTop":false,"position":{"x":160,"y":300}}"#,
        )
        .unwrap();
        assert!(legacy.enabled && legacy.auto_dock_top);
        assert_eq!(legacy.tool_scope, ToolScope::Installed);
        assert_eq!(legacy.edge(), DockEdge::Top);
    }
    #[test]
    fn drops_choose_a_near_edge_and_leave_far_drops_in_place() {
        let area = PhysicalRect {
            position: PhysicalPosition::new(-1920, 48),
            size: PhysicalSize::new(1920, 1032),
        };
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let gap = (6.0_f64 * scale).round() as i32;
            let compact = size(&area, scale, false, 2, DockEdge::Top);
            let left = -1920 + gap;
            let right = -gap - compact.width as i32;
            assert_eq!(
                drop_edge(
                    PhysicalPosition::new(left + 10, 400),
                    compact,
                    &area,
                    scale,
                    DockEdge::Top
                ),
                DockEdge::Left
            );
            assert_eq!(
                drop_edge(
                    PhysicalPosition::new(right - 10, 400),
                    compact,
                    &area,
                    scale,
                    DockEdge::Top
                ),
                DockEdge::Right
            );
            // 屏幕中间既不属于上/左/右：保持落点，不能再被改写成回到顶部。
            assert_eq!(
                drop_edge(
                    PhysicalPosition::new(-1000, 400),
                    compact,
                    &area,
                    scale,
                    DockEdge::Left
                ),
                DockEdge::Free
            );
            assert_eq!(
                drop_edge(
                    PhysicalPosition::new(left, 48 + gap),
                    compact,
                    &area,
                    scale,
                    DockEdge::Left
                ),
                DockEdge::Left
            );
            assert_eq!(
                drop_edge(
                    PhysicalPosition::new(left, 48 + gap),
                    compact,
                    &area,
                    scale,
                    DockEdge::Top
                ),
                DockEdge::Top
            );
            assert_eq!(
                drop_edge(
                    PhysicalPosition::new(100, 400),
                    compact,
                    &area,
                    scale,
                    DockEdge::Top
                ),
                DockEdge::Right
            );
        }
    }
    #[test]
    fn side_expansion_stays_inside_work_area_and_keeps_strip_in_place() {
        let area = PhysicalRect {
            position: PhysicalPosition::new(-1920, 48),
            size: PhysicalSize::new(1920, 1032),
        };
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for count in [0, 1, 3, 8, 9, 10] {
                for edge in [DockEdge::Left, DockEdge::Right] {
                    let compact = size(&area, scale, false, count, edge);
                    assert_eq!(compact.width, (72.0_f64 * scale).round() as u32);
                    for y in [48, 400, 1000] {
                        let anchor = PhysicalPosition::new(-1000, y);
                        let (collapsed, _, _) = geometry(anchor, &area, scale, false, count, edge);
                        let (expanded, window, offset) =
                            geometry(anchor, &area, scale, true, count, edge);
                        assert_eq!(
                            PhysicalPosition::new(expanded.x + offset.x, expanded.y + offset.y),
                            collapsed
                        );
                        assert!(expanded.x >= -1920 && expanded.y >= 48);
                        assert!(
                            expanded.x + window.width as i32 <= 0
                                && expanded.y + window.height as i32 <= 1080
                        );
                        assert_eq!(
                            offset.x,
                            if edge == DockEdge::Right {
                                (window.width - compact.width) as i32
                            } else {
                                0
                            }
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn horizontal_expansion_near_right_edge_preserves_strip_anchor() {
        let area = PhysicalRect {
            position: PhysicalPosition::new(0, 0),
            size: PhysicalSize::new(1920, 1080),
        };
        let anchor = PhysicalPosition::new(1690, 6);
        let (collapsed, _, _) = geometry(anchor, &area, 1.0, false, 2, DockEdge::Top);
        let (expanded, _, offset) = geometry(anchor, &area, 1.0, true, 2, DockEdge::Top);
        assert_eq!(expanded.x + offset.x, collapsed.x);
        assert!(offset.x > 0);
    }
    #[test]
    fn auto_dock_uses_current_monitor_top_and_keeps_horizontal_position() {
        let area = PhysicalRect {
            position: PhysicalPosition::new(-1920, 48),
            size: PhysicalSize::new(1920, 1032),
        };
        let window = PhysicalSize::new(594, 120);
        assert_eq!(
            anchor_position(
                PhysicalPosition::new(-1300, 800),
                window,
                &area,
                1.5,
                DockEdge::Top,
                true
            ),
            PhysicalPosition::new(-1300, 57)
        );
        assert_eq!(
            anchor_position(
                PhysicalPosition::new(-1300, 800),
                window,
                &area,
                1.5,
                DockEdge::Free,
                true
            ),
            PhysicalPosition::new(-1300, 800)
        );
        assert_eq!(
            anchor_position(
                PhysicalPosition::new(-9000, 800),
                window,
                &area,
                1.5,
                DockEdge::Top,
                true
            ),
            PhysicalPosition::new(-1911, 57)
        );
    }
}
