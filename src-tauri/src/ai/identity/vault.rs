//! 本地身份快照；仅白名单路径。浏览器资料独立保存，不包含在身份归档中。
use crate::platform::files::{environment_root as root, error};
use crate::platform::state::EnvironmentOperation as Operation;
use crate::platform::{archive_codec as codec, files};
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};
use tauri::Manager;
use zeroize::Zeroizing;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct Policy {
    pub history: bool,
    pub cache: bool,
    pub settings: bool,
    pub skills: bool,
    pub plugins: bool,
    pub mcp: bool,
    pub projects: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: String,
    pub name: String,
    pub email: Option<String>,
    pub created_at: String,
    pub bytes: u64,
    pub policy: Policy,
    pub browser_slot: String,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub records: Vec<Record>,
    pub active: Option<String>,
    pub browser_slot: String,
    pub recovery: Option<String>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
struct Spec {
    root: String,
    path: String,
    keys: Vec<String>,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    specs: Vec<Spec>,
    kinds: Vec<Option<bool>>,
}
struct Stage {
    path: PathBuf,
    manifest: Manifest,
}
impl Drop for Stage {
    fn drop(&mut self) {
        if self
            .path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("stage-"))
        {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn valid_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|u| u.to_string() == id)
}
pub fn catalog(dir: &Path) -> Result<Catalog> {
    let mut state: Catalog = files::read(&dir.join("catalog.json"))?;
    if state.browser_slot.is_empty() {
        state.browser_slot = "default".into();
    }
    if state.browser_slot != "default" && !valid_id(&state.browser_slot) {
        bail!("浏览器资料编号无效")
    }
    if state
        .records
        .iter()
        .any(|r| !valid_id(&r.id) || (r.browser_slot != "default" && !valid_id(&r.browser_slot)))
    {
        bail!("身份目录无效")
    }
    Ok(state)
}
pub fn require_ready(dir: &Path) -> Result<()> {
    if catalog(dir)?.recovery.is_some() {
        bail!("本地身份切换尚未恢复完成，请先到「环境与身份 → Claude 身份」恢复，再启动客户端")
    }
    Ok(())
}
fn persist(dir: &Path, state: &Catalog) -> Result<()> {
    files::write(&dir.join("catalog.json"), state)
}

fn begin_checkpoint(state: &mut Catalog, outgoing_id: &str) -> Option<String> {
    let previous = state.recovery.clone();
    // 恢复重试中的工作区可能是混合状态，不能替换最初的完整检查点。
    if previous.is_none() {
        state.recovery = Some(outgoing_id.to_owned());
    }
    previous
}

fn change_policy(requested: &Policy, target: Option<&Record>) -> Policy {
    target
        .map(|record| record.policy.clone())
        .unwrap_or_else(|| requested.clone())
}

fn restore_transaction(
    dir: &Path,
    state: &mut Catalog,
    previous_recovery: Option<String>,
    sources: &[(String, PathBuf)],
    clear: &[Spec],
    incoming: Option<&Stage>,
    rollback: &Stage,
) -> Result<()> {
    if let Err(original) = apply(sources, clear, incoming) {
        if apply(sources, clear, Some(rollback)).is_ok() {
            let recovering = previous_recovery.is_some();
            state.recovery = previous_recovery;
            persist(dir, state)?;
            if recovering {
                bail!("恢复未完成，已撤销本轮修改；原恢复检查点保留，请再次恢复：{original}")
            }
            bail!("切换失败，已恢复原身份：{original}")
        }
        bail!("切换和回滚未完成；已保留原恢复检查点，请使用恢复操作")
    }
    Ok(())
}
fn home() -> Result<PathBuf> {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("无法定位当前用户目录"))
}
fn roots(user: &Path) -> Vec<(String, PathBuf)> {
    let mut result = vec![("home".into(), user.to_owned())];
    let roaming = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| user.join("AppData/Roaming"));
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| user.join("AppData/Local"));
    for (index, path) in [
        roaming.join("Claude"),
        roaming.join("Claude-3p"),
        local.join("Claude"),
        local.join("Claude-3p"),
        local.join("Packages/Claude_pzs8sxrjxfjjc/LocalCache/Roaming/Claude"),
        local.join("Packages/Claude_pzs8sxrjxfjjc/LocalCache/Local/Claude"),
    ]
    .into_iter()
    .enumerate()
    {
        result.push((format!("desktop{index}"), path));
    }
    result
}
fn spec(root: &str, path: &str, keys: &[&str]) -> Spec {
    Spec {
        root: root.into(),
        path: path.into(),
        keys: keys.iter().map(|s| s.to_string()).collect(),
    }
}
fn specs(policy: &Policy) -> Vec<Spec> {
    let mut result = vec![
        spec("home", ".claude.json", super::identity_keys::ACCOUNT_KEYS),
        spec(
            "home",
            ".claude/.credentials.json",
            &["claudeAiOauth", "trustedDeviceToken"],
        ),
    ];
    for name in [".user-id", ".machine-id", "backups"] {
        result.push(spec("home", &format!(".claude/{name}"), &[]));
    }
    for name in ["settings.json", "settings.local.json"] {
        result.push(spec(
            "home",
            &format!(".claude/{name}"),
            if policy.settings {
                &[]
            } else {
                &[
                    "env.CLAUDE_CODE_OAUTH_TOKEN",
                    "env.ANTHROPIC_API_KEY",
                    "env.ANTHROPIC_AUTH_TOKEN",
                    "apiKeyHelper",
                ]
            },
        ));
    }
    for (enabled, names) in [
        (
            policy.history,
            vec![
                "history.jsonl",
                "projects",
                "sessions",
                "session-env",
                "file-history",
            ],
        ),
        (
            policy.cache,
            vec![
                "cache",
                "debug",
                "telemetry",
                "statsig",
                "stats-cache.json",
                "paste-cache",
            ],
        ),
        (policy.skills, vec!["skills", "commands", "agents", "hooks"]),
        (policy.plugins, vec!["plugins"]),
        (policy.mcp, vec![".mcp.json"]),
    ] {
        if enabled {
            for name in names {
                result.push(spec("home", &format!(".claude/{name}"), &[]));
            }
        }
    }
    if policy.mcp {
        result[0].keys.push("mcpServers".into());
        result[1].keys.extend(
            [
                "mcpOAuth",
                "mcpOAuthClientConfig",
                "mcpXaaIdp",
                "mcpXaaIdpConfig",
            ]
            .map(String::from),
        );
    }
    if policy.plugins {
        result[1].keys.push("pluginSecrets".into());
    }
    if policy.projects {
        result[0]
            .keys
            .extend(["projects", "githubRepoPaths"].map(String::from));
    }
    for index in 0..6 {
        for name in [
            "ant-did",
            "ant-device-registry.json",
            ".machine-id",
            "machine-id",
        ] {
            result.push(spec(&format!("desktop{index}"), name, &[]));
        }
        if policy.mcp {
            result.push(spec(
                &format!("desktop{index}"),
                "claude_desktop_config.json",
                &["mcpServers"],
            ));
        }
        for name in [
            "Local Storage",
            "Session Storage",
            "IndexedDB",
            "Network",
            "Cookies",
            "Cookies-journal",
            "Service Worker",
            "WebStorage",
            "Sessions",
            "Storage",
            "SharedStorage",
            "DIPS",
            "Trust Tokens",
            "TransportSecurity",
            "blob_storage",
        ] {
            result.push(spec(&format!("desktop{index}"), name, &[]));
        }
    }
    result
}
fn snapshot_specs(roots: &[(String, PathBuf)], policy: &Policy) -> Result<Vec<Spec>> {
    let mut result = specs(policy);
    let user = &roots
        .iter()
        .find(|(id, _)| id == "home")
        .ok_or_else(|| anyhow!("缺少用户目录"))?
        .1;
    if user.try_exists()? {
        for entry in fs::read_dir(user)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(".claude.json.backup") && !entry.file_type()?.is_dir() {
                result.push(spec("home", &name, &[]));
            }
        }
    }
    Ok(result)
}
fn resolved(roots: &[(String, PathBuf)], spec: &Spec) -> Result<PathBuf> {
    if !Path::new(&spec.path)
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        bail!("身份路径无效")
    }
    let base = &roots
        .iter()
        .find(|(id, _)| id == &spec.root)
        .ok_or_else(|| anyhow!("未知身份目录"))?
        .1;
    let path = base.join(&spec.path);
    files::safe(&path)?;
    Ok(path)
}
fn json_file(path: &Path) -> Result<Value> {
    files::safe(path)?;
    match fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e.into()),
        Ok(meta) => {
            if meta.len() > 16 * 1024 * 1024 || !meta.is_file() {
                bail!("Claude 配置过大或类型无效，未修改")
            }
            let value: Value =
                serde_json::from_slice(&fs::read(path)?).context("Claude 配置损坏，未修改")?;
            if !value.is_object() {
                bail!("Claude 配置必须是 JSON 对象")
            };
            Ok(value)
        }
    }
}
fn narrow(value: &Value, keys: &[String]) -> Value {
    let mut out = json!({});
    for key in keys {
        let pointer = format!("/{}", key.replace('.', "/"));
        if let Some(v) = value.pointer(&pointer) {
            out.as_object_mut().unwrap().insert(key.clone(), v.clone());
        }
    }
    out
}
fn merge(mut value: Value, keys: &[String], saved: &Value) -> Value {
    for key in keys {
        let parts: Vec<_> = key.split('.').collect();
        if parts.len() == 2 {
            if let Some(parent) = value.get_mut(parts[0]).and_then(Value::as_object_mut) {
                parent.remove(parts[1]);
            }
            if let Some(v) = saved.get(key) {
                if !value.get(parts[0]).is_some_and(Value::is_object) {
                    value[parts[0]] = json!({})
                }
                value[parts[0]][parts[1]] = v.clone();
            }
        } else {
            value.as_object_mut().unwrap().remove(key);
            if let Some(v) = saved.get(key) {
                value[key] = v.clone();
            }
        }
    }
    value
}
fn walk(path: &Path, relative: &Path, files_out: &mut Vec<(PathBuf, PathBuf)>) -> Result<()> {
    files::safe(path)?;
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            walk(&entry.path(), &relative.join(entry.file_name()), files_out)?;
        }
    } else if meta.is_file() {
        files_out.push((path.into(), relative.into()));
    } else {
        bail!("身份目录包含特殊文件")
    }
    Ok(())
}
fn append_tree<W: std::io::Write>(
    tar: &mut tar::Builder<W>,
    path: &Path,
    name: &Path,
) -> Result<()> {
    files::safe(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Directory);
        header.set_size(0);
        header.set_mode(0o700);
        header.set_cksum();
        tar.append_data(&mut header, name, std::io::empty())?;
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            append_tree(tar, &entry.path(), &name.join(entry.file_name()))?;
        }
    } else if metadata.is_file() {
        use std::os::windows::fs::OpenOptionsExt;
        // 读取期间禁止其他进程写入或删除正在归档的文件。
        let mut file = fs::OpenOptions::new().read(true).share_mode(1).open(path)?;
        tar.append_file(name, &mut file)?;
    } else {
        bail!("身份资料包含特殊文件")
    }
    Ok(())
}
fn workspace_stamp(roots: &[(String, PathBuf)], definitions: &[Spec]) -> Result<Vec<u8>> {
    use std::io::Read;
    fn add(context: &mut ring::digest::Context, path: &Path) -> Result<()> {
        files::safe(path)?;
        context.update(path.to_string_lossy().as_bytes());
        if !path.try_exists()? {
            context.update(b"missing");
            return Ok(());
        }
        if path.is_dir() {
            context.update(b"directory");
            let mut entries = fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                add(context, &entry.path())?;
            }
        } else {
            let mut file = fs::File::open(path)?;
            let mut buffer = [0u8; 65536];
            loop {
                let n = file.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                context.update(&buffer[..n]);
            }
        }
        Ok(())
    }
    let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
    for spec in definitions {
        add(&mut digest, &resolved(roots, spec)?)?
    }
    Ok(digest.finish().as_ref().to_vec())
}
fn master(dir: &Path) -> Result<Zeroizing<Vec<u8>>> {
    let path = dir.join("key.bin");
    files::safe(&path)?;
    if !path.exists() {
        if catalog(dir)?.records.len() > 0
            || fs::read_dir(dir)?.any(|e| {
                e.ok()
                    .is_some_and(|e| e.path().extension().is_some_and(|x| x == "baga"))
            })
        {
            bail!("身份主密钥缺失，拒绝覆盖已有归档")
        }
        crate::config::atomic_write(&path, &codec::random::<32>().map_err(|e| anyhow!(e))?)?;
    }
    let key = Zeroizing::new(fs::read(&path)?);
    if key.len() != 32 {
        bail!("身份主密钥无效")
    };
    Ok(key)
}
fn account(roots: &[(String, PathBuf)]) -> Result<Option<String>> {
    let value = json_file(&roots[0].1.join(".claude.json"))?;
    Ok(value
        .pointer("/oauthAccount/emailAddress")
        .and_then(Value::as_str)
        .map(|v| v.chars().filter(|c| !c.is_control()).take(254).collect()))
}
fn save(
    dir: &Path,
    roots: &[(String, PathBuf)],
    policy: &Policy,
    name: &str,
    slot: &str,
) -> Result<Record> {
    let id = uuid::Uuid::new_v4().to_string();
    let archive = dir.join(format!("{id}.baga"));
    let definitions = snapshot_specs(roots, policy)?;
    let kinds = definitions
        .iter()
        .map(|spec| -> Result<Option<bool>> {
            let path = resolved(roots, spec)?;
            Ok(if path.try_exists()? {
                Some(path.is_dir())
            } else {
                None
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let manifest = Manifest {
        version: 1,
        specs: definitions,
        kinds,
    };
    let key = master(dir)?;
    let sealed = codec::Seal::new(
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&archive)?,
        &key,
    )?;
    let mut compressed = zstd::stream::write::Encoder::new(sealed, 3)?;
    compressed.include_checksum(true)?;
    let mut tar = tar::Builder::new(compressed);
    let bytes = serde_json::to_vec(&manifest)?;
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o600);
    header.set_cksum();
    tar.append_data(&mut header, "manifest.json", bytes.as_slice())?;
    for (index, spec) in manifest.specs.iter().enumerate() {
        let path = resolved(roots, spec)?;
        if !path.try_exists()? {
            continue;
        }
        if !spec.keys.is_empty() {
            let bytes =
                Zeroizing::new(serde_json::to_vec(&narrow(&json_file(&path)?, &spec.keys))?);
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o600);
            header.set_cksum();
            tar.append_data(
                &mut header,
                format!("data/{index}/value.json"),
                bytes.as_slice(),
            )?;
        } else {
            let archive_path = PathBuf::from(format!("data/{index}"));
            append_tree(
                &mut tar,
                &path,
                &if path.is_file() {
                    archive_path.join("file")
                } else {
                    archive_path
                },
            )?;
        }
    }
    let file = tar.into_inner()?.finish()?.finish()?;
    file.sync_all()?;
    drop(file);
    let _verified = load(dir, &id)?;
    Ok(Record {
        id,
        name: name
            .trim()
            .chars()
            .filter(|c| !c.is_control())
            .take(80)
            .collect(),
        email: account(roots)?,
        created_at: chrono::Utc::now().to_rfc3339(),
        bytes: fs::metadata(archive)?.len(),
        policy: policy.clone(),
        browser_slot: slot.into(),
    })
}
fn load(dir: &Path, id: &str) -> Result<Stage> {
    if !valid_id(id) {
        bail!("身份编号无效")
    }
    let source = dir.join(format!("{id}.baga"));
    files::safe(&source)?;
    let path = dir.join(format!("stage-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&path)?;
    let mut stage = Stage {
        path,
        manifest: Manifest {
            version: 0,
            specs: vec![],
            kinds: vec![],
        },
    };
    let opened = codec::Open::new(fs::File::open(source)?, &master(dir)?)?;
    let mut decoder = zstd::stream::read::Decoder::new(opened)?;
    decoder.window_log_max(27)?;
    let mut archive = tar::Archive::new(decoder);
    for entry in archive.entries()? {
        let mut entry = entry?;
        let relative = entry.path()?.into_owned();
        if !relative
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
            || !matches!(
                entry.header().entry_type(),
                tar::EntryType::Regular | tar::EntryType::Directory
            )
        {
            bail!("身份归档含非法成员")
        }
        if !entry.unpack_in(&stage.path)? {
            bail!("身份归档路径越界")
        }
    }
    std::io::copy(&mut archive.into_inner(), &mut std::io::sink())?;
    stage.manifest = serde_json::from_slice(&fs::read(stage.path.join("manifest.json"))?)?;
    let all = specs(&Policy {
        history: true,
        cache: true,
        settings: true,
        skills: true,
        plugins: true,
        mcp: true,
        projects: true,
    });
    if stage.manifest.version != 1
        || stage.manifest.kinds.len() != stage.manifest.specs.len()
        || stage.manifest.specs.iter().any(|s| {
            if s.root == "home"
                && s.path.starts_with(".claude.json.backup")
                && Path::new(&s.path).components().count() == 1
                && s.keys.is_empty()
            {
                return false;
            }
            !all.iter().any(|a| {
                a.root == s.root
                    && a.path == s.path
                    && (a.keys.is_empty()
                        || (!s.keys.is_empty() && s.keys.iter().all(|k| a.keys.contains(k))))
            })
        })
    {
        bail!("归档来源不在白名单")
    }
    Ok(stage)
}
fn erase(path: &Path) -> Result<()> {
    files::safe(path)?;
    if !path.try_exists()? {
        return Ok(());
    }
    let mut checked = vec![];
    walk(path, Path::new(""), &mut checked)?;
    if path.is_dir() {
        fs::remove_dir_all(path)?
    } else {
        fs::remove_file(path)?
    }
    Ok(())
}
fn apply(roots: &[(String, PathBuf)], clear: &[Spec], target: Option<&Stage>) -> Result<()> {
    for spec in clear {
        let path = resolved(roots, spec)?;
        if spec.keys.is_empty() {
            erase(&path)?
        } else if path.try_exists()? {
            let next = merge(json_file(&path)?, &spec.keys, &json!({}));
            crate::config::atomic_write(&path, &serde_json::to_vec_pretty(&next)?)?;
        }
    }
    if let Some(stage) = target {
        for (index, spec) in stage.manifest.specs.iter().enumerate() {
            let source = stage.path.join(format!("data/{index}"));
            if !source.exists() {
                continue;
            }
            let destination = resolved(roots, spec)?;
            if !spec.keys.is_empty() {
                let saved: Value = serde_json::from_slice(&fs::read(source.join("value.json"))?)?;
                let next = merge(json_file(&destination)?, &spec.keys, &saved);
                crate::config::atomic_write(&destination, &serde_json::to_vec_pretty(&next)?)?;
            } else if stage.manifest.kinds[index] == Some(true) {
                fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
                    files::safe(source)?;
                    files::safe(destination)?;
                    if source.is_dir() {
                        fs::create_dir_all(destination)?;
                        for entry in fs::read_dir(source)? {
                            let entry = entry?;
                            copy_tree(&entry.path(), &destination.join(entry.file_name()))?
                        }
                    } else {
                        fs::copy(source, destination)?;
                    }
                    Ok(())
                }
                copy_tree(&source, &destination)?;
            } else {
                fs::create_dir_all(destination.parent().unwrap())?;
                fs::copy(source.join("file"), destination)?;
            }
        }
    }
    Ok(())
}
pub(crate) fn ensure_stopped() -> Result<()> {
    files::shell(r#"$ErrorActionPreference='Stop'; $p=@(Get-CimInstance Win32_Process | Where-Object { $_.Name -match '^(claude|claude-desktop)\.exe$' -or ($_.Name -ieq 'node.exe' -and $_.CommandLine -match '@anthropic-ai[\\/]claude-code|claude-code[\\/]cli\.js') }); if($p.Count){exit 9}"#,&[]).context("请先保存工作并退出 Claude Code 与 Claude Desktop；软件不会强制结束进程")?;
    Ok(())
}
fn check_custom(user: &Path) -> Result<()> {
    if std::env::var_os("CLAUDE_CONFIG_DIR")
        .is_some_and(|v| PathBuf::from(v) != user.join(".claude"))
    {
        bail!("检测到自定义 CLAUDE_CONFIG_DIR，本次未操作；当前仅管理默认 Claude 目录")
    };
    Ok(())
}
#[tauri::command]
pub async fn identity_status(app: tauri::AppHandle) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move||->Result<Value>{let dir=root(&app)?.join("vault");let state=catalog(&dir)?;let user=home()?;let current=account(&roots(&user))?;Ok(json!({"catalog":state,"currentEmail":current,"customDirectory":check_custom(&user).is_err()}))}).await.map_err(error)?.map_err(error)
}
#[tauri::command]
pub async fn identity_action(
    app: tauri::AppHandle,
    action: String,
    id: Option<String>,
    name: String,
    policy: Policy,
    confirmed: bool,
) -> Result<Value, String> {
    if !confirmed {
        return Err("请确认操作范围后重试".into());
    }
    let lock_app = app.clone();
    let operation = lock_app.state::<Operation>();
    let _operation = operation.lock().await;
    let network_operation = lock_app.state::<crate::platform::state::ConfigUpdateMutex>();
    let _network_operation = network_operation.lock().await;
    if matches!(action.as_str(), "switch" | "empty" | "restore" | "recover") {
        super::ExecutionHooks::pause_claude(&app)?;
    }
    tauri::async_runtime::spawn_blocking(move || -> Result<Value> {
        let dir = root(&app)?.join("vault");
        files::private_dir(&dir)?;
        let user = home()?;
        check_custom(&user)?;
        let sources = roots(&user);
        let mut state = catalog(&dir)?;
        if action == "delete" {
            let id = id.ok_or_else(|| anyhow!("缺少身份编号"))?;
            if state.active.as_deref() == Some(&id) || state.recovery.as_deref() == Some(&id) {
                bail!("不能删除当前或恢复所需的身份")
            };
            let record = state
                .records
                .iter()
                .position(|r| r.id == id)
                .ok_or_else(|| anyhow!("身份不存在"))?;
            let path = dir.join(format!("{id}.baga"));
            let trash = dir.join(format!("trash-{id}.baga"));
            files::safe(&path)?;
            files::safe(&trash)?;
            fs::rename(&path, &trash)?;
            let removed = state.records.remove(record);
            if let Err(e) = persist(&dir, &state) {
                state.records.insert(record, removed);
                let _ = fs::rename(&trash, &path);
                return Err(e);
            }
            fs::remove_file(trash)?;
            return Ok(json!({"message":"已删除所选备份；浏览器资料保留"}));
        }
        ensure_stopped()?;
        super::ExecutionHooks::ensure_environment_closed(&app)?;
        if state.recovery.is_some() && action != "recover" {
            bail!("请先恢复上次未完成的切换")
        }
        if action == "archive" {
            let record = save(&dir, &sources, &policy, &name, &state.browser_slot)?;
            state.records.push(record);
            persist(&dir, &state)?;
            return Ok(json!({"message":"已加密备份当前本地身份，当前登录保持不变"}));
        }
        if !matches!(action.as_str(), "switch" | "empty" | "recover") {
            bail!("未知身份操作")
        }
        let target_id = if action == "recover" {
            Some(
                state
                    .recovery
                    .clone()
                    .ok_or_else(|| anyhow!("没有需要恢复的切换"))?,
            )
        } else if action == "switch" {
            Some(id.ok_or_else(|| anyhow!("缺少目标身份编号"))?)
        } else {
            None
        };
        let target_record = target_id
            .as_ref()
            .map(|id| {
                state
                    .records
                    .iter()
                    .find(|r| &r.id == id)
                    .cloned()
                    .ok_or_else(|| anyhow!("身份不存在"))
            })
            .transpose()?;
        let incoming = target_record
            .as_ref()
            .map(|r| load(&dir, &r.id))
            .transpose()?;
        // 已保存身份只修改其自身范围，页面上额外勾选的类别不能扩大清理范围。
        let capture = change_policy(&policy, target_record.as_ref());
        let expected = workspace_stamp(&sources, &snapshot_specs(&sources, &capture)?)?;
        let outgoing = save(
            &dir,
            &sources,
            &capture,
            if name.trim().is_empty() {
                "切换前自动备份"
            } else {
                &name
            },
            &state.browser_slot,
        )?;
        if workspace_stamp(&sources, &snapshot_specs(&sources, &capture)?)? != expected {
            state.records.push(outgoing);
            persist(&dir, &state)?;
            bail!("归档期间 Claude 资料发生变化，已保留备份但未切换；请退出相关程序后重试")
        }
        let outgoing_id = outgoing.id.clone();
        state.records.push(outgoing);
        let previous_recovery = begin_checkpoint(&mut state, &outgoing_id);
        persist(&dir, &state)?;
        let rollback = load(&dir, &outgoing_id)?;
        let mut clear = rollback.manifest.specs.clone();
        if let Some(stage) = &incoming {
            for spec in &stage.manifest.specs {
                if !clear.contains(spec) {
                    clear.push(spec.clone())
                }
            }
        }
        ensure_stopped()?;
        if workspace_stamp(&sources, &snapshot_specs(&sources, &capture)?)? != expected {
            state.recovery = previous_recovery;
            persist(&dir, &state)?;
            bail!("切换前资料发生变化，未修改工作区，请重试")
        }
        super::ExecutionHooks::invalidate_environment(&app);
        restore_transaction(
            &dir,
            &mut state,
            previous_recovery,
            &sources,
            &clear,
            incoming.as_ref(),
            &rollback,
        )?;
        state.active = target_record.as_ref().map(|r| r.id.clone());
        state.browser_slot = target_record
            .map(|r| r.browser_slot)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        state.recovery = None;
        persist(&dir, &state)?;
        Ok(json!({"message":"本地身份与浏览器资料选择已更新；网页登录需在对应浏览器资料中完成"}))
    })
    .await
    .map_err(error)?
    .map_err(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_recovery_failure_keeps_original_checkpoint_even_after_rollback() {
        for damage_rollback in [false, true] {
            let fixture = crate::test_support::FixtureDir::new();
            let dir = fixture.path().join("vault");
            fs::create_dir(&dir).unwrap();
            let sources = fixture_roots(fixture.path());
            fs::create_dir_all(&sources[0].1).unwrap();
            let path = sources[0].1.join(".claude.json");
            fs::write(
                &path,
                br#"{"oauthAccount":{"emailAddress":"original@example.invalid"}}"#,
            )
            .unwrap();
            let original = save(&dir, &sources, &Policy::default(), "original", "default").unwrap();
            let original_id = original.id.clone();
            let mut state = Catalog {
                records: vec![original],
                recovery: Some(original_id.clone()),
                ..Default::default()
            };
            for _ in 0..2 {
                fs::write(
                    &path,
                    br#"{"oauthAccount":{"emailAddress":"partial@example.invalid"}}"#,
                )
                .unwrap();
                let outgoing =
                    save(&dir, &sources, &Policy::default(), "partial", "default").unwrap();
                let rollback = load(&dir, &outgoing.id).unwrap();
                let incoming = load(&dir, &original_id).unwrap();
                fs::write(incoming.path.join("data/0/value.json"), b"invalid").unwrap();
                if damage_rollback {
                    fs::write(rollback.path.join("data/0/value.json"), b"invalid").unwrap();
                }
                let previous = begin_checkpoint(&mut state, &outgoing.id);
                state.records.push(outgoing);
                persist(&dir, &state).unwrap();
                assert_eq!(
                    catalog(&dir).unwrap().recovery.as_deref(),
                    Some(original_id.as_str())
                );
                assert!(
                    restore_transaction(
                        &dir,
                        &mut state,
                        previous,
                        &sources,
                        &rollback.manifest.specs,
                        Some(&incoming),
                        &rollback
                    )
                    .is_err()
                );
                assert_eq!(
                    catalog(&dir).unwrap().recovery.as_deref(),
                    Some(original_id.as_str())
                );
                assert!(require_ready(&dir).is_err());
            }
        }
    }
    #[test]
    fn saved_identity_ignores_extra_checkboxes_and_preserves_shared_files() {
        let fixture = crate::test_support::FixtureDir::new();
        let dir = fixture.path().join("vault");
        fs::create_dir(&dir).unwrap();
        let sources = fixture_roots(fixture.path());
        let user = &sources[0].1;
        fs::create_dir_all(user.join(".claude/projects")).unwrap();
        let target = save(&dir, &sources, &Policy::default(), "target", "default").unwrap();
        fs::write(user.join(".claude/projects/history.jsonl"), b"keep-history").unwrap();
        fs::write(
            user.join(".claude/settings.json"),
            br#"{"theme":"keep","env":{"HTTP_PROXY":"keep"}}"#,
        )
        .unwrap();
        let requested = Policy {
            history: true,
            settings: true,
            cache: true,
            mcp: true,
            ..Default::default()
        };
        let effective = change_policy(&requested, Some(&target));
        let outgoing = save(&dir, &sources, &effective, "before", "default").unwrap();
        let rollback = load(&dir, &outgoing.id).unwrap();
        let incoming = load(&dir, &target.id).unwrap();
        apply(&sources, &rollback.manifest.specs, Some(&incoming)).unwrap();
        assert_eq!(
            fs::read(user.join(".claude/projects/history.jsonl")).unwrap(),
            b"keep-history"
        );
        assert_eq!(
            json_file(&user.join(".claude/settings.json")).unwrap()["theme"],
            "keep"
        );
        assert!(!effective.history && !effective.settings && !effective.cache && !effective.mcp);
        assert!(change_policy(&requested, None).history);
    }
    #[test]
    fn pending_recovery_blocks_launch_until_catalog_is_recovered() {
        let fixture = crate::test_support::FixtureDir::new();
        let mut state = Catalog::default();
        state.recovery = Some(uuid::Uuid::new_v4().to_string());
        persist(fixture.path(), &state).unwrap();
        assert!(require_ready(fixture.path()).is_err());
        state.recovery = None;
        persist(fixture.path(), &state).unwrap();
        assert!(require_ready(fixture.path()).is_ok());
    }
    fn fixture_roots(root: &Path) -> Vec<(String, PathBuf)> {
        let mut roots = vec![("home".into(), root.join("user"))];
        for i in 0..6 {
            roots.push((format!("desktop{i}"), root.join(format!("desktop{i}"))))
        }
        roots
    }
    #[test]
    fn history_is_opt_in_and_roundtrip_preserves_empty_directories_and_project_source() {
        let fixture = crate::test_support::FixtureDir::new();
        let dir = fixture.path().join("vault");
        fs::create_dir(&dir).unwrap();
        let roots = fixture_roots(fixture.path());
        let user = &roots[0].1;
        fs::create_dir_all(user.join(".claude/projects/example/empty")).unwrap();
        fs::write(
            user.join(".claude/projects/example/directory"),
            b"conversation",
        )
        .unwrap();
        fs::create_dir_all(user.join("actual-project")).unwrap();
        fs::write(user.join("actual-project/main.rs"), b"source").unwrap();
        let shared = save(&dir, &roots, &Policy::default(), "shared", "default").unwrap();
        let shared = load(&dir, &shared.id).unwrap();
        apply(&roots, &shared.manifest.specs, None).unwrap();
        assert!(user.join(".claude/projects/example/directory").exists());
        let policy = Policy {
            history: true,
            ..Default::default()
        };
        let record = save(&dir, &roots, &policy, "with history", "default").unwrap();
        let stage = load(&dir, &record.id).unwrap();
        apply(&roots, &stage.manifest.specs, None).unwrap();
        assert!(!user.join(".claude/projects").exists());
        assert_eq!(
            fs::read(user.join("actual-project/main.rs")).unwrap(),
            b"source"
        );
        apply(&roots, &stage.manifest.specs, Some(&stage)).unwrap();
        assert!(user.join(".claude/projects/example/empty").is_dir());
        assert_eq!(
            fs::read(user.join(".claude/projects/example/directory")).unwrap(),
            b"conversation"
        );
    }
    #[test]
    fn damaged_archive_and_missing_key_do_not_modify_the_workspace() {
        let fixture = crate::test_support::FixtureDir::new();
        let dir = fixture.path().join("vault");
        fs::create_dir(&dir).unwrap();
        let roots = fixture_roots(fixture.path());
        fs::create_dir_all(&roots[0].1).unwrap();
        let source = roots[0].1.join(".claude.json");
        let current = br#"{"oauthAccount":{"emailAddress":"fixture@example.invalid"}}"#;
        fs::write(&source, current).unwrap();
        let record = save(&dir, &roots, &Policy::default(), "fixture", "default").unwrap();
        let archive = dir.join(format!("{}.baga", record.id));
        let mut data = fs::read(&archive).unwrap();
        data[80] ^= 1;
        fs::write(&archive, &data).unwrap();
        assert!(load(&dir, &record.id).is_err());
        assert_eq!(fs::read(&source).unwrap(), current);
        fs::remove_file(dir.join("key.bin")).unwrap();
        assert!(master(&dir).is_err());
        assert_eq!(fs::read(&archive).unwrap(), data);
    }
    #[test]
    fn failed_restore_can_restore_the_preceding_checkpoint() {
        let fixture = crate::test_support::FixtureDir::new();
        let dir = fixture.path().join("vault");
        fs::create_dir(&dir).unwrap();
        let roots = fixture_roots(fixture.path());
        fs::create_dir_all(&roots[0].1).unwrap();
        let source = roots[0].1.join(".claude.json");
        fs::write(&source,br#"{"oauthAccount":{"emailAddress":"fixture@example.invalid"},"mcpServers":{"keep":{}}}"#).unwrap();
        let record = save(&dir, &roots, &Policy::default(), "rollback", "default").unwrap();
        let rollback = load(&dir, &record.id).unwrap();
        let damaged = load(&dir, &record.id).unwrap();
        fs::write(damaged.path.join("data/0/value.json"), b"invalid").unwrap();
        assert!(apply(&roots, &rollback.manifest.specs, Some(&damaged)).is_err());
        apply(&roots, &rollback.manifest.specs, Some(&rollback)).unwrap();
        let restored = json_file(&source).unwrap();
        assert_eq!(
            restored["oauthAccount"]["emailAddress"],
            "fixture@example.invalid"
        );
        assert!(restored["mcpServers"]["keep"].is_object());
    }
    #[test]
    fn login_merge_retains_mcp_and_other_environment_keys() {
        let input = json!({"env":{"ANTHROPIC_API_KEY":"old","HTTP_PROXY":"keep"},"theme":"dark"});
        let keys = vec!["env.ANTHROPIC_API_KEY".into()];
        let out = merge(input, &keys, &json!({"env.ANTHROPIC_API_KEY":"new"}));
        assert_eq!(out["env"]["HTTP_PROXY"], "keep");
        assert_eq!(out["theme"], "dark");
        assert_eq!(out["env"]["ANTHROPIC_API_KEY"], "new");
    }
    #[test]
    fn encrypted_roundtrip_restores_login_and_preserves_shared_settings() {
        let fixture = crate::test_support::FixtureDir::new();
        let dir = fixture.path().join("vault");
        let user = fixture.path().join("user");
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(user.join(".claude")).unwrap();
        fs::write(user.join(".claude.json"),br#"{"oauthAccount":{"emailAddress":"fixture@example.invalid"},"mcpServers":{"shared":{}},"theme":"dark"}"#).unwrap();
        fs::write(
            user.join(".claude/.credentials.json"),
            br#"{"claudeAiOauth":{"accessToken":"fixture-secret"},"mcpOAuth":{"shared":"keep"}}"#,
        )
        .unwrap();
        let roots = vec![("home".into(), user.clone())];
        let policy = Policy::default();
        let mut defs = specs(&policy);
        defs.retain(|s| s.root == "home");
        // 测试中补充不存在的隔离 Desktop 根，避免读取真实 AppData。
        let mut roots = roots;
        for i in 0..6 {
            roots.push((
                format!("desktop{i}"),
                fixture.path().join(format!("desktop{i}")),
            ))
        }
        let record = save(&dir, &roots, &policy, "fixture", "default").unwrap();
        let bytes = fs::read(dir.join(format!("{}.baga", record.id))).unwrap();
        assert!(!bytes.windows(14).any(|b| b == b"fixture-secret"));
        let stage = load(&dir, &record.id).unwrap();
        apply(&roots, &stage.manifest.specs, None).unwrap();
        assert!(
            json_file(&user.join(".claude.json"))
                .unwrap()
                .get("oauthAccount")
                .is_none()
        );
        apply(&roots, &stage.manifest.specs, Some(&stage)).unwrap();
        let credentials = json_file(&user.join(".claude/.credentials.json")).unwrap();
        assert_eq!(credentials["mcpOAuth"]["shared"], "keep");
        assert_eq!(
            credentials["claudeAiOauth"]["accessToken"],
            "fixture-secret"
        );
    }
}
