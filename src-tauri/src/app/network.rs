//! 网络命令：内核、安全服务、网络模式、订阅、系统代理与 TUN。

use crate::app::commands::{patch_config, patch_config_locked};
use crate::config::AppConfig;
use crate::platform::state::{ConfigUpdateMutex, data_dir};
use crate::proxy::core::{self, CoreState};
use crate::proxy::profile::{ManualQuota, ProfileManager};
use crate::proxy::sysproxy;
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};
/// 代理与网络的唯一只读投影。字段统一 snake_case，前端不再维护第二套命名。
#[derive(Serialize)]
pub struct CoreStatus {
    pub running: bool,
    pub adopted: bool,
    pub version: Option<String>,
    pub started_at: Option<i64>,
    pub elevated: bool,
    pub state: crate::proxy::api::Lifecycle,
    pub service_managed: bool,
    pub system_proxy: bool,
    pub tun: bool,
    pub last_error: Option<String>,
    /// 主界面的保护投影与内核状态共用一次 IPC，避免各自重复查询服务。
    pub privacy: Option<Value>,
    // ---- 以下由原 proxy_snapshot 合并进来，仍是同一次读取的观察结果 ----
    /// 控制器已就绪；与 `running` 同源，保留以便按语义命名读取。
    pub controller_ready: bool,
    /// 内核代际与网络修订号，供调用方发现读取期间发生过替换或网络变更。
    pub generation: u64,
    pub network_revision: u64,
    pub observed_at: String,
    /// 配置意图（是否要求 TUN），与上面表示实际网卡已起的 `tun` 区分。
    pub tun_requested: bool,
    /// 严格保护的布尔投影；`privacy` 未查询时为 None（未知），不代表未开启。
    pub protection: Option<bool>,
    /// 纯常量能力描述，读取不产生额外 IPC。
    pub capabilities: crate::proxy::api::Capabilities,
    /// 最近一次成功应用配置的方式；从未应用过时为 None。
    pub last_apply: Option<core::ApplyRecord>,
}

#[tauri::command]
pub async fn core_status(app: AppHandle) -> CoreStatus {
    // 只读命令不参与配置事务：UAC 等待或 TUN 启动期间界面仍能读到状态。
    let st = app.state::<CoreState>();
    let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
    // 只保留两条并发分支：服务嵌入能力要发 /ping，属于高频轮询里不值得的 IPC。
    let (observed, privacy) = tokio::join!(core::observe_status(&st, &cfg), async {
        if cfg.enable_tun || crate::proxy::service::registered() {
            core::privacy_status().await.ok()
        } else {
            None
        }
    });
    // 保护状态就是同一份隐私上报里的 armed，避免为布尔投影再查一次服务。
    let protection = privacy.as_ref().and_then(|value| value["armed"].as_bool());
    CoreStatus {
        running: observed.controller_ready,
        adopted: observed.adopted,
        version: observed.version,
        started_at: observed.started_at,
        elevated: core::is_elevated(),
        state: observed.phase,
        service_managed: observed.service_managed,
        system_proxy: observed.controller_ready && sysproxy::is_ours(&cfg),
        tun: observed.controller_ready
            && cfg.enable_tun
            && proxy_runtime::adapter_up(crate::privacy::TUN_NAME),
        last_error: observed.last_error,
        privacy,
        controller_ready: observed.controller_ready,
        generation: observed.generation,
        network_revision: observed.network_revision,
        observed_at: chrono::Utc::now().to_rfc3339(),
        tun_requested: cfg.enable_tun,
        protection,
        capabilities: crate::proxy::api::capabilities(true),
        last_apply: app.state::<CoreState>().last_apply(),
    }
}

#[tauri::command]
pub async fn core_start(app: AppHandle) -> Result<(), crate::AppError> {
    let environment = app.state::<crate::platform::state::EnvironmentOperation>();
    let _environment = environment.lock().await;
    let operation = app.state::<ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
    prepare_proxy_runtime(&app, &cfg).await?;
    let rearm = crate::app::commands::strict_rearm_required(&cfg, true).await?;
    crate::app::environment::invalidate(&app);
    core::start_core(&app)
        .await
        .map_err(|e| crate::AppError::new("core.start_failed", format!("{e:#}")))?;
    crate::app::commands::finish_network_commit(&app, &cfg, rearm)
        .await
        .map_err(crate::AppError::internal)
}

#[tauri::command]
pub async fn core_restart(app: AppHandle) -> Result<(), crate::AppError> {
    let environment = app.state::<crate::platform::state::EnvironmentOperation>();
    let _environment = environment.lock().await;
    let operation = app.state::<ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
    prepare_proxy_runtime(&app, &cfg).await?;
    let rearm = crate::app::commands::strict_rearm_required(&cfg, true).await?;
    crate::app::environment::invalidate(&app);
    core::restart_core(&app).await.map_err(|e| {
        let message = format!("重启内核失败：{e:#}");
        // 详情进右上角消息中心，页面只留一行结论。
        crate::platform::notifications::report_as(
            &app,
            crate::platform::notifications::Level::Error,
            message.clone(),
        );
        crate::AppError::new("core.restart_failed", message)
    })?;
    crate::app::commands::finish_network_commit(&app, &cfg, rearm)
        .await
        .map_err(crate::AppError::internal)
}

#[tauri::command]
pub async fn core_stop(app: AppHandle) -> Result<(), crate::AppError> {
    let environment = app.state::<crate::platform::state::EnvironmentOperation>();
    let _environment = environment.lock().await;
    let operation = app.state::<ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    crate::app::environment::invalidate(&app);
    core::stop_core_async(&app)
        .await
        .map_err(|e| crate::AppError::new("core.stop_failed", format!("{e:#}")))
}

/// Called only by explicit user mutations while the environment/configuration
/// locks are held. Validate the candidate before prompting for service setup.
pub(crate) async fn prepare_proxy_runtime(app: &AppHandle, cfg: &AppConfig) -> Result<(), String> {
    if cfg.enable_tun {
        if !core::secure_service_available().await {
            // Validate before UAC only when service setup is actually needed.
            // A ready service leaves compilation and preflight to Session once.
            let content = ProfileManager::new(&data_dir(app))
                .current_content()
                .map_err(|e| format!("读取订阅失败：{e:#}"))?;
            let overlay = crate::proxy::overlay::load(&data_dir(app))
                .map_err(|e| format!("订阅覆写无效：{e:#}"))?;
            core::generate_config_with_overlay(cfg, content.as_deref(), &overlay)
                .map_err(|e| format!("代理配置无效，原状态保留：{e:#}"))?;
            crate::proxy::network_preflight::ensure_tun_available()
                .map_err(|e| format!("{e:#}"))?;
            core::install_service(app)
                .await
                .map_err(|e| format!("TUN 服务准备失败：{e:#}；原网络配置保持不变"))?;
        }
    }
    Ok(())
}

async fn protection_armed(cfg: &AppConfig) -> Result<bool, String> {
    if !crate::proxy::service::registered() {
        if cfg.network_mode == Some(crate::network_mode::NetworkMode::Strict) {
            return Err("严格保护服务不可用，请先安装或修复服务再切换".into());
        }
        return Ok(false);
    }
    core::privacy_status()
        .await
        .map_err(|_| "无法确认严格保护状态，请先修复安全服务")?["armed"]
        .as_bool()
        .ok_or_else(|| "保护状态不完整，原配置保留".into())
}

// ---------- TUN 服务模式 ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatusInfo {
    /// 服务程序已安装（ProgramData 中存在且注册）
    pub installed: bool,
    /// 服务进程在线（可通信）
    pub running: bool,
    /// 服务托管的内核是否在运行
    pub core_running: bool,
    pub pid: u32,
    pub compatible: bool,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn service_status() -> ServiceStatusInfo {
    let health = crate::proxy::service::health().await;
    let (_, core_running, pid) = if health.running {
        core::service_query_status().await
    } else {
        (false, false, 0)
    };
    ServiceStatusInfo {
        installed: health.installed,
        running: health.running,
        compatible: health.compatible,
        error: health.error,
        core_running,
        pid,
    }
}

#[tauri::command]
pub async fn service_install(app: AppHandle) -> Result<(), crate::AppError> {
    let environment = app.state::<crate::platform::state::EnvironmentOperation>();
    let _environment = environment.lock().await;
    let operation = app.state::<ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
    if cfg.enable_tun && core::child_alive(&app) {
        return Err(crate::AppError::new(
            "service.tun_running",
            "请先关闭 TUN，再修复服务，避免中断当前连接；若服务内核仍在托管，重启云阔后再修复",
        ));
    }
    core::install_service(&app)
        .await
        .map_err(|e| crate::AppError::new("service.install_failed", format!("{e:#}")))
}

#[tauri::command]
pub async fn service_uninstall(app: AppHandle) -> Result<(), crate::AppError> {
    let environment = app.state::<crate::platform::state::EnvironmentOperation>();
    let _environment = environment.lock().await;
    let operation = app.state::<ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    if app.state::<crate::config::ConfigMutex>().lock().enable_tun {
        return Err(crate::AppError::new(
            "service.tun_running",
            "请先关闭 TUN，再卸载服务；普通代理不受影响",
        ));
    }
    let status = core::privacy_status().await.map_err(|_| {
        crate::AppError::new(
            "service.privacy_unknown",
            "无法确认防泄露状态，请先启动或修复新版安全服务，再确认保护已解除后卸载",
        )
    })?;
    if status["armed"].as_bool() != Some(false) {
        return Err(crate::AppError::new(
            "service.protection_armed",
            "请先关闭所有 AI 客户端并在设置中解除网络保护；保护状态未知时也不能卸载服务",
        ));
    }
    core::uninstall_service(&app)
        .await
        .map_err(|e| crate::AppError::new("service.uninstall_failed", format!("{e:#}")))
}

#[tauri::command]
pub async fn privacy_enable(app: AppHandle) -> Result<(), String> {
    set_network_mode(app, crate::network_mode::NetworkMode::Strict, false)
        .await
        .map(|_| ())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkModeBackupStatus {
    available: bool,
    created_at: Option<String>,
}

#[tauri::command]
pub fn network_mode_backup_status(app: AppHandle) -> Result<NetworkModeBackupStatus, String> {
    let path = data_dir(&app).join(crate::network_mode::BACKUP_FILE);
    if !path.exists() {
        return Ok(NetworkModeBackupStatus {
            available: false,
            created_at: None,
        });
    }
    let backup: crate::network_mode::ModeBackup = crate::config::FileStore::new(path)
        .load()
        .map_err(|e| format!("网络配置备份读取失败：{e:#}"))?;
    Ok(NetworkModeBackupStatus {
        available: true,
        created_at: Some(backup.created_at),
    })
}

fn network_patch(cfg: &AppConfig) -> Value {
    serde_json::json!({"network_mode":cfg.network_mode,"daily_proxy":cfg.daily_proxy,"mode":cfg.mode,
        "enable_tun":cfg.enable_tun,"enable_system_proxy":cfg.enable_system_proxy,"ipv6":cfg.ipv6,"allow_lan":cfg.allow_lan})
}

async fn apply_network_policy(
    app: &AppHandle,
    target: &AppConfig,
    arm: bool,
    currently_armed: bool,
) -> Result<AppConfig, String> {
    apply_network_patch(app, network_patch(target), arm, currently_armed).await
}

async fn apply_network_patch(
    app: &AppHandle,
    patch: Value,
    arm: bool,
    currently_armed: bool,
) -> Result<AppConfig, String> {
    app.state::<crate::diagnostics::detect::WebScanMutex>()
        .lock()
        .record(None)?;
    // The configuration transaction must observe the previous active runtime;
    // stopping here would make its rollback stop an already restored instance.
    // 仍有严格规则时，在它的阻断下准备日常配置，最后一步才明确解除。
    let cfg = patch_config_locked(app, patch).await.map_err(|error| {
        if currently_armed {
            format!("{error}；请核对严格保护状态并重新启用，恢复旧配置不代表保护已恢复放行")
        } else {
            error
        }
    })?;
    if arm {
        core::privacy_request("arm")
            .await
            .map_err(|e| format!("严格配置已保存，保护尚未就绪：{e:#}"))?;
    } else if currently_armed {
        crate::fingerprint_browser::defaultbrowser::restore(app)
            .map_err(|e| format!("默认浏览器恢复失败，严格阻断继续保留：{e:#}"))?;
        core::privacy_request("release")
            .await
            .map_err(|e| format!("新配置已准备，但严格阻断未确认解除：{e:#}"))?;
    }
    Ok(cfg)
}

#[tauri::command]
pub async fn set_network_mode(
    app: AppHandle,
    mode: crate::network_mode::NetworkMode,
    allow_release: bool,
) -> Result<AppConfig, String> {
    let environment_operation = app.state::<crate::platform::state::EnvironmentOperation>();
    let _environment_operation = environment_operation.lock().await;
    let operation = app.state::<ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    crate::app::maintenance::require_ready(&data_dir(&app)).map_err(|e| format!("{e:#}"))?;
    let before = app.state::<crate::config::ConfigMutex>().lock().clone();
    let mut target = crate::proxy::policy::managed_target(&before, mode);
    if mode == crate::network_mode::NetworkMode::Daily
        && core::verify_proxy_ready(&app, &before).await.is_ok()
    {
        if let Ok(payload) = core::controller_request(&before, "GET", "/proxies", None).await {
            // 旧配置里 GLOBAL 可能指向原「云阔·海外节点」组，按链走到底再取真实节点。
            if let Some(name) =
                crate::network_mode::selected_proxy(&payload, crate::network_mode::GLOBAL_GROUP)
            {
                target.daily_proxy = Some(name);
            }
        }
    }
    prepare_proxy_runtime(&app, &target).await?;
    let status = core::privacy_status()
        .await
        .map_err(|_| "无法确认严格保护状态，模式未切换，请先修复安全服务")?;
    let armed = status["armed"]
        .as_bool()
        .ok_or("保护状态不完整，模式未切换")?;
    if mode == crate::network_mode::NetworkMode::Daily && armed && !allow_release {
        return Err(
            "严格保护仍在运行，请先关闭 AI 客户端，并使用“解除严格保护并切换日常分流”按钮".into(),
        );
    }
    let backup_path = data_dir(&app).join(crate::network_mode::BACKUP_FILE);
    if !backup_path.exists() {
        let backup = crate::network_mode::ModeBackup {
            config: before.clone(),
            armed,
            core_running: core::child_alive(&app),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        crate::config::FileStore::new(backup_path)
            .save(&backup)
            .map_err(|e| format!("备份网络配置失败，模式未切换：{e:#}"))?;
    } else {
        let _: crate::network_mode::ModeBackup = crate::config::FileStore::new(backup_path)
            .load()
            .map_err(|e| format!("已有备份无法读取，模式未切换：{e:#}"))?;
    }
    apply_network_policy(
        &app,
        &target,
        mode == crate::network_mode::NetworkMode::Strict,
        armed,
    )
    .await
}

/// Select an original routing mode without silently releasing existing WFP protection.
#[tauri::command]
pub async fn set_routing_mode(
    app: AppHandle,
    mode: crate::network_mode::RoutingMode,
    allow_release: bool,
) -> Result<AppConfig, String> {
    let environment_operation = app.state::<crate::platform::state::EnvironmentOperation>();
    let _environment_operation = environment_operation.lock().await;
    let operation = app.state::<ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    crate::app::maintenance::require_ready(&data_dir(&app)).map_err(|e| format!("{e:#}"))?;
    let before = app.state::<crate::config::ConfigMutex>().lock().clone();
    let armed = protection_armed(&before).await?;
    let target = crate::network_mode::routing_target(&before, mode, armed, allow_release)
        .map_err(|e| format!("{e:#}"))?;
    // Ordinary routing keeps the existing proxy and TUN switches and subscription.
    // Under WFP, prepare and save the requested configuration before releasing it.
    apply_network_patch(
        &app,
        serde_json::json!({"mode":target.mode,"network_mode":null,"daily_proxy":null}),
        false,
        armed,
    )
    .await
}

#[tauri::command]
pub async fn restore_network_mode(
    app: AppHandle,
    allow_release: bool,
) -> Result<AppConfig, String> {
    let environment_operation = app.state::<crate::platform::state::EnvironmentOperation>();
    let _environment_operation = environment_operation.lock().await;
    let operation = app.state::<ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    let path = data_dir(&app).join(crate::network_mode::BACKUP_FILE);
    if !path.exists() {
        return Err("没有切换前的网络配置备份".into());
    }
    let backup: crate::network_mode::ModeBackup = crate::config::FileStore::new(path.clone())
        .load()
        .map_err(|e| format!("备份损坏，原状态保留：{e:#}"))?;
    let before = app.state::<crate::config::ConfigMutex>().lock().clone();
    let mut target = backup.restore_into(&before);
    if backup.armed {
        target =
            crate::proxy::policy::managed_target(&target, crate::network_mode::NetworkMode::Strict);
    }
    prepare_proxy_runtime(&app, &target).await?;
    let armed = protection_armed(&before).await?;
    if armed && !backup.armed && !allow_release {
        return Err("恢复将解除严格保护，请先关闭 AI 客户端后确认恢复".into());
    }
    let cfg = apply_network_policy(&app, &target, backup.armed, armed).await?;
    if !backup.core_running && !backup.armed {
        core::stop_core_async(&app)
            .await
            .map_err(|e| format!("配置已恢复，但停止内核失败：{e:#}"))?;
    }
    std::fs::remove_file(path).map_err(|e| format!("网络已恢复，但备份暂未清除：{e}"))?;
    Ok(cfg)
}

#[tauri::command]
pub async fn privacy_release(app: AppHandle) -> Result<(), String> {
    let environment = app.state::<crate::platform::state::EnvironmentOperation>();
    let _environment = environment.lock().await;
    let operation = app.state::<ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    core::stop_for_protection_release(&app)
        .await
        .map_err(|e| format!("解除前停止代理失败：{e:#}"))?;
    let mut cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
    cfg.enable_system_proxy = false;
    cfg.enable_tun = false;
    cfg.network_mode = None;
    cfg.daily_proxy = None;
    sysproxy::apply(&app, &cfg).map_err(|e| format!("还原系统代理失败：{e:#}"))?;
    crate::config::FileStore::new(data_dir(&app).join("config.yaml"))
        .save(&cfg)
        .map_err(|e| format!("{e:#}"))?;
    *app.state::<crate::config::ConfigMutex>().lock() = cfg.clone();
    // Persistence succeeded. Later restore/release failures retain blocking but
    // cannot leave other windows displaying an obsolete managed scheme.
    let _ = app.emit("config-changed", &cfg);
    crate::fingerprint_browser::defaultbrowser::restore(&app)
        .map_err(|e| format!("默认浏览器恢复失败，阻断保护已保留；修复后可再次解除：{e:#}"))?;
    core::privacy_request("release")
        .await
        .map_err(|e| format!("{e:#}"))?;
    app.state::<crate::diagnostics::detect::WebScanMutex>()
        .lock()
        .record(None)?;
    Ok(())
}

#[tauri::command]
pub async fn privacy_status() -> Result<Value, String> {
    core::privacy_status().await.map_err(|e| format!("{e:#}"))
}

// ---------- 订阅 ----------

#[tauri::command]
pub async fn list_profiles(
    app: AppHandle,
) -> Result<Vec<crate::proxy::profile::ProfileItem>, String> {
    let pm = ProfileManager::new(&data_dir(&app));
    Ok(pm.load().map_err(|e| format!("{e:#}"))?.items)
}

#[tauri::command]
pub async fn current_profile(
    app: AppHandle,
) -> Result<Option<crate::proxy::profile::ProfileItem>, String> {
    let pm = ProfileManager::new(&data_dir(&app));
    pm.current_summary().map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn create_profile(
    app: AppHandle,
    url: Option<String>,
    local_path: Option<String>,
    name: Option<String>,
) -> Result<crate::proxy::profile::ProfileItem, String> {
    let environment = app.state::<crate::platform::state::EnvironmentOperation>();
    let _environment = environment.lock().await;
    let operation = app.state::<ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    let pm = ProfileManager::new(&data_dir(&app));
    let item = if let Some(u) = url {
        pm.create_remote(&u, name).await
    } else if let Some(p) = local_path {
        pm.create_local(&p, name).await
    } else {
        Err(anyhow::anyhow!("必须提供 url 或 local_path"))
    };
    let item = item.map_err(|e| format!("{e:#}"))?;
    if pm
        .current_summary()
        .map_err(|e| format!("{e:#}"))?
        .as_ref()
        .is_some_and(|current| current.uid == item.uid)
    {
        reload_current_profile_locked(&app).await?;
    }
    Ok(item)
}

#[tauri::command]
pub async fn create_profile_node(
    app: AppHandle,
    node: String,
    protocol: Option<String>,
    name: Option<String>,
) -> Result<crate::proxy::profile::ProfileItem, String> {
    let environment = app.state::<crate::platform::state::EnvironmentOperation>();
    let _environment = environment.lock().await;
    let operation = app.state::<ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    let pm = ProfileManager::new(&data_dir(&app));
    let item = pm
        .create_from_node(&node, protocol.as_deref(), name)
        .await
        .map_err(|e| format!("{e:#}"))?;
    if pm
        .current_summary()
        .map_err(|e| format!("{e:#}"))?
        .as_ref()
        .is_some_and(|current| current.uid == item.uid)
    {
        reload_current_profile_locked(&app).await?;
    }
    Ok(item)
}

#[tauri::command]
pub async fn update_profile(app: AppHandle, uid: String) -> Result<(), String> {
    let environment = app.state::<crate::platform::state::EnvironmentOperation>();
    let _environment = environment.lock().await;
    let operation = app.state::<ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    let pm = ProfileManager::new(&data_dir(&app));
    pm.update(&uid).await.map_err(|e| format!("{e:#}"))?;
    if pm
        .current_summary()
        .map_err(|e| format!("{e:#}"))?
        .as_ref()
        .is_some_and(|current| current.uid == uid)
    {
        reload_current_profile_locked(&app).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn delete_profile(app: AppHandle, uid: String) -> Result<(), String> {
    let environment = app.state::<crate::platform::state::EnvironmentOperation>();
    let _environment = environment.lock().await;
    let operation = app.state::<ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    let pm = ProfileManager::new(&data_dir(&app));
    let before = pm.load().map_err(|e| format!("{e:#}"))?.current;
    pm.delete(&uid).map_err(|e| format!("{e:#}"))?;
    if pm.load().map_err(|e| format!("{e:#}"))?.current != before {
        reload_current_profile_locked(&app).await?;
    }
    Ok(())
}

/// Caller holds EnvironmentOperation followed by ConfigUpdateMutex across
/// profile mutation and reload, so a new subscription cannot race a rollback.
async fn reload_current_profile_locked(app: &AppHandle) -> Result<(), String> {
    let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
    if core::child_alive(app) || cfg.enable_system_proxy || cfg.enable_tun {
        prepare_proxy_runtime(app, &cfg).await?;
        let rearm = crate::app::commands::strict_rearm_required(&cfg, true).await?;
        crate::app::proxy::invalidate_network(app);
        core::start_core(app)
            .await
            .map_err(|e| format!("订阅已保存，但代理应用失败，旧会话已尝试保留：{e:#}"))?;
        crate::app::commands::finish_network_commit(app, &cfg, rearm).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn patch_profile(
    app: AppHandle,
    uid: String,
    name: Option<String>,
    update_interval: Option<u64>,
) -> Result<(), String> {
    let environment = app.state::<crate::platform::state::EnvironmentOperation>();
    let _environment = environment.lock().await;
    let operation = app.state::<ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    let pm = ProfileManager::new(&data_dir(&app));
    pm.patch(&uid, name, update_interval)
        .map_err(|e| format!("{e:#}"))
}

/// 手动额度：本地导入、或服务商没有返回 `subscription-userinfo` 时，
/// 由用户自己填总量与到期，卡片才有「剩余 / 到期」可显示。
///
/// `quota = None` 表示清除；「不限量 + 不到期」由 `ManualQuota` 自身表达。
/// 只写本地记录，不触碰内核，因此这里不重载代理。
#[tauri::command]
pub async fn set_profile_quota(
    app: AppHandle,
    uid: String,
    quota: Option<ManualQuota>,
) -> Result<(), String> {
    let environment = app.state::<crate::platform::state::EnvironmentOperation>();
    let _environment = environment.lock().await;
    let operation = app.state::<ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    let pm = ProfileManager::new(&data_dir(&app));
    pm.set_quota(&uid, quota).map_err(|e| format!("{e:#}"))
}

/// 切换订阅并热重载内核
#[tauri::command]
pub async fn select_profile(app: AppHandle, uid: String) -> Result<(), String> {
    let environment = app.state::<crate::platform::state::EnvironmentOperation>();
    let _environment = environment.lock().await;
    let operation = app.state::<ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    let pm = ProfileManager::new(&data_dir(&app));
    pm.select(&uid).map_err(|e| format!("{e:#}"))?;
    reload_current_profile_locked(&app).await
}

/// 历史控制器入口只读兼容；网络写入必须进入会话和配置事务。
#[tauri::command]
pub async fn native_proxy_request(
    app: AppHandle,
    method: String,
    path: String,
    body: Option<Value>,
) -> Result<Value, String> {
    if !method.eq_ignore_ascii_case("GET") || body.as_ref().is_some_and(|value| !value.is_null()) {
        return Err("此入口仅支持读取；请通过代理设置、节点选择或连接管理修改网络".into());
    }
    // 只读通道不占用配置事务：验证通过后读控制器，长操作期间也能取到当前视图。
    let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
    core::verify_proxy_ready(&app, &cfg)
        .await
        .map_err(|e| format!("{e:#}"))?;
    core::controller_request(&cfg, "GET", &path, None)
        .await
        .map_err(|e| format!("{e:#}"))
}

/// 旧客户端的命令别名；所有请求均进入本软件mihomo 代理。
#[tauri::command]
pub async fn mihomo_request(
    app: AppHandle,
    method: String,
    path: String,
    body: Option<Value>,
) -> Result<Value, String> {
    native_proxy_request(app, method, path, body).await
}

// ---------- 系统代理 ----------

#[tauri::command]
pub async fn connection_totals(app: AppHandle) -> Result<core::ConnectionTotals, String> {
    let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
    core::connection_totals(&cfg)
        .await
        .map_err(|e| format!("{e:#}"))
}

// 系统代理的完整快照不再单独开命令：界面判断“是否指向本软件”用 core_status 的
// system_proxy，诊断页的 sysproxyOn/Host/Port 由 get_ip_info 提供。

/// 系统代理与其他入口共用配置事务：先就绪内核，再接管并保存。
#[tauri::command]
pub async fn set_sysproxy(app: AppHandle, enable: bool) -> Result<AppConfig, crate::AppError> {
    patch_config(app, serde_json::json!({ "enable_system_proxy": enable })).await
}

/// TUN 开关：同一事务内自动准备服务，确认网卡启动后提交配置。
#[tauri::command]
pub async fn set_tun(app: AppHandle, enable: bool) -> Result<AppConfig, crate::AppError> {
    patch_config(app, serde_json::json!({ "enable_tun": enable })).await
}

// ---------- 端口占用 ----------

/// 代理页进入时读取一次：端口被其他程序占用时由界面弹窗提示。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyPortUsage {
    pub port: u16,
    pub occupied: bool,
    /// 占用者就是本应用内核（正常监听），不算冲突。
    pub owned_by_core: bool,
    pub pid: Option<u32>,
    pub process: Option<String>,
}

fn process_display_name(pid: u32) -> Option<String> {
    use sysinfo::{Pid, System};
    let system = System::new_all();
    system
        .process(Pid::from_u32(pid))
        .map(|process| process.name().to_string_lossy().into_owned())
        .filter(|name| !name.trim().is_empty())
}

/// 只读命令：不启动、不停止内核，只回答“当前混合代理端口被谁占着”。
/// 端口表与进程名读取放到阻塞线程，避免进入代理页时卡住界面线程。
#[tauri::command]
pub async fn proxy_port_usage(app: AppHandle) -> ProxyPortUsage {
    let port = app.state::<crate::config::ConfigMutex>().lock().mixed_port;
    let core_alive = core::child_alive(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        let owner = proxy_runtime::tcp_listener_owner(address).ok().flatten();
        // 端口被自己上一轮遗留的内核占着不算冲突：按内核可执行文件名识别，
        // 应用托管与服务托管（ProgramData 下的副本）都能认出来。
        let engine_owned = owner.is_some()
            && proxy_runtime::engine_path()
                .map(|engine| proxy_runtime::listener_owned_by_engine(address, &engine))
                .unwrap_or(false);
        ProxyPortUsage {
            port,
            occupied: owner.is_some(),
            owned_by_core: owner.is_some() && (core_alive || engine_owned),
            pid: owner,
            process: owner.and_then(process_display_name),
        }
    })
    .await
    .unwrap_or(ProxyPortUsage {
        port,
        occupied: false,
        owned_by_core: false,
        pid: None,
        process: None,
    })
}
