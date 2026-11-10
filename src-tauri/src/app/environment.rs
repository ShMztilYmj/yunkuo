//! 环境跨模块编排：保持环境锁 → 配置锁顺序，先关闭浏览器再修复系统。
use crate::diagnostics::timezone;
use crate::fingerprint_browser::browser::{LaunchGate, NetworkEvidence, Runtime};
use crate::platform::state::{ConfigUpdateMutex, data_dir};
use crate::proxy::core;
use tauri::{AppHandle, Emitter, Manager};

pub(crate) use crate::platform::state::EnvironmentOperation as Operation;

struct BrowserLaunchGate;

impl LaunchGate for BrowserLaunchGate {
    fn authorize<'a>(
        &'a self,
        app: &'a AppHandle,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<NetworkEvidence, String>> + Send + 'a>,
    > {
        Box::pin(async move {
            let basis = crate::diagnostics::report::EnvironmentBasis::capture(app);
            let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
            crate::app::maintenance::require_ready(&data_dir(app)).map_err(|e| format!("{e:#}"))?;
            let network = crate::diagnostics::detect::proxy_network(&cfg).await;
            let failures = browser_network_failures(&network);
            if !failures.is_empty() {
                return Err(failures.join("；"));
            }
            if basis != crate::diagnostics::report::EnvironmentBasis::capture(app) {
                return Err("网络或配置在检查期间发生变化，请重新启动浏览器".into());
            }
            Ok(NetworkEvidence {
                mixed_port: cfg.mixed_port,
            })
        })
    }

    fn profile_slot(&self, app: &AppHandle) -> Result<String, String> {
        let base = crate::platform::files::environment_root(app).map_err(|e| format!("{e:#}"))?;
        let catalog = crate::ai::identity::vault::catalog(&base.join("vault"))
            .map_err(|e| format!("{e:#}"))?;
        if catalog.recovery.is_some() {
            return Err("身份恢复尚未完成，请先恢复后启动浏览器".into());
        }
        Ok(catalog.browser_slot)
    }
}

fn browser_network_failures(network: &crate::diagnostics::detect::ProxyNetwork) -> Vec<String> {
    let mut failures = Vec::new();
    if !network.sysproxy_on {
        failures.push("系统代理未开启或未指向本软件".into());
    }
    if !network.tun_on {
        failures.push("虚拟网卡（TUN）未实际开启".into());
    }
    if !matches!(network.mode.as_deref(), Some("rule" | "global")) {
        failures.push(
            "出站规则须为规则或全局模式，请关闭直连模式；若无法读取状态，请检查代理内核".into(),
        );
    }
    failures
}

pub(crate) fn install_browser_gate(app: &AppHandle) -> Result<(), String> {
    app.state::<Runtime>()
        .install_gate(std::sync::Arc::new(BrowserLaunchGate))
}

pub(crate) fn invalidate(app: &AppHandle) {
    let _ = app
        .state::<crate::diagnostics::detect::WebScanMutex>()
        .lock()
        .record(None);
    app.state::<crate::fingerprint_browser::browser::Runtime>()
        .invalidate();
    let _ = app.emit("environment-invalidated", ());
}

#[tauri::command]
pub(crate) async fn computer_environment_apply(
    app: AppHandle,
    region: String,
    restore: bool,
    confirmed: bool,
) -> Result<serde_json::Value, String> {
    if !confirmed {
        return Err("请确认系统设置变更后重试".into());
    }
    let environment = app.state::<Operation>();
    let _environment = environment.lock().await;
    let configuration = app.state::<ConfigUpdateMutex>();
    let _configuration = configuration.lock().await;
    crate::fingerprint_browser::browser::ensure_closed(&app).map_err(|e| format!("{e:#}"))?;
    invalidate(&app);
    let worker = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::diagnostics::computer::apply(&worker, &region, restore)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn sync_timezone_by_ip(app: AppHandle) -> Result<timezone::SyncResult, String> {
    let environment_operation = app.state::<Operation>();
    let _environment_operation = environment_operation.lock().await;
    let operation = app.state::<ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
    crate::fingerprint_browser::browser::ensure_closed(&app).map_err(|e| format!("{e:#}"))?;
    invalidate(&app);
    let dd = data_dir(&app);
    let core_on = core::probe_controller(&cfg).await.is_ok();
    let result =
        timezone::sync_timezone_by_ip(&dd, if core_on { Some(cfg.mixed_port) } else { None })
            .await
            .map_err(|e| format!("{e:#}"))?;
    if let Some(info) = &result.info {
        let st = app.state::<crate::config::ConfigMutex>();
        {
            let mut snapshot = st.lock().clone();
            snapshot.last_tz_sync = Some(info.clone());
            crate::config::FileStore::new(dd.join("config.yaml"))
                .save(&snapshot)
                .map_err(|e| format!("时区已设置，但记录保存失败：{e:#}"))?;
            *st.lock() = snapshot;
        }
    }
    Ok(result)
}

#[tauri::command]
pub async fn restore_timezone(app: AppHandle) -> Result<(), String> {
    let environment_operation = app.state::<Operation>();
    let _environment_operation = environment_operation.lock().await;
    let operation = app.state::<ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    crate::fingerprint_browser::browser::ensure_closed(&app).map_err(|e| format!("{e:#}"))?;
    invalidate(&app);
    let dd = data_dir(&app);
    timezone::restore_original(&dd).map_err(|e| format!("{e:#}"))?;
    let mut snapshot = app.state::<crate::config::ConfigMutex>().lock().clone();
    snapshot.last_tz_sync = None;
    crate::config::FileStore::new(dd.join("config.yaml"))
        .save(&snapshot)
        .map_err(|e| format!("时区已恢复，但记录保存失败：{e:#}"))?;
    *app.state::<crate::config::ConfigMutex>().lock() = snapshot;
    Ok(())
}

#[tauri::command]
pub async fn set_timezone_manual(app: AppHandle, name: String) -> Result<(), String> {
    let environment_operation = app.state::<Operation>();
    let _environment_operation = environment_operation.lock().await;
    let operation = app.state::<ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    crate::fingerprint_browser::browser::ensure_closed(&app).map_err(|e| format!("{e:#}"))?;
    invalidate(&app);
    timezone::backup_current(&data_dir(&app))
        .map_err(|e| format!("备份原时区失败，未修改：{e:#}"))?;
    timezone::set_timezone_by_name(&name).map_err(|e| format!("{e:#}"))?;
    let mut snapshot = app.state::<crate::config::ConfigMutex>().lock().clone();
    snapshot.last_tz_sync = None;
    crate::config::FileStore::new(data_dir(&app).join("config.yaml"))
        .save(&snapshot)
        .map_err(|e| format!("时区已修改，但记录保存失败：{e:#}"))?;
    *app.state::<crate::config::ConfigMutex>().lock() = snapshot;
    Ok(())
}

#[cfg(test)]
mod browser_launch_tests {
    use super::*;
    use crate::diagnostics::detect::ProxyNetwork;

    #[test]
    fn rule_and_global_allow_browser_without_strict_or_environment_report() {
        for mode in ["rule", "global"] {
            let network = ProxyNetwork {
                sysproxy_on: true,
                tun_on: true,
                mode: Some(mode.into()),
                ..Default::default()
            };
            assert!(browser_network_failures(&network).is_empty());
        }
    }

    #[test]
    fn system_proxy_tun_and_supported_routing_are_required() {
        let mut network = ProxyNetwork {
            sysproxy_on: true,
            tun_on: true,
            mode: Some("rule".into()),
            ..Default::default()
        };
        network.sysproxy_on = false;
        assert_eq!(browser_network_failures(&network).len(), 1);
        network.sysproxy_on = true;
        network.tun_on = false;
        assert_eq!(browser_network_failures(&network).len(), 1);
        network.tun_on = true;
        for mode in [Some("direct".into()), None, Some("unknown".into())] {
            network.mode = mode;
            assert_eq!(browser_network_failures(&network).len(), 1);
        }
        assert_eq!(browser_network_failures(&ProxyNetwork::default()).len(), 3);
    }
}
