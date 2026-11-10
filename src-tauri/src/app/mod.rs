//! 应用装配与具体流程；业务事实仍由各业务模块维护。
pub(crate) mod ai;
pub(crate) mod commands;
pub(crate) mod config_update;
pub(crate) mod desktop;
pub(crate) mod environment;
pub(crate) mod maintenance;
pub(crate) mod migrate;
pub(crate) mod network;
#[cfg(feature = "performance-probes")]
pub(crate) mod performance_probe;
pub(crate) mod proxy;
pub(crate) mod tray_menu;
pub(crate) mod uninstall;
pub(crate) mod updates;

/// 自建窗口必须与 tauri.conf.json 中主窗口保持同一组 WebView2 参数：
/// 同一数据目录下创建参数不同的 WebView2 环境会失败，因此不能只改一处。
/// 这里禁用「非代理 UDP」，让 WebRTC 只走经代理的 UDP；代价是 STUN 探测
/// 将拿不到任何公网候选（检测结论变为「未取得公网候选」，而非「已禁用」）。
/// 前三个参数是 wry 的默认值：一旦显式设置 additional_browser_args，
/// 默认串会被整体替换，必须自己补回来，否则 WebView2 的右键菜单、
/// PDF 工具栏和 SmartScreen 会重新出现。
pub(crate) const WEBVIEW_BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --force-webrtc-ip-handling-policy=disable_non_proxied_udp --disable-non-proxied-udp";
