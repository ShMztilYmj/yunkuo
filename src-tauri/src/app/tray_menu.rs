//! 托盘快捷面板：贴着托盘图标弹出的独立小窗口。
//!
//! 收起面板不能只靠 `WindowEvent::Focused(false)`：右击托盘时前台通常是资源管理器，
//! 后台进程的 `SetForegroundWindow` 会被系统拒绝，面板于是「显示着但没激活」。
//! 而 tao 的失焦事件只在激活状态真的变化时才发（`set_active` 返回「有没有变」），
//! 从头到尾没激活过的窗口以后永远收不到 `Focused(false)`——面板就挂在那里不消失。
//! 所以这里另起一条轻量监听：面板显示期间轮询鼠标按键，按下点落在面板矩形之外就收起，
//! 与窗口有没有焦点无关。焦点只决定键盘能不能用，拿不到不算失败。

use crate::config::ConfigMutex;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalRect, PhysicalSize, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder,
};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, GetCursorPos, GetWindowRect, IsWindowVisible, SetForegroundWindow,
};

pub const LABEL: &str = "tray-menu";
// 牌面比例 2.5 : 3.5（272 : 380.8）：和扑克牌一样，竖着立在托盘旁边。
// 面板只有「打开」「退出」两个按钮，中间靠一枚淡标志撑住牌心。
const WIDTH: f64 = 272.0;
const HEIGHT: f64 = 381.0;

/// 瞬时故障的重试节奏。宿主已经失联（见 [`host_gone`]）的错误不走这套：
/// 那种情况重试只会再多等一轮，用户还是看不到面板。
const RETRY_DELAYS: [Duration; 2] = [Duration::from_millis(250), Duration::from_millis(1_200)];
/// 窗口建出来了、前端却这么久还没回报 ready，就按「空壳」处理：下次点击拆掉重建。
const LOADING_GRACE: Duration = Duration::from_secs(8);
/// 启动后隔多久预建面板，让主窗先画完首帧，别跟启动抢 WebView2。
const PRELOAD_DELAY: Duration = Duration::from_secs(6);
/// 回退到原生菜单后，隔这么久再悄悄重建一次（瞬时故障可以自愈，不必等用户再点）。
const FALLBACK_RETRY_DELAY: Duration = Duration::from_secs(10);
/// destroy 是投递到事件循环的异步操作，重建前最多等这么久让它真的消失。
const DESTROY_WAIT: Duration = Duration::from_secs(1);

/// 收起后这段时间内的托盘右击不重开：右击托盘的那次按下往往先被「点到外面」捕获，
/// 松开时托盘点击才到，不挡一下面板会立刻弹回来。
const REOPEN_GUARD: Duration = Duration::from_millis(320);
/// 面板刚出现时鼠标键可能还按着（托盘右击就发生在这一刻），这段时间内不算「点到外面」。
const PRESS_GRACE: Duration = Duration::from_millis(140);
/// 轮询鼠标按键的间隔：正常点击的按下时长在 60ms 以上，30ms 足够抓住又几乎不吃 CPU。
const PRESS_POLL: Duration = Duration::from_millis(30);
/// 显示后没拿到焦点（后台进程抢前台常被拒绝）时，在这个时间点再补一次激活。
const FOCUS_RETRY: Duration = Duration::from_millis(90);

#[derive(Default)]
pub struct State {
    ready: AtomicBool,
    initializing: AtomicBool,
    native_fallback: AtomicBool,
    /// webview 已建成、等前端 tray_ready 的时刻；只用来识别空壳窗口。
    loading_since: Mutex<Option<Instant>>,
    pending: Mutex<Option<PhysicalPosition<f64>>>,
    reopen: Mutex<ReopenGuard>,
    /// 每次显示换一代；收起时换代，上一代的「点到外面」监听据此退出。
    generation: AtomicU64,
}

/// 收起后的短暂冷却，避免「收起面板的那一次点击」顺手又把面板叫出来。
#[derive(Default)]
struct ReopenGuard {
    dismissed_at: Option<Instant>,
}

impl ReopenGuard {
    fn mark(&mut self, now: Instant) {
        self.dismissed_at = Some(now);
    }

    fn blocks(&self, now: Instant) -> bool {
        self.dismissed_at
            .is_some_and(|at| now.saturating_duration_since(at) < REOPEN_GUARD)
    }
}

pub fn is_ready(app: &AppHandle) -> bool {
    app.state::<State>().ready.load(Ordering::Acquire)
}

pub fn uses_native_menu(app: &AppHandle) -> bool {
    app.state::<State>().native_fallback.load(Ordering::Acquire)
}

/// WebView2 宿主已经不可用，重试没有意义，只能重启应用。
///
/// - `0x80010108` RPC_E_DISCONNECTED：宿主进程没了，或运行时在会话中被静默升级
///   （旧宿主还在跑旧版本，新建 environment 时 COM 代理当场失联）。
/// - `0x8007139F` ERROR_INVALID_STATE：同一数据目录被不同 EnvironmentOptions 占着。
/// - `0x80070490` ELEMENT_NOT_FOUND / `0x800706BE` RPC_S_CALL_FAILED：运行时注册项被改写或调用栈崩了。
fn host_gone(error: &str) -> bool {
    [
        "0x80010108",
        "0x8007139F",
        "0x80070490",
        "0x800706BE",
        "0x800706BA",
    ]
    .iter()
    .any(|code| error.contains(code))
}

fn fallback(app: &AppHandle) {
    app.state::<State>()
        .native_fallback
        .store(true, Ordering::Release);
    app.state::<State>().pending.lock().take();
    crate::update_tray(app);
}

#[tauri::command]
pub fn tray_ready(app: AppHandle, window: tauri::WebviewWindow) -> Result<(), String> {
    if window.label() != LABEL {
        return Err("此命令仅用于托盘面板".into());
    }
    if let Some(tray) = app.tray_by_id("main-tray") {
        tray.set_menu(None::<tauri::menu::Menu<tauri::Wry>>)
            .map_err(|e| e.to_string())?;
    }
    let state = app.state::<State>();
    state.ready.store(true, Ordering::Release);
    state.initializing.store(false, Ordering::Release);
    // 面板这次真的起来了：撤掉回退标记，托盘回到「右键出面板」。
    state.native_fallback.store(false, Ordering::Release);
    state.loading_since.lock().take();
    let anchor = state.pending.lock().take();
    if let Some(anchor) = anchor {
        show_ready(&app, anchor).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn create(app: &AppHandle) -> tauri::Result<()> {
    // 上一次创建半途失败会留下一个没有 webview 的空壳窗口，占着标签会让重建
    // 直接报「窗口已存在」。destroy 要走事件循环，所以等它真消失再建。
    if let Some(shell) = app.get_webview_window(LABEL) {
        let _ = shell.destroy();
        let deadline = Instant::now() + DESTROY_WAIT;
        while app.get_webview_window(LABEL).is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html?view=tray".into()))
        .title("云阔 · 快捷面板")
        .inner_size(WIDTH, HEIGHT)
        .visible(false)
        .focused(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .additional_browser_args(super::WEBVIEW_BROWSER_ARGS)
        .build()?;
    Ok(())
}

/// 瞬时错误退避重试；宿主失联就立刻放弃，别让用户等一轮无用的 sleep。
fn create_with_retry(app: &AppHandle) -> tauri::Result<()> {
    let mut attempt = 0;
    loop {
        match create(app) {
            Ok(()) => return Ok(()),
            Err(error) => {
                let text = error.to_string();
                if host_gone(&text) || attempt >= RETRY_DELAYS.len() {
                    return Err(error);
                }
                std::thread::sleep(RETRY_DELAYS[attempt]);
                attempt += 1;
            }
        }
    }
}

/// 启动后不久就把面板建好（隐藏）。
///
/// 面板原来是「第一次右键才创建」，而 WebView2 运行时会被后台静默升级：升级后
/// 进程里已有的 webview 还能用，但再新建 environment 就会 0x80010108 失败
/// （实测过一次：运行时在会话里从 156.0.4314.50 被升到 .58，之后第一次点开面板就报错）。
/// 跟着启动一起建，面板和主窗就永远属于同一代运行时。
///
/// 预建失败不打扰启动流程：用户真的点开时再走一次带提示的创建。
pub fn preload(app: &AppHandle) {
    // 托盘都没建起来（托盘不可用）就没人能点开面板，别白占一个 webview。
    if app.tray_by_id("main-tray").is_none() || app.get_webview_window(LABEL).is_some() {
        return;
    }
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        std::thread::sleep(PRELOAD_DELAY);
        let state = handle.state::<State>();
        if handle.get_webview_window(LABEL).is_some() || is_ready(&handle) {
            return;
        }
        // 用户抢先点了面板就交给点击路径，不要两路同时建。
        if state.initializing.swap(true, Ordering::AcqRel) {
            return;
        }
        let result = create_with_retry(&handle);
        state.initializing.store(false, Ordering::Release);
        match result {
            Ok(()) => *state.loading_since.lock() = Some(Instant::now()),
            Err(error) => log::warn!("[tray-menu] 预建快捷面板失败：{error}"),
        }
    });
}

/// 面板不可用时再给一次机会：只用于非宿主失联的错误。
fn retry_after_fallback(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        std::thread::sleep(FALLBACK_RETRY_DELAY);
        let state = handle.state::<State>();
        if handle.get_webview_window(LABEL).is_some() || is_ready(&handle) {
            return;
        }
        if state.initializing.swap(true, Ordering::AcqRel) {
            return;
        }
        let result = create_with_retry(&handle);
        state.initializing.store(false, Ordering::Release);
        // 前端 onMount 会调 tray_ready：那时自动撤掉原生菜单、恢复面板。
        if result.is_ok() {
            *state.loading_since.lock() = Some(Instant::now());
        }
    });
}

fn watch_timeout(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(5)).await;
        if is_ready(&handle) {
            return;
        }
        fallback(&handle);
        crate::platform::notifications::report(
            &handle,
            "快捷面板加载超时，已切换到系统托盘菜单。".into(),
        );
    });
}

fn position(
    anchor: PhysicalPosition<f64>,
    area: &PhysicalRect<i32, u32>,
    size: PhysicalSize<u32>,
    gap: i32,
) -> PhysicalPosition<i32> {
    let left = area.position.x + gap;
    let top = area.position.y + gap;
    let right = (area.position.x + area.size.width as i32 - size.width as i32 - gap).max(left);
    let bottom = (area.position.y + area.size.height as i32 - size.height as i32 - gap).max(top);
    let above = anchor.y as i32 - size.height as i32 - gap;
    let y = if above >= top {
        above
    } else {
        anchor.y as i32 + gap
    };
    PhysicalPosition::new(
        (anchor.x as i32 - size.width as i32 + gap).clamp(left, right),
        y.clamp(top, bottom),
    )
}

/// 右键托盘：面板已就绪就直接弹，否则现建。
///
/// 返回 `String` 而不是 `tauri::Error`：宿主失联时要给用户一句能照做的事
/// （重启应用），而这不是构造得出来的 `tauri::Error`。
pub fn show(app: &AppHandle, anchor: PhysicalPosition<f64>) -> Result<(), String> {
    // 面板开着时再右击托盘就是「收起」；刚被这次点击收起过也别立刻叫回来。
    if live_visible(app) {
        dismiss(app);
        return Ok(());
    }
    if app.state::<State>().reopen.lock().blocks(Instant::now()) {
        return Ok(());
    }
    let state = app.state::<State>();
    *state.pending.lock() = Some(anchor);

    if let Some(window) = app.get_webview_window(LABEL) {
        if is_ready(app) {
            return show_ready(app, anchor).map_err(|error| error.to_string());
        }
        if state.initializing.load(Ordering::Acquire) {
            return Ok(());
        }
        let loading = state.loading_since.lock().map(|since| since.elapsed());
        if matches!(loading, Some(elapsed) if elapsed < LOADING_GRACE) {
            return Ok(());
        }
        // 只剩空壳（上一次创建半途失败，或前端从没回报 ready）：拆掉重建，
        // 否则这次会话里就一直「点了没反应」。
        let _ = window.destroy();
        state.loading_since.lock().take();
    }

    if state.initializing.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    let result = create_with_retry(app);
    state.initializing.store(false, Ordering::Release);
    match result {
        Ok(()) => {
            *state.loading_since.lock() = Some(Instant::now());
            watch_timeout(app);
            Ok(())
        }
        Err(error) => {
            let text = error.to_string();
            fallback(app);
            if host_gone(&text) {
                Err(format!(
                    "{text}（WebView2 运行环境已在本会话中被更新或损坏，重启云阔即可恢复）"
                ))
            } else {
                retry_after_fallback(app);
                Err(text)
            }
        }
    }
}

fn show_ready(app: &AppHandle, anchor: PhysicalPosition<f64>) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window(LABEL) else {
        return Ok(());
    };
    if let Some(monitor) = app
        .monitor_from_point(anchor.x, anchor.y)?
        .or(app.primary_monitor()?)
    {
        let scale = monitor.scale_factor();
        let gap = (6.0 * scale).round() as i32;
        let area = monitor.work_area();
        let size = PhysicalSize::new(
            ((WIDTH * scale).round() as u32)
                .min(area.size.width.saturating_sub((gap * 2) as u32))
                .max(1),
            ((HEIGHT * scale).round() as u32)
                .min(area.size.height.saturating_sub((gap * 2) as u32))
                .max(1),
        );
        window.set_position(position(anchor, area, size, gap))?;
        window.set_size(size)?;
    }
    window.show()?;
    // 拿焦点只为了键盘（Esc / 方向键）能用。右击托盘多半发生在别的程序前台，
    // 后台进程抢前台会被系统拒绝，那属于正常情况：收起交给下面的「点到外面」监听。
    let _ = window.set_focus();
    activate(&window);
    window.emit("tray-visibility", true)?;
    window.emit("tray-opened", ())?;
    let generation = {
        let state = app.state::<State>();
        state.generation.fetch_add(1, Ordering::AcqRel) + 1
    };
    watch_outside(app.clone(), generation);
    Ok(())
}

/// 面板此刻是否真的显示着。以窗口状态为准：前端自己收起时原生侧不必被通知。
fn live_visible(app: &AppHandle) -> bool {
    app.get_webview_window(LABEL)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
}

/// 把面板拉到前台。`set_focus` 走的是同一套系统调用，这里补一次显式调用；
/// 失败也不影响显示与收起，只是键盘要到用户点过面板之后才好使。
fn activate(window: &WebviewWindow) {
    if window.is_focused().unwrap_or(false) {
        return;
    }
    let Ok(hwnd) = window.hwnd() else { return };
    // tauri 与主 crate 用的 `windows` 版本不同，按裸指针换成自己的 HWND。
    let hwnd = HWND(hwnd.0);
    unsafe {
        let _ = SetForegroundWindow(hwnd);
        let _ = BringWindowToTop(hwnd);
    }
}

/// 收起面板：等同于「在面板外面点了一下」。
pub fn dismiss(app: &AppHandle) {
    {
        let state = app.state::<State>();
        state.reopen.lock().mark(Instant::now());
        state.generation.fetch_add(1, Ordering::AcqRel);
    }
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    let _ = window.hide();
    let _ = window.emit("tray-visibility", false);
}

/// 面板显示期间盯住鼠标：按下点落在面板外面就收起。
///
/// 只认「刚按下」的边沿：按住不放、在面板里点都不算；面板隐藏或换了新的一代就退出。
fn watch_outside(app: AppHandle, generation: u64) {
    std::thread::spawn(move || {
        let Some(window) = app.get_webview_window(LABEL) else {
            return;
        };
        let Ok(hwnd) = window.hwnd() else { return };
        let hwnd = HWND(hwnd.0);
        let started = Instant::now();
        let mut held = pointer_down();
        let mut refocused = false;
        while app.state::<State>().generation.load(Ordering::Acquire) == generation {
            if !window_visible(hwnd) {
                break;
            }
            if !refocused && started.elapsed() >= FOCUS_RETRY {
                refocused = true;
                if !window.is_focused().unwrap_or(true) {
                    activate(&window);
                }
            }
            let down = pointer_down();
            if down && !held && dismissable(started, Instant::now(), cursor_inside(hwnd)) {
                dismiss(&app);
                break;
            }
            held = down;
            std::thread::sleep(PRESS_POLL);
        }
    });
}

/// 这次刚按下的鼠标该不该收起面板：太靠近显示时刻的那一下（按着不放）不算。
fn dismissable(started: Instant, now: Instant, inside: bool) -> bool {
    now.saturating_duration_since(started) >= PRESS_GRACE && !inside
}

fn pointer_down() -> bool {
    [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON]
        .iter()
        .any(|key| unsafe { GetAsyncKeyState(key.0 as i32) } < 0)
}

fn window_visible(hwnd: HWND) -> bool {
    unsafe { IsWindowVisible(hwnd) }.as_bool()
}

fn cursor_inside(hwnd: HWND) -> bool {
    unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            // 读不到窗口矩形时宁可不收，避免面板点一下就不见了。
            return true;
        }
        let mut point = POINT::default();
        if GetCursorPos(&mut point).is_err() {
            return true;
        }
        hit(&rect, point.x, point.y)
    }
}

/// 半开区间判定：右/下边界算在窗口外，和 `GetWindowRect` 的语义一致。
fn hit(rect: &RECT, x: i32, y: i32) -> bool {
    (rect.left..rect.right).contains(&x) && (rect.top..rect.bottom).contains(&y)
}

#[tauri::command]
pub async fn tray_action(app: AppHandle, action: String) -> Result<Option<String>, String> {
    match action.as_str() {
        "show" | "detect" | "settings" | "ai-stats" => {
            if let Some(window) = app.get_webview("main") {
                match action.as_str() {
                    "detect" => window
                        .eval("window.location.hash = '/detect'")
                        .map_err(|e| e.to_string())?,
                    "settings" => window
                        .eval("window.location.hash = '/settings'")
                        .map_err(|e| e.to_string())?,
                    "ai-stats" => window
                        .eval("window.location.hash = '/ai-stats'")
                        .map_err(|e| e.to_string())?,
                    _ => {}
                }
            }
            crate::show_main(&app);
        }
        "launch" => {
            let result = crate::app::ai::launch_claude(app.clone(), true).await?;
            if !result.ok {
                return Err(result.failures.join("\n"));
            }
        }
        "sysproxy" => {
            let cfg = app.state::<ConfigMutex>().lock().clone();
            let active = crate::proxy::sysproxy::is_ours(&cfg)
                && crate::proxy::core::verify_proxy_ready(&app, &cfg)
                    .await
                    .is_ok();
            let enable = !active;
            crate::app::network::set_sysproxy(app.clone(), enable).await?;
        }
        "tun" => {
            let cfg = app.state::<ConfigMutex>().lock().clone();
            let active = cfg.enable_tun
                && proxy_runtime::adapter_up(crate::privacy::TUN_NAME)
                && crate::proxy::core::verify_proxy_ready(&app, &cfg)
                    .await
                    .is_ok();
            let enable = !active;
            crate::app::network::set_tun(app.clone(), enable).await?;
        }
        "mode-rule" | "mode-global" | "mode-direct" => {
            let mode = action.strip_prefix("mode-").ok_or("运行模式无效")?;
            crate::app::commands::patch_config(app.clone(), serde_json::json!({ "mode": mode }))
                .await?;
        }
        "sync-tz" => {
            let result = crate::app::environment::sync_timezone_by_ip(app.clone()).await?;
            return Ok(Some(result.message));
        }
        "data" => {
            let directory = crate::data_dir(&app).map_err(|e| e.to_string())?;
            std::process::Command::new("explorer")
                .arg(directory)
                .spawn()
                .map_err(|e| format!("打开数据目录失败：{e}"))?;
        }
        "quit" => app.exit(0),
        _ => return Err("未知的托盘操作".into()),
    }
    crate::update_tray(&app);
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_stays_inside_bottom_taskbar_work_area() {
        let area = PhysicalRect {
            position: PhysicalPosition::new(0, 0),
            size: PhysicalSize::new(1920, 1040),
        };
        let size = PhysicalSize::new(400, 610);
        let p = position(PhysicalPosition::new(1880.0, 1060.0), &area, size, 8);
        assert!(p.x >= 8 && p.x + size.width as i32 <= 1912);
        assert!(p.y >= 8 && p.y + size.height as i32 <= 1032);
    }

    #[test]
    fn popup_supports_top_taskbar_and_negative_monitor_coordinates() {
        let area = PhysicalRect {
            position: PhysicalPosition::new(-1920, 48),
            size: PhysicalSize::new(1920, 1032),
        };
        let p = position(
            PhysicalPosition::new(-50.0, 24.0),
            &area,
            PhysicalSize::new(320, 488),
            6,
        );
        assert!(p.x >= -1914 && p.x + 320 <= -6);
        assert!(p.y >= 54 && p.y + 488 <= 1074);
    }

    #[test]
    fn host_disconnect_codes_skip_retry() {
        let disconnected = "runtime error: failed to create webview: WebView2 error: WindowsError(Error { code: HRESULT(0x80010108), message: \"The object invoked has disconnected from its clients.\" })";
        assert!(host_gone(disconnected));
        assert!(host_gone("WebView2 error: ... 0x8007139F"));
        // 普通的建窗失败仍然值得重试。
        assert!(!host_gone("WebView2 error: ... 0x80070005"));
    }

    #[test]
    fn outside_press_is_judged_against_half_open_window_rect() {
        let rect = RECT {
            left: 100,
            top: 200,
            right: 372,
            bottom: 482,
        };
        assert!(hit(&rect, 100, 200));
        assert!(hit(&rect, 371, 481));
        // 右/下边界已经在窗口外：点在边缘上也要算「点出去了」。
        assert!(!hit(&rect, 372, 300));
        assert!(!hit(&rect, 200, 482));
        assert!(!hit(&rect, 99, 300));
        assert!(!hit(&rect, 200, 199));
    }

    #[test]
    fn press_inside_the_grace_window_never_dismisses() {
        let started = Instant::now();
        // 面板刚弹出来就按住不放（就是托盘右击那一下）：不动。
        assert!(!dismissable(started, started + PRESS_GRACE / 2, false));
        assert!(dismissable(started, started + PRESS_GRACE, false));
        // 按在面板里面本来就不该收。
        assert!(!dismissable(started, started + PRESS_GRACE, true));
    }

    #[test]
    fn reopen_right_after_dismiss_is_swallowed() {
        let mut guard = ReopenGuard::default();
        let now = Instant::now();
        assert!(!guard.blocks(now));
        guard.mark(now);
        // 收起它的那一次点击（先按下、后松开）不该把面板立刻叫回来。
        assert!(guard.blocks(now));
        assert!(guard.blocks(now + REOPEN_GUARD / 2));
        // 隔开一次正常点击的间隔之后要能重新打开。
        assert!(!guard.blocks(now + REOPEN_GUARD));
    }
}
