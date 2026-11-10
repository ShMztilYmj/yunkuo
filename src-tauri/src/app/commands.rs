//! 平台命令：配置读写、运行信息、数据目录与前端日志。

use crate::config::AppConfig;
use crate::diagnostics::timezone;
use crate::platform::state::{ConfigUpdateMutex, data_dir};
use crate::proxy::core;
use crate::proxy::sysproxy;
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt as _;
#[tauri::command]
pub fn get_config(state: State<'_, crate::config::ConfigMutex>) -> AppConfig {
    state.lock().clone()
}

#[tauri::command]
pub async fn patch_config(app: AppHandle, patch: Value) -> Result<AppConfig, crate::AppError> {
    patch_config_inner(app, patch)
        .await
        .map_err(crate::AppError::config)
}

async fn patch_config_inner(app: AppHandle, patch: Value) -> Result<AppConfig, String> {
    let parsed = crate::config::patch::ConfigPatch::parse(patch.clone())?;
    if parsed.changes_network_mode() {
        return Err("请通过网络模式面板切换，以保存备份并核对保护状态".into());
    }
    let environment_operation = app.state::<crate::platform::state::EnvironmentOperation>();
    let _environment_operation = environment_operation.lock().await;
    let operation = app.state::<ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    let before = app.state::<crate::config::ConfigMutex>().lock().clone();
    let candidate = parsed.apply(&before)?;
    let changed = parsed.needs_runtime_apply()
        && (runtime_replacement_required(&before, &candidate)
            || (!core::child_alive(&app)
                && (candidate.enable_system_proxy || candidate.enable_tun)));
    let rearm = strict_rearm_required(&before, changed).await?;
    patch_config_with_completion(&app, patch, rearm).await
}

/// 调用者必须持有 ConfigUpdateMutex，组合网络操作不能重入公共命令的锁。
pub(crate) async fn patch_config_locked(
    app: &AppHandle,
    patch: Value,
) -> Result<AppConfig, String> {
    // Scheme transitions explicitly arm/release after this transaction.
    patch_config_with_completion(app, patch, false).await
}

async fn patch_config_with_completion(
    app: &AppHandle,
    patch: Value,
    rearm: bool,
) -> Result<AppConfig, String> {
    crate::ai::management::require_ready(&data_dir(app)).map_err(|e| format!("{e:#}"))?;
    let before = app.state::<crate::config::ConfigMutex>().lock().clone();
    let patch = crate::config::patch::ConfigPatch::parse(patch)?;
    let cfg = patch.apply(&before)?;
    let runtime_requested = patch.needs_runtime_apply();
    let runtime_changed = runtime_requested && runtime_configuration_changed(&before, &cfg);
    let proxy_changed = patch.contains("enable_system_proxy")
        || cfg.mixed_port != before.mixed_port
        || cfg.proxy_bypass != before.proxy_bypass;
    let enabling_proxy = proxy_changed && cfg.enable_system_proxy;
    let core_active = core::child_alive(&app);
    let should_run = core_active || cfg.enable_tun || cfg.enable_system_proxy;
    let apply_core =
        should_run && (runtime_changed || (!core_active && (enabling_proxy || runtime_requested)));
    if apply_core {
        crate::app::network::prepare_proxy_runtime(app, &cfg).await?;
    }
    // A host switch may restore its proxy lease while replacing the core.
    // Reapply the requested takeover only after the replacement is ready.
    let proxy_changed = proxy_changed || (apply_core && cfg.enable_system_proxy);
    crate::app::config_update::run(&mut ConfigChange {
        app,
        before: &before,
        after: &cfg,
        apply_core,
        core_active,
        proxy_changed,
        core_applied: false,
        proxy_attempted: false,
    })
    .await
    .map_err(|error| {
        if rearm {
            format!("{error}；请核对严格保护状态并重新启用，恢复旧配置不代表保护已恢复放行")
        } else {
            error
        }
    })?;
    *app.state::<crate::config::ConfigMutex>().lock() = cfg.clone();
    if cfg.enable_auto_launch != before.enable_auto_launch {
        let autostart = app.autolaunch();
        let result = (if cfg.enable_auto_launch {
            autostart.enable()
        } else {
            autostart.disable()
        })
        .and_then(|_| autostart.is_enabled());
        match result {
            Ok(actual) if actual == cfg.enable_auto_launch => {}
            Ok(_) => crate::platform::notifications::report_as(
                app,
                crate::platform::notifications::Level::Warn,
                "开机启动设置未实际生效，请重新设置".into(),
            ),
            Err(error) => {
                crate::platform::notifications::report(app, format!("开机启动设置未完成：{error}"))
            }
        }
    }
    if !cfg.enable_system_proxy && before.enable_system_proxy && cfg.tz_restore_on_proxy_off {
        crate::app::environment::invalidate(app);
        if let Err(error) = timezone::restore_original(&data_dir(&app)) {
            crate::platform::notifications::report(
                app,
                format!("系统代理已关闭，但自动还原时区失败：{error:#}"),
            );
        } else {
            let mut restored = cfg.clone();
            restored.last_tz_sync = None;
            match crate::config::FileStore::new(data_dir(&app).join("config.yaml")).save(&restored)
            {
                Ok(()) => *app.state::<crate::config::ConfigMutex>().lock() = restored,
                Err(error) => crate::platform::notifications::report(
                    app,
                    format!("配置已保存、时区已恢复，但恢复记录保存失败：{error:#}"),
                ),
            }
        }
    }
    if let Err(error) = crate::fingerprint_browser::defaultbrowser::sync(&app) {
        crate::platform::notifications::report(app, format!("默认浏览器联动失败：{error:#}"));
    }
    let current = app.state::<crate::config::ConfigMutex>().lock().clone();
    if before.ai_tool_cards != current.ai_tool_cards {
        let scope = crate::ai::agent_tools::MonitoringScope::from_cards(&current.ai_tool_cards);
        crate::ai::aimon::engine::invalidate_scope(scope.clone());
        crate::ai::monitor::scope_changed(app);
        if let Err(error) = crate::ai::agent_events::set_scope(&scope) {
            crate::platform::notifications::report(app, error);
        }
    }
    finish_network_commit(app, &current, rearm).await?;
    Ok(current)
}

fn runtime_configuration_changed(before: &AppConfig, after: &AppConfig) -> bool {
    // 路由方式与日志等级交给 Engine 判断：能原地改就不重建会话。
    runtime_replacement_required(before, after)
        || before.mode != after.mode
        || before.log_level != after.log_level
}

/// Mode/log changes can be applied to the running mihomo process. Only changes
/// to interception, hosting or listeners require re-establishing protection.
fn runtime_replacement_required(before: &AppConfig, after: &AppConfig) -> bool {
    before.proxy_engine != after.proxy_engine
        || before.network_mode != after.network_mode
        || before.daily_proxy != after.daily_proxy
        || before.enable_tun != after.enable_tun
        || before.mixed_port != after.mixed_port
        || before.external_controller_port != after.external_controller_port
        || before.allow_lan != after.allow_lan
        || before.ipv6 != after.ipv6
}

/// Observe before a mutation: an already released protection must stay released.
pub(crate) async fn strict_rearm_required(
    before: &AppConfig,
    runtime_changed: bool,
) -> Result<bool, String> {
    let strict = before.network_mode == Some(crate::network_mode::NetworkMode::Strict);
    if !strict || !runtime_changed {
        return Ok(false);
    }
    let status = core::privacy_status()
        .await
        .map_err(|_| "无法确认严格保护状态，配置未应用，请先修复安全服务".to_string())?;
    let armed = status["armed"]
        .as_bool()
        .ok_or("严格保护状态不完整，配置未应用")?;
    Ok(crate::app::config_update::needs_rearm(
        strict,
        armed,
        runtime_changed,
    ))
}

struct NetworkCompletion<'a> {
    app: &'a AppHandle,
    config: &'a AppConfig,
}
impl crate::app::config_update::CompletionEffects for NetworkCompletion<'_> {
    fn config_changed(&mut self) {
        let _ = self.app.emit("config-changed", self.config);
    }
    async fn rearm(&mut self) -> Result<(), String> {
        core::wait_tun_ready(self.config)
            .await
            .map_err(|error| format!("TUN 核验失败：{error:#}"))?;
        verify_sysproxy(self.config)?;
        core::privacy_request("arm")
            .await
            .map(|_| ())
            .map_err(|error| format!("保护重验失败：{error:#}"))
    }
}

pub(crate) async fn finish_network_commit(
    app: &AppHandle,
    config: &AppConfig,
    rearm: bool,
) -> Result<(), String> {
    crate::app::config_update::finish_committed(&mut NetworkCompletion { app, config }, rearm).await
}

struct ConfigChange<'a> {
    app: &'a AppHandle,
    before: &'a AppConfig,
    after: &'a AppConfig,
    apply_core: bool,
    core_active: bool,
    proxy_changed: bool,
    core_applied: bool,
    proxy_attempted: bool,
}
fn verify_sysproxy(cfg: &AppConfig) -> Result<(), String> {
    let actual = sysproxy::get_sysproxy().map_err(|e| format!("系统代理状态核验失败：{e:#}"))?;
    let ours = actual.enable && actual.host == "127.0.0.1" && actual.port == cfg.mixed_port;
    if ours != cfg.enable_system_proxy {
        Err("系统代理未实际生效".into())
    } else {
        Ok(())
    }
}
impl crate::app::config_update::Effects for ConfigChange<'_> {
    async fn prepare(&mut self) -> Result<(), String> {
        if !self.after.enable_system_proxy
            && self.before.enable_system_proxy
            && self.after.tz_restore_on_proxy_off
        {
            let app = self.app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                crate::fingerprint_browser::browser::ensure_closed(&app)
                    .map_err(|e| format!("自动还原时区前请关闭指纹浏览器：{e:#}"))
            })
            .await
            .map_err(|e| format!("核对浏览器状态失败：{e}"))??;
        }
        Ok(())
    }
    async fn commit(&mut self) -> Result<(), String> {
        if self.apply_core {
            if let Err(error) = core::apply_config(self.app, self.after).await {
                // Session owns replacement rollback. Do not restart that same
                // restored session again from this outer persistence transaction.
                if core::verify_proxy_ready(self.app, self.before)
                    .await
                    .is_err()
                {
                    sysproxy::release(self.app).map_err(|restore| {
                        format!("内核应用失败：{error:#}；原系统代理恢复失败：{restore:#}")
                    })?;
                }
                return Err(format!("内核应用失败：{error:#}"));
            }
            self.core_applied = true;
        }
        if self.proxy_changed {
            if self.after.enable_system_proxy {
                core::verify_proxy_ready(self.app, self.after)
                    .await
                    .map_err(|e| format!("系统代理未接管：{e:#}"))?;
            }
            self.proxy_attempted = true;
            sysproxy::apply(self.app, self.after)
                .map_err(|e| format!("系统代理应用失败：{e:#}"))?;
            verify_sysproxy(self.after)?;
        }
        crate::config::FileStore::new(data_dir(self.app).join("config.yaml"))
            .save(self.after)
            .map_err(|e| format!("保存配置失败：{e:#}"))
    }
    async fn rollback(&mut self) -> Vec<String> {
        let mut failures = vec![];
        if self.core_applied {
            let result = if self.core_active {
                core::apply_config(self.app, self.before).await
            } else {
                core::stop_core_async(self.app).await
            };
            if let Err(error) = result {
                failures.push(format!("内核恢复失败：{error:#}"));
            }
        }
        if self.proxy_attempted {
            let result = if self.before.enable_system_proxy && self.core_active {
                core::verify_proxy_ready(self.app, self.before)
                    .await
                    .and_then(|_| sysproxy::apply(self.app, self.before))
            } else {
                // The old config can request a proxy while no old core was
                // actually running. Restore its actual lease, not that wish.
                sysproxy::release(self.app)
            };
            if let Err(error) = result {
                failures.push(format!("系统代理恢复失败：{error:#}"));
                let _ = sysproxy::release(self.app);
            }
        }
        failures
    }
}

#[derive(Serialize)]
pub struct RuntimeInfo {
    pub controller: String,
    pub secret: String,
    pub mixed_port: u16,
    pub data_dir: String,
}

#[tauri::command]
pub fn get_runtime_info(app: AppHandle) -> RuntimeInfo {
    let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
    RuntimeInfo {
        controller: cfg.controller_addr(),
        secret: cfg.secret,
        mixed_port: cfg.mixed_port,
        data_dir: data_dir(&app).to_string_lossy().to_string(),
    }
}

#[tauri::command]
pub fn open_data_dir(app: AppHandle) -> Result<(), String> {
    let dd = data_dir(&app);
    std::process::Command::new("explorer")
        .arg(dd.to_string_lossy().to_string())
        .spawn()
        .map_err(|e| format!("{e}"))?;
    Ok(())
}

#[tauri::command]
pub fn is_elevated_cmd() -> bool {
    core::is_elevated()
}

/// 前端检测流程写入诊断日志（与后端 probe.log 同文件，形成配对证据链）
#[tauri::command]
pub fn ui_log(app: AppHandle, line: String) {
    let dd = data_dir(&app);
    core::write_probe_log(&dd, &format!("UI {line}"));
}
