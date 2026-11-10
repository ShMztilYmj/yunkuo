//! 报告必须属于当前环境和当前应用会话；事件失效之外还在使用时复核。
use super::detect::{WebScanMutex, WebScanState};
use crate::config::AppConfig;
use serde::Serialize;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};

/// 配置内容只参与摘要，不进入报告、前端或日志。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EnvironmentBasis {
    config_digest: Vec<u8>,
    core_generation: u64,
    network_revision: u64,
}

impl EnvironmentBasis {
    pub(crate) fn capture(app: &AppHandle) -> Self {
        let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
        Self::from_config(
            &cfg,
            app.state::<crate::proxy::core::CoreState>()
                .generation
                .load(Ordering::SeqCst),
            app.state::<crate::proxy::core::CoreState>()
                .network_revision
                .load(Ordering::SeqCst),
        )
    }

    pub(super) fn from_config(
        cfg: &AppConfig,
        core_generation: u64,
        network_revision: u64,
    ) -> Self {
        Self {
            config_digest: config_digest(cfg),
            core_generation,
            network_revision,
        }
    }

    /// 检验环境绑定覆盖配置及运行代次，不暴露原始配置。
    #[cfg(test)]
    pub(crate) fn binding(&self) -> String {
        let mut bytes = self.config_digest.clone();
        bytes.extend_from_slice(&self.core_generation.to_be_bytes());
        bytes.extend_from_slice(&self.network_revision.to_be_bytes());
        ring::digest::digest(&ring::digest::SHA256, &bytes)
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

/// 只绑定会改变网络或放行策略的字段。托盘、快捷方式和默认浏览器备份更新
/// 不应使一个仍然有效的报告失效；系统/浏览器副作用由编排主动撤销依据。
///
/// 只取配置本身、不含运行代次，因此可以跨重启比较：上次检测摘要靠它判断
/// “旧结论是否还代表当前网络策略”。
pub(crate) fn config_digest(cfg: &AppConfig) -> Vec<u8> {
    let config = serde_json::json!({
        "proxyEngine": cfg.proxy_engine,
        "mixedPort": cfg.mixed_port,
        "controllerPort": cfg.external_controller_port,
        "controllerSecret": cfg.secret,
        "systemProxy": cfg.enable_system_proxy,
        "proxyGuard": cfg.enable_proxy_guard,
        "proxyBypass": cfg.proxy_bypass,
        "tun": cfg.enable_tun,
        "mode": cfg.mode,
        "networkMode": cfg.network_mode,
        "dailyProxy": cfg.daily_proxy,
        "allowLan": cfg.allow_lan,
        "ipv6": cfg.ipv6,
        "gateThreshold": cfg.gate_threshold,
        "requireClaudeReachable": cfg.require_claude_reachable,
        "blockedCountries": cfg.blocked_countries,
    });
    let bytes = serde_json::to_vec(&config).expect("environment view serialization is infallible");
    ring::digest::digest(&ring::digest::SHA256, &bytes)
        .as_ref()
        .to_vec()
}

pub(crate) fn config_digest_hex(cfg: &AppConfig) -> String {
    config_digest(cfg)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Clone)]
pub(crate) struct ReportSnapshot {
    state: WebScanState,
    basis: EnvironmentBasis,
}

impl ReportSnapshot {
    pub(crate) fn score(&self) -> Option<i32> {
        self.state.score()
    }
}

pub(crate) fn snapshot(app: &AppHandle) -> Result<ReportSnapshot, String> {
    let basis = EnvironmentBasis::capture(app);
    let state = app.state::<WebScanMutex>().lock().clone();
    if !state.valid_for(&basis) {
        return Err("环境检测尚未完成、已过期或配置发生变化，请重新检测".into());
    }
    Ok(ReportSnapshot { state, basis })
}

pub(crate) fn still_current(app: &AppHandle, snapshot: &ReportSnapshot) -> bool {
    snapshot.basis == EnvironmentBasis::capture(app)
        && snapshot.state.valid_for(&snapshot.basis)
        && *app.state::<WebScanMutex>().lock() == snapshot.state
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReportStatus {
    pub revision: u64,
    pub valid: bool,
    pub age_seconds: Option<u64>,
    pub lifetime_seconds: u64,
}

#[tauri::command]
pub(crate) fn get_diagnostics_report_status(app: AppHandle) -> ReportStatus {
    let basis = EnvironmentBasis::capture(&app);
    let handle = app.state::<WebScanMutex>();
    let state = handle.lock();
    ReportStatus {
        revision: state.revision(),
        valid: state.valid_for(&basis),
        age_seconds: state.age_seconds(),
        lifetime_seconds: super::detect::REPORT_LIFETIME.as_secs(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_changes_for_config_and_runtime_generation() {
        let mut cfg = AppConfig::default();
        let baseline = EnvironmentBasis::from_config(&cfg, 1, 1);
        assert_eq!(baseline, EnvironmentBasis::from_config(&cfg, 1, 1));
        assert_ne!(
            baseline.binding(),
            EnvironmentBasis::from_config(&cfg, 2, 1).binding()
        );
        assert_ne!(
            baseline.binding(),
            EnvironmentBasis::from_config(&cfg, 1, 2).binding()
        );
        cfg.mixed_port += 1;
        assert_ne!(baseline, EnvironmentBasis::from_config(&cfg, 1, 1));
    }

    #[test]
    fn display_preferences_do_not_revoke_network_evidence() {
        let mut cfg = AppConfig::default();
        let baseline = EnvironmentBasis::from_config(&cfg, 1, 1);
        cfg.close_to_tray = !cfg.close_to_tray;
        cfg.claude_shortcut = "different-shortcut".into();
        assert_eq!(baseline, EnvironmentBasis::from_config(&cfg, 1, 1));
        cfg.gate_threshold += 1;
        assert_ne!(baseline, EnvironmentBasis::from_config(&cfg, 1, 1));
    }
}
