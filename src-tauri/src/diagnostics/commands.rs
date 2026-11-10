//! 检测命令：时区同步、原生环境检测与首页 IP 概览。

use super::timezone;
use crate::config::TzSyncInfo;
use crate::platform::state::data_dir;
use crate::proxy::core;
use serde::Serialize;
use tauri::{AppHandle, Manager};
// ---------- 时区 ----------

#[derive(Serialize)]
pub struct TzStatus {
    pub current: timezone::CurrentTz,
    pub backup: Option<crate::config::TzBackup>,
    pub last_sync: Option<TzSyncInfo>,
    pub available: Vec<timezone::TzEntry>,
}

#[tauri::command]
pub async fn get_timezone_status(app: AppHandle) -> Result<TzStatus, String> {
    let dd = data_dir(&app);
    let current = timezone::get_current_tz().map_err(|e| format!("{e:#}"))?;
    let last_sync = app
        .state::<crate::config::ConfigMutex>()
        .lock()
        .last_tz_sync
        .clone();
    let available = timezone::list_timezones()
        .map_err(|e| format!("{e:#}"))?
        .into_iter()
        .filter(|e| !e.display.is_empty() && e.key_name != "UTC")
        .collect();
    Ok(TzStatus {
        current,
        backup: timezone::get_backup(&dd),
        last_sync,
        available,
    })
}

// ---------- 检测 ----------

#[tauri::command]
pub async fn detect_native(
    app: AppHandle,
) -> Result<crate::diagnostics::detect::NativeDetect, String> {
    let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
    let dd = data_dir(&app);
    Ok(crate::diagnostics::detect::detect_native(&cfg, &dd).await)
}

#[tauri::command]
pub fn record_web_scan(app: AppHandle, web_score: Option<i32>) -> Result<(), String> {
    let basis = super::report::EnvironmentBasis::capture(&app);
    app.state::<super::detect::WebScanMutex>()
        .lock()
        .record_for(web_score, basis)
}

/// 新客户端使用显式票据完成同一轮检测；旧命令继续作为兼容入口。
#[tauri::command]
pub fn begin_environment_scan(app: AppHandle) -> String {
    let basis = super::report::EnvironmentBasis::capture(&app);
    app.state::<super::detect::WebScanMutex>()
        .lock()
        .begin_scan(basis)
}

#[tauri::command]
pub fn finish_environment_scan(
    app: AppHandle,
    scan_id: String,
    web_score: i32,
) -> Result<(), String> {
    let basis = super::report::EnvironmentBasis::capture(&app);
    app.state::<super::detect::WebScanMutex>()
        .lock()
        .finish_scan(&scan_id, web_score, basis)
}

/// 首页概览用的 IP 信息（内网 IP + 出口 IP 归属地）
#[derive(Serialize, Default)]
pub struct IpInfo {
    pub local_ip: Option<String>,
    #[serde(rename = "exitIp")]
    pub exit_ip: Option<String>,
    #[serde(rename = "exitCountry")]
    pub exit_country: Option<String>,
    #[serde(rename = "exitCountryName")]
    pub exit_country_name: Option<String>,
}

#[tauri::command]
pub async fn get_ip_info(app: AppHandle) -> IpInfo {
    let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
    let core_on = core::probe_controller(&cfg).await.is_ok();
    let geo = crate::diagnostics::timezone::fetch_ip_geo(if core_on {
        Some(cfg.mixed_port)
    } else {
        None
    })
    .await
    .ok();
    let local_ip = if_addrs::get_if_addrs().ok().and_then(|ifs| {
        ifs.into_iter()
            .filter_map(|i| {
                if i.is_loopback() {
                    None
                } else if let if_addrs::IfAddr::V4(v4) = i.addr {
                    Some(v4.ip.to_string())
                } else {
                    None
                }
            })
            .next()
    });
    IpInfo {
        local_ip,
        exit_ip: geo.as_ref().map(|g| g.ip.clone()),
        exit_country: geo.as_ref().map(|g| g.country_code.clone()),
        exit_country_name: geo.as_ref().map(|g| g.country_name.clone()),
    }
}
