//! 桌面运行时装配、Tauri 命令注册和应用生命周期。
use crate::{
    ai, app, config, diagnostics, faults, fingerprint_browser, platform, proxy, workspace,
};

pub fn diagnose_claude_resume(output: std::path::PathBuf) {
    workspace::resume::claude::diagnose(output);
}
pub fn diagnose_codex_resume(output: std::path::PathBuf) {
    workspace::resume::codex::diagnose(output);
}
pub fn install_agent_monitor() -> Result<(), String> {
    ai::agent_events::install_all()
}
pub fn verify_agent_usage_engine() -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|_| "无法定位主程序")?;
    let folder = executable.parent().ok_or("无法定位资源目录")?;
    ai::aimon::engine::init(folder);
    ai::aimon::engine::verify_bundle()
}
/// 安装器卸载前清理；返回进程退出码，失败原因写入 report。
pub fn uninstall_cleanup(report: Option<std::path::PathBuf>) -> i32 {
    app::uninstall::run(report)
}

use config::{AppConfig, ConfigMutex};
use proxy::core;
use proxy::core::CoreState;
use proxy::profile::ProfileManager;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

pub(crate) fn data_dir(app: &AppHandle) -> anyhow::Result<std::path::PathBuf> {
    Ok(app.path().app_data_dir()?)
}

fn load_or_init(app: &AppHandle) -> anyhow::Result<AppConfig> {
    let dir = data_dir(app)?;
    std::fs::create_dir_all(&dir)?;
    app::maintenance::recover_pending(&dir)?;
    config::load(&dir.join("config.yaml"))
}

/// 构建/刷新托盘菜单（带状态标记）
pub(crate) fn update_tray(app: &AppHandle) {
    if let Err(error) = try_update_tray(app) {
        platform::notifications::report(app, format!("托盘菜单更新失败：{error}"));
    }
}

fn try_update_tray(app: &AppHandle) -> tauri::Result<()> {
    if !app::tray_menu::uses_native_menu(app) {
        if let Some(tray) = app.tray_by_id("main-tray") {
            tray.set_tooltip(Some("云阔 · 自在连接，从容探索"))?;
        }
        return Ok(());
    }
    let cfg = app.state::<ConfigMutex>().lock().clone();
    let proxy_label = if cfg.enable_system_proxy {
        "系统代理：已开启（点击关闭）"
    } else {
        "系统代理：已关闭（点击开启）"
    };
    let tun_label = if cfg.enable_tun {
        "TUN 模式：已开启（点击关闭）"
    } else {
        "TUN 模式：已关闭（点击开启）"
    };
    let scheme = match cfg.network_mode {
        Some(crate::network_mode::NetworkMode::Daily) => "日常分流",
        Some(crate::network_mode::NetworkMode::Strict) => "AI 严格保护",
        None => "自定义",
    };
    let mode_label = format!(
        "网络方案：{} · {}路由（打开设置）",
        scheme,
        match cfg.mode.as_str() {
            "global" => "全局",
            "direct" => "直连",
            _ => "规则",
        }
    );

    let show = MenuItem::with_id(app, "show", "显示主界面", true, None::<&str>)?;
    let launch = MenuItem::with_id(app, "launch", "启动 Claude", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let custom = cfg.network_mode.is_none();
    let proxy = MenuItem::with_id(app, "sysproxy", proxy_label, custom, None::<&str>)?;
    let tun = MenuItem::with_id(app, "tun", tun_label, custom, None::<&str>)?;
    let mode = MenuItem::with_id(app, "mode", &mode_label, true, None::<&str>)?;
    let sync_tz = MenuItem::with_id(app, "sync-tz", "同步时区（按出口 IP）", true, None::<&str>)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let data = MenuItem::with_id(app, "data", "打开数据目录", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &show, &launch, &sep1, &proxy, &tun, &mode, &sync_tz, &sep2, &data, &quit,
        ],
    )?;

    if let Some(tray) = app.tray_by_id("main-tray") {
        tray.set_menu(Some(menu))?;
    }
    Ok(())
}

fn handle_tray_event(app: &AppHandle, id: &str) {
    match id {
        "show" => show_main(app),
        "quit" => {
            app.exit(0);
        }
        "launch" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let error = match app::ai::launch_claude(app.clone(), true).await {
                    Ok(result) if result.ok => None,
                    Ok(result) => Some(result.failures.join("\n")),
                    Err(error) => Some(error),
                };
                if let Some(error) = error {
                    show_main(&app);
                    app.dialog()
                        .message(error)
                        .title("无法启动 Claude")
                        .kind(MessageDialogKind::Error)
                        .show(|_| {});
                }
            });
        }
        "sysproxy" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let enable = !app.state::<ConfigMutex>().lock().enable_system_proxy;
                if let Err(error) = app::network::set_sysproxy(app.clone(), enable).await {
                    show_main(&app);
                    platform::notifications::report(&app, format!("切换系统代理失败：{error}"));
                }
                update_tray(&app);
            });
        }
        "tun" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let enable = !app.state::<ConfigMutex>().lock().enable_tun;
                if let Err(error) = app::network::set_tun(app.clone(), enable).await {
                    show_main(&app);
                    platform::notifications::report(&app, format!("切换 TUN 失败：{error}"));
                }
                update_tray(&app);
            });
        }
        "mode" => {
            // Scheme transitions and protection confirmation share one UI.
            show_main(app);
            if let Some(window) = app.get_webview("main") {
                if let Err(error) = window.eval("window.location.hash = '/settings?tab=proxy'") {
                    platform::notifications::report(app, format!("打开网络方案设置失败：{error}"));
                }
            }
        }
        "sync-tz" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = app::environment::sync_timezone_by_ip(app.clone()).await {
                    show_main(&app);
                    platform::notifications::report(&app, format!("同步时区失败：{error}"));
                }
            });
        }
        "data" => match data_dir(app) {
            Ok(dir) => {
                let _ = std::process::Command::new("explorer").arg(dir).spawn();
            }
            Err(error) => {
                show_main(app);
                platform::notifications::report(app, format!("打开数据目录失败：{error}"));
            }
        },
        _ => {}
    }
}

pub(crate) fn show_main(app: &AppHandle) {
    // 主窗就是「面板之外」：它一露出来，快捷面板必须收掉，否则那块 always-on-top
    // 的小窗会一直压在界面上。
    app::tray_menu::dismiss(app);
    if let Some(win) = app.get_window("main") {
        let _ = win.show();
        let _ = app.emit("app-visibility", true);
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

/// 主窗口在配置里以隐藏状态创建，前端画出启动画面后调用本命令显示。
/// 静默启动时保持隐藏，只留托盘。
#[tauri::command]
pub fn main_show(app: AppHandle) {
    MAIN_FRAME_SHOWN.store(true, std::sync::atomic::Ordering::Release);
    if app.state::<ConfigMutex>().lock().enable_silent_start {
        return;
    }
    show_main(&app);
}

/// 前端异常时用于兜底显示主窗口：首帧命令未到达才补显示。
static MAIN_FRAME_SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 订阅自动更新：每 30 分钟检查一次，到期的 remote 订阅重新下载并重载内核
async fn auto_update_profiles(app: &AppHandle) -> anyhow::Result<()> {
    let environment = app.state::<platform::state::EnvironmentOperation>();
    let _environment = environment.lock().await;
    let operation = app.state::<platform::state::ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    let cfg = app.state::<ConfigMutex>().lock().clone();
    // 自动重载会撤销 TUN 放行；受保护会话期间留到用户主动更新并重新启用保护。
    if cfg.network_mode == Some(crate::network_mode::NetworkMode::Strict)
        || proxy::service::registered()
    {
        match proxy::core::privacy_status().await {
            Ok(status) if status["armed"].as_bool() == Some(false) => {}
            _ => return Ok(()),
        }
    }
    let dd = data_dir(app)?;
    let pm = ProfileManager::new(&dd);
    let ps = pm.load()?;
    let now = chrono::Utc::now().timestamp();
    let mut changed = false;
    for item in &ps.items {
        if item.item_type != "remote" || item.update_interval == 0 {
            continue;
        }
        let due = proxy::profile::update_due(item, now);
        if due {
            match pm.update(&item.uid).await {
                Ok(()) => {
                    changed |= pm.load()?.current.as_deref() == Some(item.uid.as_str());
                }
                Err(error) => {
                    platform::notifications::report(&app, format!("订阅自动更新失败：{error:#}"))
                }
            }
        }
    }
    if changed && proxy::core::child_alive(app) {
        app::proxy::invalidate_network(app);
        proxy::core::start_core(app).await?;
    }
    Ok(())
}

#[derive(Default)]
struct ProxyExit {
    allowed: std::sync::atomic::AtomicBool,
    pending: std::sync::atomic::AtomicBool,
}

/// 用户在「仍有会话在运行」的确认框里点了「仍要退出」。
///
/// 关闭请求与退出请求原来各自硬拦一次，只发一条提示；现在两道守卫都看这个标记，
/// 免得点多少次都关不掉。标记只在本次进程内有效。
#[derive(Default)]
pub(crate) struct ForceExit(std::sync::atomic::AtomicBool);

impl ForceExit {
    fn allowed(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[tauri::command]
pub fn app_force_exit(app: tauri::AppHandle, force: tauri::State<'_, ForceExit>) {
    force.0.store(true, std::sync::atomic::Ordering::SeqCst);
    app.exit(0);
}

/// 退出被拦下的原因，前端据此弹确认框。
///
/// `proxy`：退出前的代理恢复失败。
#[derive(Clone, serde::Serialize)]
struct ExitBlocked {
    reason: &'static str,
    detail: Option<String>,
}

/// 关闭请求的两种去向。抽成纯函数，免得多道守卫再写出「怎么点都关不掉」。
#[derive(Debug, PartialEq, Eq)]
enum CloseAction {
    HideToTray,
    Exit,
}

fn close_action(close_to_tray: bool, has_tray: bool, forced: bool) -> CloseAction {
    if close_to_tray && has_tray && !forced {
        return CloseAction::HideToTray;
    }
    CloseAction::Exit
}

#[cfg(test)]
mod close_tests {
    use super::{CloseAction, close_action};

    #[test]
    fn closing_hides_to_tray_without_asking() {
        assert_eq!(close_action(true, true, false), CloseAction::HideToTray);
    }

    #[test]
    fn tray_icon_failure_still_exits_instead_of_doing_nothing() {
        // 托盘图标创建失败（has_tray=false）不能变成「点了没反应」。
        assert_eq!(close_action(true, false, false), CloseAction::Exit);
    }

    #[test]
    fn forced_exit_always_gets_through() {
        assert_eq!(close_action(false, false, true), CloseAction::Exit);
        assert_eq!(close_action(true, true, true), CloseAction::Exit);
    }
}

fn with_managed_states(builder: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    builder
        .manage(ProxyExit::default())
        .manage(ForceExit::default())
        .manage(CoreState::new())
        .manage(platform::notifications::ErrorState::default())
        .manage(platform::state::ConfigUpdateMutex::default())
        .manage(diagnostics::detect::WebScanMutex::default())
        .manage(platform::state::EnvironmentOperation::default())
        .manage(fingerprint_browser::browser::Runtime::default())
        .manage(fingerprint_browser::selfcheck::SelfcheckState::default())
        .manage(ai::management::State::default())
        .manage(ai::execution_policy::State::new(app::ai::check_execution))
        .manage(ai::identity::ExecutionHooks::new(
            workspace::resume::claude::pause_for_identity_change,
            workspace::resume::codex::pause_for_account_change,
            fingerprint_browser::browser::ensure_closed,
            app::environment::invalidate,
        ))
        .manage(ai::monitor::State::default())
        .manage(workspace::resume::codex::State::default())
        .manage(workspace::resume::claude::State::default())
        .manage(app::tray_menu::State::default())
        .manage(ai::agent_hud::State::default())
}

/// 专用无副作用自检：只注册与正式入口相同的内存状态，不构建窗口或执行 setup。
pub fn verify_managed_states() {
    let _builder = with_managed_states(tauri::Builder::default());
}

/// 工作区浏览器里的网页内容不允许调用宿主命令；其 webview label 带固定前缀。
const WEB_CONTENT_PREFIX: &str = "workspace-browser-";
fn trusted_invoker(label: &str) -> bool {
    !label.starts_with(WEB_CONTENT_PREFIX)
}

pub fn initialize_fault_reporting() {
    if let Some(root) = faults::app_root() {
        faults::install(root, "app");
    }
}
pub fn crash_journal_probe(root: std::path::PathBuf) -> ! {
    faults::panic_probe(root)
}

#[tauri::command]
fn record_frontend_fault(kind: faults::Kind) -> Result<(), String> {
    if !matches!(
        kind,
        faults::Kind::Render | faults::Kind::Rejection | faults::Kind::Script
    ) {
        return Err("未知的界面故障类型".into());
    }
    faults::record(kind);
    Ok(())
}

fn show_startup_error(message: &str) {
    use windows::{
        Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW},
        core::{PCWSTR, w},
    };
    let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            w!("云阔无法启动"),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 旧品牌迁移先于一切存储初始化：数据目录一旦按新标识创建，旧目录就再也接不上了。
    if let Some(note) = super::migrate::adopt_legacy_data_dir() {
        log::info!("{note}");
    }
    super::migrate::drop_legacy_autostart();
    // Legacy DNS recovery is journal-scoped inside the owned runtime.
    // Startup must not reset another proxy client's DNS adapters.
    let webview_profile = match platform::startup::prepare_webview() {
        Ok(profile) => profile,
        Err(error) => {
            faults::record(faults::Kind::StartupFailed);
            show_startup_error(&error);
            return;
        }
    };
    let application = with_managed_states(tauri::Builder::default())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        // 开机启动项名随品牌改为「云阔」；旧键名由 migrate::drop_legacy_autostart 清掉。
        .plugin(tauri_plugin_autostart::Builder::new().app_name("云阔").build())
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // 第二个实例启动时：聚焦主窗口（防止双开造成的状态分裂）
            if let Some(win) = app.get_window("main") {
                let _ = win.show();
                let _ = app.emit("app-visibility", true);
                let _ = win.unminimize();
                let _ = win.set_focus();
            }
            let handle = app.clone();
            tauri::async_runtime::spawn(fingerprint_browser::browser::handle_arguments(handle, args));
        }))
        .invoke_handler(|invoke| {
            if !trusted_invoker(invoke.message.webview_ref().label()) {
                invoke.resolver.reject("网页内容不能调用宿主命令");
                return true;
            }
            let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
            app::desktop::main_show,
            app::desktop::app_force_exit,
            app::tray_menu::tray_action,
            app::tray_menu::tray_ready,
            ai::agent_hud::agent_hud_status,
            ai::agent_hud::agent_hud_ready,
            ai::agent_hud::agent_hud_set_enabled,
            ai::agent_hud::agent_hud_set_top,
            ai::agent_hud::agent_hud_set_options,
            ai::agent_hud::agent_hud_tools,
            ai::agent_hud::agent_hud_expand,
            ai::agent_hud::agent_hud_snap,
            ai::agent_hud::agent_hud_reset,
            ai::agent_hud::agent_hud_open_main,
            app::commands::get_config,
            record_frontend_fault,
            ai::tool_setup::tool_setup_status,
            ai::tool_setup::tool_setup_inspect,
            ai::tool_setup::tool_setup_test,
            ai::tool_setup::tool_setup_autodetect,
            ai::capabilities::capability_snapshot,
            ai::capabilities::capability_action,
            ai::capabilities::capability_sources,
            ai::capabilities::capability_mcp_detail,
            ai::capabilities::capability_diff,
            ai::capabilities::capability_skill_detail,
            platform::notifications::get_core_errors,
            platform::notifications::dismiss_core_error,
            app::commands::patch_config,
            app::network::core_status,
            app::network::proxy_port_usage,
            app::network::core_start,
            app::network::core_restart,
            app::network::core_stop,
            app::commands::get_runtime_info,
            app::network::list_profiles,
            app::network::current_profile,
            app::network::create_profile,
            app::network::create_profile_node,
            app::network::update_profile,
            app::network::delete_profile,
            app::network::patch_profile,
            app::network::set_profile_quota,
            app::network::select_profile,
            app::network::mihomo_request,
            app::network::native_proxy_request,
            app::proxy::proxy_nodes,
            app::proxy::proxy_rules,
            app::proxy::proxy_connections,
            app::proxy::proxy_select_node,
            app::proxy::proxy_close_connections,
            app::proxy::proxy_delay,
            app::proxy::proxy_explain,
            app::proxy::proxy_overlay,
            app::proxy::proxy_overlay_save,
            app::proxy::proxy_overlay_reset,
            app::network::connection_totals,
            app::network::set_sysproxy,
            app::environment::sync_timezone_by_ip,
            app::environment::restore_timezone,
            app::environment::set_timezone_manual,
            diagnostics::commands::get_timezone_status,
            diagnostics::commands::detect_native,
            diagnostics::commands::record_web_scan,
            diagnostics::commands::begin_environment_scan,
            diagnostics::commands::finish_environment_scan,
            diagnostics::report::get_diagnostics_report_status,
            diagnostics::history::get_last_scan,
            diagnostics::history::save_last_scan,
            app::ai::launch_claude,
            app::ai::launch_ai,
            app::commands::open_data_dir,
            app::commands::is_elevated_cmd,
            diagnostics::commands::get_ip_info,
            app::commands::ui_log,
            app::network::set_tun,
            app::network::privacy_release,
            app::network::privacy_enable,
            app::network::privacy_status,
            app::network::set_network_mode,
            app::network::set_routing_mode,
            app::network::restore_network_mode,
            app::network::network_mode_backup_status,
            proxy::ipmap::trace_exit,
            proxy::ipmap::multi_geo,
            proxy::ipmap::ip_intel,
            proxy::ipmap::scan_ports,
            proxy::ipmap::dns_leak,
            diagnostics::dns_deep::dns_deep_start,
            diagnostics::dns_deep::dns_deep_collect,
            diagnostics::dns_deep::dns_deep_cancel,
            fingerprint_browser::defaultbrowser::default_browser_status,
            app::network::service_status,
            app::network::service_install,
            app::network::service_uninstall,
            ai::aimon::ai_usage_status,
            ai::aimon::engine::ai_usage_engine_status,
            ai::aimon::engine::ai_usage_engine_refresh,
            ai::agent_events::agent_integration_status,
            ai::agent_events::agent_integration_connect,
            ai::monitor::ai_monitor_status,
            ai::monitor::ai_monitor_set_enabled,
            ai::monitor::ai_monitor_dismiss,
            workspace::resume::codex::codex_resume_status,
            workspace::resume::codex::codex_resume_candidates,
            workspace::resume::codex::codex_resume_check,
            workspace::resume::codex::codex_resume_enable,
            workspace::resume::codex::codex_resume_watch,
            workspace::resume::codex::codex_resume_cancel,
            workspace::resume::codex::codex_resume_forget,
            workspace::resume::claude::claude_resume_status,
            workspace::resume::claude::claude_resume_candidates,
            workspace::resume::claude::claude_resume_check,
            workspace::resume::claude::claude_resume_enable,
            workspace::resume::claude::claude_resume_watch,
            workspace::resume::claude::claude_resume_cancel,
            workspace::resume::claude::claude_resume_forget,
            ai::aimon::ai_switch_account,
            ai::aimon::ai_snapshot_account,
            ai::aimon::ai_list_accounts,
            ai::aimon::ai_migrate_accounts,
            app::updates::inspect_installer,
            app::updates::launch_verified_installer,
            app::maintenance::maintenance_status,
            app::maintenance::export_diagnostics,
            app::maintenance::restore_config_backup,
            ai::identity::vault::identity_status,
            ai::identity::vault::identity_action,
            fingerprint_browser::browser::browser_environment_status,
            fingerprint_browser::browser::browser_environment_save,
            fingerprint_browser::browser::browser_environment_launch,
            fingerprint_browser::browser::browser_environment_clear,
            fingerprint_browser::browser::browser_request_status,
            fingerprint_browser::browser::browser_open_pending,
            fingerprint_browser::selfcheck::browser_selfcheck_start,
            fingerprint_browser::selfcheck::browser_selfcheck_status,
            diagnostics::computer::computer_environment_status,
            app::environment::computer_environment_apply,
            ];
            handler(invoke)
        })
        .setup(move |app| {
            let handle = app.handle().clone();
            app::environment::install_browser_gate(&handle).map_err(std::io::Error::other)?;

            if webview_profile.fallback {
                // 数据目录已回退但仍能启动，只需告知，不必占用错误区。
                platform::notifications::report_as(&handle, platform::notifications::Level::Info, "WebView2 默认数据目录无法写入，已使用程序目录下的兼容数据目录。".into());
            }

            if faults::start() { platform::notifications::report_as(&handle, platform::notifications::Level::Warn, "上次程序未正常结束，请先检查当前代理与保护状态。故障记录位于本机诊断目录。".into()); }

            // 配置初始化
            let cfg = load_or_init(&handle)?;
            handle.manage(ConfigMutex::new(cfg.clone()));
            ai::aimon::engine::init(&handle.path().resource_dir()?);
            // 额度查询走本机代理端口：启动时先写一次，避免首轮扫描早于页面读取而直连。
            ai::aimon::set_proxy_port(cfg.mixed_port);
            ai::aimon::engine::invalidate_scope(ai::agent_tools::MonitoringScope::from_cards(&cfg.ai_tool_cards));
            if let Err(error) = ai::agent_events::set_scope(&ai::agent_tools::MonitoringScope::from_cards(&cfg.ai_tool_cards)) {
                platform::notifications::report(&handle, error);
            }
            ai::monitor::start(&handle);
            workspace::resume::codex::start(&handle);
            workspace::resume::claude::start(&handle);
            if let Err(error) = ai::agent_hud::start(&handle) {
                platform::notifications::report(&handle, format!("Agent 悬浮条未能打开，可在设置中重试：{error}"));
            }

            // 默认浏览器联动：启动时对齐（残留备份还原 / 全局+TUN 直接切换）
            if let Err(error) = fingerprint_browser::defaultbrowser::prepare_registration() { platform::notifications::report(&handle, format!("浏览器入口注册失败：{error:#}")); }
            if let Err(error) = fingerprint_browser::defaultbrowser::restore(&handle) { platform::notifications::report(&handle, format!("默认浏览器恢复失败：{error:#}")); }

            // 数据目录与内置 geodata
            let dd = data_dir(&handle)?;
            {
                let st = handle.state::<CoreState>();
                *st.data_dir.lock() = dd.clone();
            }
            let resource_dir = handle.path().resource_dir()?;
            proxy::core::ensure_geodata(&resource_dir, &dd)?;

            // 订阅存储初始化
            let pm = ProfileManager::new(&dd);
            let profiles = pm.load()?;
            if !dd.join("profiles.yaml").exists() {
                pm.save(&profiles)?;
            }

            // 自启动
            let autostart = handle.autolaunch();
            if cfg.enable_auto_launch {
                if let Err(error) = autostart.enable() { platform::notifications::report(&handle, format!("启用开机启动失败：{error}")); }
            } else {
                if let Err(error) = autostart.disable() { platform::notifications::report(&handle, format!("关闭开机启动失败：{error}")); }
            }

            // 托盘
            // 自定义面板首次打开时再创建；创建失败或超时后回退到原生菜单。
            let mut tray = TrayIconBuilder::with_id("main-tray");
            if let Some(icon) = handle.default_window_icon() { tray = tray.icon(icon.clone()); }
            let tray_result = tray.tooltip("云阔")
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| handle_tray_event(app, event.id().as_ref()))
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Right,
                        button_state: tauri::tray::MouseButtonState::Up,
                        position,
                        ..
                    } = &event {
                        let handle = tray.app_handle().clone();
                        let position = *position;
                        // WebView2 不允许在 Windows 事件回调线程中同步创建窗口。
                        tauri::async_runtime::spawn_blocking(move || {
                            if let Err(error) = app::tray_menu::show(&handle, position) {
                                show_main(&handle);
                                platform::notifications::report(&handle, format!("打开快捷面板失败：{error}"));
                            }
                        });
                    }
                    if let tauri::tray::TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        button_state: tauri::tray::MouseButtonState::Up,
                        ..
                    } = event
                    {
                        // 主界面一出来，面板留着就是挡住它的一块浮层。
                        app::tray_menu::dismiss(tray.app_handle());
                        show_main(tray.app_handle());
                    }
                })
                .build(&handle);
            if let Err(error) = tray_result { show_main(&handle); platform::notifications::report_as(&handle, platform::notifications::Level::Warn, format!("托盘不可用，主窗口保持显示：{error}")); }
            else { update_tray(&handle); }
            // 快捷面板在启动后不久就预建（隐藏）：面板是全应用唯一「运行期才建」的 webview，
            // 留到第一次右键，正好会撞上 WebView2 运行时被后台静默升级的那个窗口期（0x80010108）。
            app::tray_menu::preload(&handle);

            // 主窗口先隐藏到首帧：前端画完启动画面会调用 main_show；
            // 前端异常时 5 秒兜底显示，避免用户只看到托盘图标。
            let first_frame = handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                if MAIN_FRAME_SHOWN.load(std::sync::atomic::Ordering::Acquire) { return; }
                if first_frame.state::<ConfigMutex>().lock().enable_silent_start { return; }
                show_main(&first_frame);
            });

            // 静默启动
            if cfg.enable_silent_start && handle.tray_by_id("main-tray").is_some() {
                if let Some(win) = handle.get_window("main") {
                    let _ = win.hide();
                    let _ = handle.emit("app-visibility", false);
                }
            }

            // 系统代理守护
            proxy::core::spawn_supervisor(handle.clone());

            // 订阅自动更新循环（每 30 分钟检查）
            let h = handle.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(1800)).await;
                    if let Err(error) = auto_update_profiles(&h).await { platform::notifications::report(&h, format!("订阅后台更新或重载失败：{error:#}")); }
                }
            });

            // 启动内核（异步）
            let h = handle.clone();
            tauri::async_runtime::spawn(async move {
                {
                let environment=h.state::<platform::state::EnvironmentOperation>();
                let _environment=environment.lock().await;
                let operation = h.state::<platform::state::ConfigUpdateMutex>();
                let _operation = operation.lock().await;
                if let Err(e) = proxy::core::start_core(&h).await {
                    platform::notifications::report(&h, format!("{e:#}"));
                }
                if let Err(error) = fingerprint_browser::defaultbrowser::sync(&h) { platform::notifications::report(&h, format!("默认浏览器联动失败：{error:#}")); }
                }
                // 启动诊断：对比内存/磁盘配置并记录真实探测结果（probe.log）
                {
                    let cfgd = h.state::<ConfigMutex>().lock().clone();
                    let dd = match data_dir(&h) { Ok(dir) => dir, Err(error) => { platform::notifications::report(&h, format!("读取数据目录失败：{error}")); return; } };
                    let disk = proxy::core::disk_cfg_snapshot(&dd.join("config.yaml"));
                    let probe = proxy::core::probe_with_fallback(&cfgd, &dd).await;
                    core::write_probe_log(
                        &dd,
                        &format!(
                            "startup → ok={:?}; controller_port mem={}/disk={}; secret_len mem={}/disk={}",
                            probe.as_ref().ok().map(|_| true),
                            cfgd.external_controller_port,
                            disk.0,
                            cfgd.secret.len(),
                            disk.1,
                        ),
                    );
                }
                // 启动后可选自动同步时区
                let cfg = h.state::<ConfigMutex>().lock().clone();
                if cfg.tz_auto_sync {
                    if let Err(error) = app::environment::sync_timezone_by_ip(h.clone()).await { platform::notifications::report(&h, format!("自动同步时区失败：{error}")); }
                }
                fingerprint_browser::browser::handle_arguments(h.clone(), std::env::args().collect()).await;
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == ai::agent_hud::LABEL {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    if let Err(error) = ai::agent_hud::set_enabled(window.app_handle(), false) { platform::notifications::report(window.app_handle(), error); }
                }
                return;
            }
            if window.label() == app::tray_menu::LABEL {
                match event {
                    // 拿到过焦点的面板靠这条收起；没拿到焦点时收不到这个事件，
                    // 由 tray_menu 里的「点到外面」监听兜底。
                    WindowEvent::Focused(false) => app::tray_menu::dismiss(window.app_handle()),
                    WindowEvent::CloseRequested { api, .. } => { api.prevent_close(); app::tray_menu::dismiss(window.app_handle()); }
                    _ => {}
                }
                return;
            }
            if window.label() != "main" { return; }
            if matches!(event, WindowEvent::Focused(_) | WindowEvent::Resized(_)) {
                let visible = window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false);
                let _ = window.emit("app-visibility", visible);
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let cfg = app.state::<ConfigMutex>().lock().clone();
                match close_action(
                    cfg.close_to_tray,
                    app.tray_by_id("main-tray").is_some(),
                    app.state::<ForceExit>().allowed(),
                ) {
                    // 关闭到托盘：沿用原语义，不问。
                    CloseAction::HideToTray => {
                        let _ = window.hide();
                        let _ = window.emit("app-visibility", false);
                        api.prevent_close();
                    }
                    // HUD 与隐藏的托盘 WebView 会让进程在主窗口销毁后继续活着，
                    // 托盘图标创建失败时也必须能退出，所以这里统一走 exit()。
                    CloseAction::Exit => {
                        api.prevent_close();
                        app.exit(0);
                    }
                }
            }
        })
        .build(tauri::generate_context!());
    let application = match application {
        Ok(application) => application,
        Err(_) => {
            faults::record(faults::Kind::StartupFailed);
            show_startup_error(
                "软件初始化失败。请检查安装完整性、配置文件和 WebView2。原始配置已保留，故障类型已尝试记录到本机诊断目录。",
            );
            return;
        }
    };
    application.run(|app, event| {
        if let RunEvent::WindowEvent {
            label,
            event: WindowEvent::Destroyed,
            ..
        } = &event
        {
            if label == ai::agent_hud::LABEL {
                ai::agent_hud::destroyed(app);
            }
        }
        if let RunEvent::ExitRequested { api, .. } = &event {
            let forced = app.state::<ForceExit>().allowed();
            let exit = app.state::<ProxyExit>();
            if !forced && !exit.allowed.load(std::sync::atomic::Ordering::SeqCst) {
                // Do not destroy the process/Job Object until its proxy lease
                // has been restored. Failed restoration leaves a usable app.
                // 用户在确认框里选了「仍要退出」时不再等代理恢复：清理阶段仍会尽力停内核。
                api.prevent_exit();
                if exit.pending.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                let handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    let environment = handle.state::<platform::state::EnvironmentOperation>();
                    let _environment = environment.lock().await;
                    let configuration = handle.state::<platform::state::ConfigUpdateMutex>();
                    let _configuration = configuration.lock().await;
                    // 恢复系统代理可能卡在内核或系统调用上；给它一个上限，超时也走确认框，
                    // 不能像以前那样把窗口晾在那里什么都不发生。
                    let outcome = tokio::time::timeout(
                        std::time::Duration::from_secs(8),
                        proxy::core::stop_core_async(&handle),
                    )
                    .await
                    .map_err(|_| "代理恢复超时（8 秒）".to_string())
                    .and_then(|result| result.map_err(|error| format!("{error:#}")));
                    match outcome {
                        Ok(()) => {
                            handle
                                .state::<ProxyExit>()
                                .allowed
                                .store(true, std::sync::atomic::Ordering::SeqCst);
                            handle.exit(0);
                        }
                        Err(detail) => {
                            handle
                                .state::<ProxyExit>()
                                .pending
                                .store(false, std::sync::atomic::Ordering::SeqCst);
                            show_main(&handle);
                            // 代理没能恢复时也不能把窗口当哑巴：给一条可以强制退出的确认。
                            if let Some(window) = handle.get_webview_window("main") {
                                let _ = window.emit(
                                    "exit-blocked",
                                    ExitBlocked {
                                        reason: "proxy",
                                        detail: Some(detail),
                                    },
                                );
                            }
                        }
                    }
                });
            }
        }
        if let RunEvent::Exit = event {
            workspace::resume::codex::stop(app);
            workspace::resume::claude::stop(app);
            // 退出清理：停内核、清系统代理、还原默认浏览器
            proxy::core::stop_core(app);
            let cfg = app.state::<ConfigMutex>().lock().clone();
            if cfg.default_browser_backup.is_some() {
                let _ = fingerprint_browser::defaultbrowser::restore(app);
            }
            faults::finish();
        }
    });
}
