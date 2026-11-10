//! 初始化状态、脱敏诊断与升级前配置回退。
use crate::{
    config::{AppConfig, FileStore, atomic_write},
    platform::secure_storage::{crypt, safe},
    proxy::profile::{ProfileManager, ProfileStore},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::Path,
};
use tauri::{AppHandle, Emitter, Manager};
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"COCBACK1";
const LIMIT: usize = 64 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Backup {
    schema: u32,
    app_version: String,
    config: AppConfig,
    profiles: ProfileStore,
    files: BTreeMap<String, String>,
}

fn bounded_read(path: &Path) -> Result<Vec<u8>> {
    safe(path)?;
    crate::platform::files::read_bounded(path, LIMIT, "备份超过 64 MiB 上限")
}
fn snapshot(root: &Path, config: AppConfig) -> Result<Backup> {
    require_ready(root)?;
    let pm = ProfileManager::new(root);
    let profiles = pm.load()?;
    let mut files = BTreeMap::new();
    let mut total = 0;
    for item in &profiles.items {
        let bytes = bounded_read(&pm.file_path(&item.file)?)?;
        total += bytes.len();
        if total > LIMIT {
            bail!("订阅内容合计超过 64 MiB");
        }
        files.insert(item.file.clone(), String::from_utf8(bytes)?);
    }
    let backup = Backup {
        schema: 1,
        app_version: env!("CARGO_PKG_VERSION").into(),
        config,
        profiles,
        files,
    };
    validate(&backup)?;
    Ok(backup)
}
fn validate(data: &Backup) -> Result<()> {
    if data.schema != 1 || data.profiles.items.len() > 128 || data.files.len() > 128 {
        bail!("不支持此备份格式或订阅过多");
    }
    let cfg = &data.config;
    if cfg.proxy_engine != "mihomo"
        || cfg.mixed_port == 0
        || cfg.external_controller_port == 0
        || cfg.mixed_port == cfg.external_controller_port
        || !(0..=100).contains(&cfg.gate_threshold)
        || !["rule", "global", "direct"].contains(&cfg.mode.as_str())
        || !["silent", "error", "warning", "info", "debug"].contains(&cfg.log_level.as_str())
    {
        bail!("备份中的配置参数无效");
    }
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    let mut total = 0;
    for item in &data.profiles.items {
        if item.uid.is_empty()
            || !ids.insert(&item.uid)
            || !names.insert(&item.file)
            || item.file.is_empty()
            || item.file.contains(['/', '\\', ':'])
            || item.file == "."
            || item.file == ".."
        {
            bail!("备份中的订阅标识或文件名无效");
        }
        if !["remote", "local"].contains(&item.item_type.as_str()) {
            bail!("未知的订阅类型");
        }
        if item.item_type == "remote"
            && !item.url.starts_with("https://")
            && !item.url.starts_with("http://")
        {
            bail!("订阅地址必须为 HTTP(S)");
        }
        let file = data.files.get(&item.file).context("备份缺少订阅文件")?;
        total += file.len();
        if file.len() > 10 * 1024 * 1024 || total > LIMIT {
            bail!("备份中的订阅内容超限");
        }
        crate::proxy::profile::validate_yaml(file).context("备份订阅内容无效")?;
    }
    if data.files.len() != names.len()
        || data
            .profiles
            .current
            .as_ref()
            .is_some_and(|id| !ids.contains(id))
    {
        bail!("备份订阅索引不一致");
    }
    Ok(())
}
fn portable_config(mut cfg: AppConfig) -> AppConfig {
    cfg.secret.clear();
    cfg.enable_tun = false;
    cfg.enable_system_proxy = false;
    cfg.enable_auto_launch = false;
    cfg.enable_silent_start = false;
    cfg.default_browser_backup = None;
    cfg.last_tz_sync = None;
    cfg.claude_shortcut.clear();
    cfg.chatgpt_shortcut.clear();
    cfg.grok_shortcut.clear();
    cfg.ai_tool_shortcuts.clear();
    cfg
}
#[cfg(test)]
fn encrypt(data: &Backup, password: &str) -> Result<Vec<u8>> {
    crate::platform::backup_crypto::seal(
        MAGIC,
        LIMIT,
        Zeroizing::new(serde_json::to_vec(data)?),
        password,
    )
}
fn decrypt(bytes: &[u8], password: &str) -> Result<Backup> {
    let plain = crate::platform::backup_crypto::open(MAGIC, LIMIT, bytes, password)?;
    let data: Backup = serde_json::from_slice(&plain).context("备份结构无效")?;
    validate(&data)?;
    Ok(data)
}
fn local_backup(path: &Path, data: &Backup) -> Result<()> {
    safe(path)?;
    let bytes = Zeroizing::new(serde_json::to_vec(data)?);
    if bytes.len() > LIMIT - 16384 {
        bail!("恢复记录超过大小上限，尚未修改配置");
    }
    let protected = crypt(&bytes, true)?;
    if protected.len() > LIMIT {
        bail!("加密恢复记录超过大小上限，尚未修改配置");
    }
    atomic_write(path, &protected)
}
fn read_local(path: &Path) -> Result<Backup> {
    let data: Backup = serde_json::from_slice(&crypt(&bounded_read(path)?, false)?)?;
    validate(&data)?;
    Ok(data)
}
fn write_data(root: &Path, data: &Backup) -> Result<()> {
    validate(data)?;
    let pm = ProfileManager::new(root);
    for (name, content) in &data.files {
        let path = pm.file_path(name)?;
        safe(&path)?;
        atomic_write(&path, content.as_bytes())?;
    }
    FileStore::new(root.join("profiles.yaml")).save(&data.profiles)?;
    FileStore::new(root.join("config.yaml")).save(&data.config)?;
    Ok(())
}

/// 冷启动在加载配置前调用；中断的恢复先回退到已持久化的原配置。
pub fn recover_pending(root: &Path) -> Result<()> {
    let pending = root.join("maintenance-pending.coc");
    if !pending.exists() {
        return Ok(());
    }
    let data =
        read_local(&pending).context("配置恢复被中断且回退记录无法验证，请保留数据目录并修复")?;
    write_data(root, &data)?;
    fs::remove_file(pending)?;
    Ok(())
}
fn restore(root: &Path, incoming: Backup, current: AppConfig) -> Result<AppConfig> {
    validate(&incoming)?;
    restore_validated(root, incoming, current)
}
fn restore_validated(root: &Path, mut incoming: Backup, current: AppConfig) -> Result<AppConfig> {
    let original = snapshot(root, current.clone())?;
    incoming.config = portable_config(incoming.config);
    incoming.config.migrate_listen_ports();
    incoming.config.normalize_managed_ipv6();
    // 自启动属于当前电脑的系统注册状态，不随便携配置迁移，避免内存偏好与注册表脱节。
    incoming.config.enable_auto_launch = current.enable_auto_launch;
    incoming.config.enable_silent_start = current.enable_silent_start;
    incoming.config.secret = current.secret;
    incoming.config.claude_shortcut = current.claude_shortcut;
    incoming.config.chatgpt_shortcut = current.chatgpt_shortcut;
    incoming.config.grok_shortcut = current.grok_shortcut;
    incoming.config.ai_tool_shortcuts = current.ai_tool_shortcuts;
    // 为导入文件生成新标识，避免事务提交前覆盖旧订阅正文。
    let mut renamed = BTreeMap::new();
    for item in &mut incoming.profiles.items {
        let old_uid = item.uid.clone();
        item.uid = uuid::Uuid::new_v4().simple().to_string();
        if incoming.profiles.current.as_ref() == Some(&old_uid) {
            incoming.profiles.current = Some(item.uid.clone());
        }
        let old_file = item.file.clone();
        item.file = format!("{}.yaml", item.uid);
        renamed.insert(item.file.clone(), incoming.files[&old_file].clone());
    }
    incoming.files = renamed;
    let pending = root.join("maintenance-pending.coc");
    require_ready(root)?;
    local_backup(&pending, &original)?;
    let commit = (|| -> Result<()> {
        write_data(root, &incoming)?;
        local_backup(&root.join("maintenance-last.coc"), &original)?;
        fs::remove_file(&pending)?;
        Ok(())
    })();
    if let Err(failure) = commit {
        recover_pending(root).context("配置恢复失败且自动回退未完成；启动入口将保留回退记录")?;
        return Err(failure.context("配置恢复失败，已恢复原配置"));
    }
    Ok(incoming.config)
}

pub fn require_ready(root: &Path) -> Result<()> {
    crate::platform::state::require_storage_ready(root)
}

pub fn before_upgrade(root: &Path, cfg: AppConfig) -> Result<()> {
    let _profiles = crate::proxy::profile::PROFILE_LOCK.lock();
    local_backup(
        &root.join("maintenance-before-upgrade.coc"),
        &snapshot(root, cfg)?,
    )
}

/// 调用者已按顺序持有环境锁和配置锁。先落下关闭状态，避免后续恢复失败时守护重开失效代理。
pub(crate) async fn pause_for_maintenance(app: &AppHandle) -> Result<AppConfig, String> {
    crate::proxy::core::stop_core_async(app)
        .await
        .map_err(|e| format!("停止内核失败：{e:#}"))?;
    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let mut cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
    cfg.enable_system_proxy = false;
    cfg.enable_tun = false;
    crate::proxy::sysproxy::apply(app, &cfg).map_err(|e| format!("关闭系统代理失败：{e:#}"))?;
    *app.state::<crate::config::ConfigMutex>().lock() = cfg.clone();
    let _ = app.emit("config-changed", &cfg);
    crate::app::environment::invalidate(app);
    FileStore::new(root.join("config.yaml"))
        .save(&cfg)
        .map_err(|e| format!("代理已停止，但关闭状态保存失败，请修复存储后重试：{e:#}"))?;
    crate::fingerprint_browser::defaultbrowser::restore(app)
        .map_err(|e| format!("代理已停止，但浏览器入口恢复失败：{e:#}"))?;
    let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
    Ok(cfg)
}

#[tauri::command]
pub async fn maintenance_status(app: AppHandle) -> Result<Value, String> {
    let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
    let (kernel, service, privacy) = tokio::join!(
        crate::proxy::core::probe_controller(&cfg),
        crate::proxy::core::service_ping(),
        crate::proxy::core::privacy_status()
    );
    let profiles = ProfileManager::new(&app.path().app_data_dir().map_err(|e| e.to_string())?)
        .load()
        .map_err(|e| format!("{e:#}"))?;
    let privacy = privacy.ok();
    Ok(
        json!({"appVersion":app.package_info().version.to_string(),"kernelVersion":kernel.as_ref().ok(),"coreRunning":kernel.is_ok(),
        "serviceVersion":service,"privacyArmed":privacy.as_ref().and_then(|v|v["armed"].as_bool()),
        "privacyHealthy":privacy.as_ref().is_some_and(|v|v["healthy"]==true&&v["tunnelOn"]==true),
        "hasProfile":profiles.current.is_some(),"profileCount":profiles.items.len(),
        "shortcutConfigured":!cfg.claude_shortcut.is_empty()&&Path::new(&cfg.claude_shortcut).is_file(),
        "browserAvailable":crate::fingerprint_browser::browser::engine(&app).is_some(),
        "canUndo":app.path().app_data_dir().map_err(|e|e.to_string())?.join("maintenance-last.coc").exists(),
        "canUndoUpgrade":app.path().app_data_dir().map_err(|e|e.to_string())?.join("maintenance-before-upgrade.coc").exists()}),
    )
}

#[tauri::command]
pub async fn export_diagnostics(app: AppHandle, path: String) -> Result<(), String> {
    let status = maintenance_status(app.clone()).await?;
    // 使用白名单构造诊断；不收集认证文件、订阅地址、节点、IP、用户路径或原始日志。
    let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
    let report = diagnostic_report(&status, &cfg);
    safe(Path::new(&path)).map_err(|e| format!("{e:#}"))?;
    atomic_write(
        Path::new(&path),
        &serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("{e:#}"))
}

fn diagnostic_report(status: &Value, cfg: &AppConfig) -> Value {
    let mut allowed = serde_json::Map::new();
    for key in ["appVersion", "kernelVersion", "serviceVersion"] {
        allowed.insert(
            key.into(),
            status[key]
                .as_str()
                .filter(|v| {
                    v.len() <= 80
                        && v.chars()
                            .all(|c| c.is_ascii_alphanumeric() || ".-+_".contains(c))
                })
                .map_or(Value::Null, |s| json!(s)),
        );
    }
    for key in [
        "coreRunning",
        "privacyArmed",
        "privacyHealthy",
        "hasProfile",
        "shortcutConfigured",
        "browserAvailable",
        "canUndo",
        "canUndoUpgrade",
    ] {
        allowed.insert(
            key.into(),
            status[key].as_bool().map_or(Value::Null, |v| json!(v)),
        );
    }
    allowed.insert(
        "profileCount".into(),
        status["profileCount"]
            .as_u64()
            .map_or(Value::Null, |v| json!(v)),
    );
    json!({"schema":1,"capturedAt":chrono::Utc::now().to_rfc3339(),"status":allowed,
        "settings":{"systemProxyEnabled":cfg.enable_system_proxy,"tunEnabled":cfg.enable_tun,"mode":if ["rule","global","direct"].contains(&cfg.mode.as_str()){&cfg.mode}else{"invalid"},"ipv6":cfg.ipv6},
        "privacy":"不含认证、订阅、节点、公网IP、用户路径和原始日志","acceptance":"此文件不是故障场景或零泄漏验收结果"})
}

#[tauri::command]
pub async fn restore_config_backup(
    app: AppHandle,
    path: Option<String>,
    password: String,
    undo: bool,
    before_upgrade: bool,
) -> Result<(), String> {
    let password = Zeroizing::new(password);
    let environment_operation = app.state::<crate::platform::state::EnvironmentOperation>();
    let _environment = environment_operation.lock().await;
    let operation = app.state::<crate::platform::state::ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    crate::fingerprint_browser::browser::ensure_closed(&app).map_err(|e| format!("{e:#}"))?;
    let privacy = crate::proxy::core::privacy_status()
        .await
        .map_err(|_| "无法确认保护状态，请先启动或修复安全服务再恢复配置".to_string())?;
    if privacy["armed"].as_bool() != Some(false) {
        return Err("请先关闭 AI 客户端并解除严格保护，再恢复配置".into());
    }
    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    // 在停止代理前完整验证输入；错误密码不会影响当前会话。
    let source_root = root.clone();
    let incoming = tokio::task::spawn_blocking(move || -> Result<Backup> {
        if undo {
            read_local(&source_root.join(if before_upgrade {
                "maintenance-before-upgrade.coc"
            } else {
                "maintenance-last.coc"
            }))
        } else {
            decrypt(
                &bounded_read(Path::new(&path.context("请选择备份文件")?))?,
                &password,
            )
        }
    })
    .await
    .map_err(|_| "验证备份未完成".to_string())?
    .map_err(|e| format!("{e:#}"))?;
    let cfg = pause_for_maintenance(&app).await?;
    let restored = tokio::task::spawn_blocking(move || {
        let _profiles = crate::proxy::profile::PROFILE_LOCK.lock();
        restore(&root, incoming, cfg)
    })
    .await
    .map_err(|_| "配置恢复任务被中断；下次启动会检查回退记录".to_string())?
    .map_err(|e| format!("{e:#}"))?;
    *app.state::<crate::config::ConfigMutex>().lock() = restored.clone();
    let _ = app.emit("config-changed", &restored);
    crate::app::environment::invalidate(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostics_never_copy_unexpected_status_or_config_fields() {
        let mut cfg = AppConfig::default();
        cfg.secret = "secret-fixture-token".into();
        cfg.mode = "https://private.invalid/secret-fixture-token".into();
        let report = diagnostic_report(
            &json!({"appVersion":"0.2.6","kernelVersion":"secret-fixture-token/","private":"secret-fixture-token","profileCount":1}),
            &cfg,
        );
        assert!(!report.to_string().contains("secret-fixture-token"));
        assert_eq!(report["status"]["appVersion"], "0.2.6");
        assert_eq!(report["status"]["profileCount"], 1);
    }
    fn empty() -> Backup {
        Backup {
            schema: 1,
            app_version: "fixture".into(),
            config: AppConfig::default(),
            profiles: ProfileStore::default(),
            files: BTreeMap::new(),
        }
    }
    #[test]
    fn password_and_integrity_are_required_before_restore() {
        let data = empty();
        let password = "fixture-password-long";
        let mut bytes = encrypt(&data, password).unwrap();
        assert!(decrypt(&bytes, "wrong-password-long").is_err());
        assert_eq!(decrypt(&bytes, password).unwrap().schema, 1);
        *bytes.last_mut().unwrap() ^= 1;
        assert!(decrypt(&bytes, password).is_err());
        assert!(decrypt(&bytes[..30], password).is_err());
    }
    #[test]
    fn portable_settings_exclude_machine_secrets_and_enable_flags() {
        let mut cfg = AppConfig::default();
        cfg.enable_tun = true;
        cfg.enable_system_proxy = true;
        cfg.claude_shortcut = "C:/private/location.exe".into();
        cfg.ai_tool_shortcuts
            .insert("kimi".into(), "C:/private/kimi.exe".into());
        cfg.ai_tool_cards.push("kimi".into());
        let portable = portable_config(cfg);
        assert!(portable.secret.is_empty());
        assert!(portable.claude_shortcut.is_empty());
        assert!(portable.ai_tool_shortcuts.is_empty());
        assert_eq!(portable.tool_cards(), ["claude", "chatgpt", "kimi"]);
        assert!(!portable.enable_tun && !portable.enable_system_proxy);
    }
    #[test]
    fn invalid_profile_paths_are_rejected() {
        let mut data = empty();
        data.profiles
            .items
            .push(crate::proxy::profile::ProfileItem {
                uid: "fixture".into(),
                item_type: "local".into(),
                file: "../auth.json".into(),
                ..Default::default()
            });
        assert!(validate(&data).is_err());
        data.config.mixed_port = data.config.external_controller_port;
        assert!(validate(&data).is_err());
    }
    #[test]
    fn interrupted_restore_rolls_back_on_startup() {
        let fixture = crate::test_support::FixtureDir::new();
        let root = fixture.path();
        let mut original = empty();
        original.config.mixed_port = 12345;
        local_backup(&root.join("maintenance-pending.coc"), &original).unwrap();
        let mut damaged = empty();
        damaged.config.mixed_port = 23456;
        write_data(root, &damaged).unwrap();
        recover_pending(root).unwrap();
        assert_eq!(
            FileStore::new(root.join("config.yaml"))
                .load::<AppConfig>()
                .unwrap()
                .mixed_port,
            12345
        );
        assert!(!root.join("maintenance-pending.coc").exists());
    }
    #[test]
    fn restore_preserves_local_secret_and_supports_undo() {
        let fixture = crate::test_support::FixtureDir::new();
        let root = fixture.path();
        let current = AppConfig {
            enable_auto_launch: true,
            enable_silent_start: true,
            ai_tool_shortcuts: [("kimi".into(), "C:/local/Kimi.exe".into())].into(),
            ..Default::default()
        };
        let mut initial = empty();
        initial.config = current.clone();
        write_data(root, &initial).unwrap();
        let mut incoming = empty();
        incoming.config.mixed_port = 12345;
        incoming
            .config
            .ai_tool_shortcuts
            .insert("kimi".into(), "C:/other/Kimi.exe".into());
        let cfg = restore(root, incoming, current.clone()).unwrap();
        assert_eq!(cfg.secret, current.secret);
        assert_eq!(cfg.mixed_port, 12345);
        assert_eq!(cfg.tool_shortcut("kimi"), Some("C:/local/Kimi.exe"));
        assert!(cfg.enable_auto_launch && cfg.enable_silent_start);
        let old = read_local(&root.join("maintenance-last.coc")).unwrap();
        let reverted = restore(root, old, cfg).unwrap();
        assert_eq!(reverted.mixed_port, current.mixed_port);
    }
    #[test]
    fn encrypted_subscription_restore_and_undo_preserve_original_files() {
        fn data(secret: &str) -> Backup {
            let mut data = empty();
            let content = format!(
                "proxies:\n- name: fixture\n  type: socks5\n  server: 127.0.0.1\n  port: 12345\n  password: {secret}\nproxy-groups: []\nrules: []\n"
            );
            data.files.insert("original.yaml".into(), content);
            data.profiles.current = Some("original".into());
            data.profiles
                .items
                .push(crate::proxy::profile::ProfileItem {
                    uid: "original".into(),
                    file: "original.yaml".into(),
                    item_type: "local".into(),
                    name: "fixture".into(),
                    ..Default::default()
                });
            data
        }
        let fixture = crate::test_support::FixtureDir::new();
        let root = fixture.path();
        let original = data("original-secret");
        let current = original.config.clone();
        write_data(root, &original).unwrap();
        let mut incoming = data("imported-secret");
        incoming.config = portable_config(incoming.config);
        let encrypted = encrypt(&incoming, "fixture-password-long").unwrap();
        assert!(
            !encrypted
                .windows(b"imported-secret".len())
                .any(|v| v == b"imported-secret")
        );
        let decoded = decrypt(&encrypted, "fixture-password-long").unwrap();
        let imported_body = decoded.files["original.yaml"].clone();
        let original_body = original.files["original.yaml"].clone();
        let cfg = restore(root, decoded, current).unwrap();
        assert_eq!(
            ProfileManager::new(root).current_content().unwrap(),
            Some(imported_body)
        );
        assert_eq!(
            fs::read_to_string(root.join("profiles/original.yaml")).unwrap(),
            original_body
        );
        let rollback = read_local(&root.join("maintenance-last.coc")).unwrap();
        restore(root, rollback, cfg).unwrap();
        assert_eq!(
            ProfileManager::new(root).current_content().unwrap(),
            Some(original_body)
        );
    }
}
