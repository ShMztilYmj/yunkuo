//! 本机能力库：与具体模型服务无关，配置按条目合并，只有本库管理的副本可以移除。
mod adapters;
mod skills;
#[cfg(test)]
mod tests;
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
};
use tauri::Manager;

const LIMIT: usize = 32 * 1024 * 1024;
pub(super) type Tree = BTreeMap<String, String>;
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Receipt {
    path: String,
    hash: String,
}
/// 条目来源。旧版本的库没有这个字段，读出来是 `None`，行为与以前一致。
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Source {
    /// `folder`：本地文件夹；`tool`：某个工具的现有配置。
    kind: String,
    /// folder 为文件夹路径，tool 为工具 id。
    #[serde(default)]
    path: String,
    /// tool 来源里在原工具中的条目名。
    #[serde(default)]
    name: String,
    /// 导入或上次更新时的内容指纹，用来判断上游是否变了。
    #[serde(default)]
    hash: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Entry {
    id: String,
    /// 内部编号，也是写入工具配置时使用的键；展示名称单独保存在 `title`。
    name: String,
    kind: String,
    #[serde(default)]
    spec: Value,
    #[serde(default)]
    files: Tree,
    #[serde(default)]
    source: Option<Source>,
    #[serde(default)]
    enabled: BTreeMap<String, bool>,
    #[serde(default)]
    applied: BTreeMap<String, Receipt>,
    #[serde(default)]
    errors: BTreeMap<String, String>,
    /// 以下是能力库自己的展示信息，不参与项目投影和同步。
    #[serde(default)]
    title: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    detail: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    repository: String,
}
/// 同步被拒绝是因为目标里已有同名内容（而不是配置读不了、字段不支持等）。
/// 单独成类型，方便把「冲突」和「失败」分开告诉界面。
#[derive(Debug)]
pub(super) struct Conflict(pub String);
impl std::fmt::Display for Conflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Conflict {}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Library {
    entries: Vec<Entry>,
}
#[derive(Clone)]
struct Target {
    id: String,
    name: String,
    /// 写入适配器用哪套投影：claude / codex / cursor / kimi / opencode / antigravity / grok / glm / deepseek。
    tool: String,
    config: PathBuf,
    skills: PathBuf,
    /// 该工具有没有 Skills 目录；Cursor 这类只有 MCP 的目标为 false。
    skills_supported: bool,
}
fn user_home() -> Result<PathBuf> {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .context("无法定位用户目录")
}
/// 名单里的存储键折成同步目标 ID：`chatgpt` 的运行时是 `codex`。没接入能力同步的工具返回 `None`。
fn target_id_for(key: &str) -> Option<String> {
    crate::ai::agent_roster::capability(key)?;
    Some(
        crate::ai::agent_roster::runtime_for(key)
            .unwrap_or(key)
            .to_string(),
    )
}
/// 已添加的 Agent 全表：前端据此把「还没接入 MCP / Skills 同步」的工具也列出来，
/// 而不是让它们从列表里凭空消失。
fn agent_rows(enabled: &[String]) -> Vec<Value> {
    enabled
        .iter()
        .filter_map(|key| {
            let agent = crate::ai::agent_roster::get(key)?;
            Some(json!({
                "id": agent.key,
                "name": agent.name,
                "target": target_id_for(key),
            }))
        })
        .collect()
}
/// 解析名单里声明的同步位置：环境变量优先，未设置时回落用户目录。
/// `use_env = false` 只给测试用，保证路径与机器无关。
fn resolve_path(
    home: &Path,
    path: &crate::ai::agent_roster::CapabilityPath,
    use_env: bool,
) -> PathBuf {
    if use_env {
        if let Some(variable) = &path.env {
            if let Some(dir) = std::env::var_os(variable) {
                let relative = path.env_home.as_deref().unwrap_or(&path.home);
                return PathBuf::from(dir).join(relative);
            }
        }
    }
    home.join(&path.home)
}
/// 同步目标全部来自 `agents.json` 的 `capability` 段：加一款工具只改 JSON，
/// 不再写死路径。顺序按 `order` 字段，和界面阅读顺序一致。
fn targets(home: &Path, use_env: bool, enabled: &[String]) -> Result<Vec<Target>> {
    let keys: Vec<&str> = if enabled.is_empty() {
        crate::ai::agent_roster::keys()
    } else {
        enabled.iter().map(String::as_str).collect()
    };
    let mut out = Vec::new();
    for key in keys {
        let Some(agent) = crate::ai::agent_roster::get(key) else {
            continue;
        };
        let Some(capability) = &agent.capability else {
            continue;
        };
        let Some(mcp) = &capability.mcp else {
            continue;
        };
        let config = resolve_path(home, mcp, use_env);
        let skills = capability
            .skills
            .as_ref()
            .map(|skills| resolve_path(home, skills, use_env))
            .unwrap_or_else(|| config.parent().unwrap_or(home).join("skills"));
        out.push(Target {
            id: agent.runtime.clone().unwrap_or_else(|| agent.key.clone()),
            name: agent.name.clone(),
            tool: if agent.key == "chatgpt" {
                "codex".into()
            } else {
                agent.key.clone()
            },
            config,
            skills,
            skills_supported: capability.skills.is_some(),
        });
    }
    out.sort_by_key(|target| {
        (
            crate::ai::agent_roster::capability_order(&target.id),
            target.id.clone(),
        )
    });
    for target in &out {
        if !target.config.is_absolute() || !target.skills.is_absolute() {
            bail!("工具配置目录必须使用绝对路径");
        }
    }
    Ok(out)
}
use crate::platform::digest::sha256_lower as hash;
fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    crate::platform::secure_storage::safe(path)?;
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut bytes = vec![];
    file.take((LIMIT + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > LIMIT {
        bail!("配置或能力库超过 32 MiB，未修改原文件");
    }
    Ok(Some(bytes))
}
fn write_secret<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > LIMIT - 65536 {
        bail!("能力库内容过多，请精简后重试");
    }
    crate::platform::secure_storage::write_auth(
        path,
        &crate::platform::secure_storage::crypt(&bytes, true)?,
    )
}
fn read_secret<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    read_bytes(path)?
        .map(|bytes| {
            Ok(serde_json::from_slice(
                &crate::platform::secure_storage::crypt(&bytes, false)?,
            )?)
        })
        .transpose()
}
fn load(root: &Path) -> Result<Library> {
    Ok(read_secret(&root.join("library.coc"))?.unwrap_or_default())
}
fn save(root: &Path, library: &Library) -> Result<()> {
    write_secret(&root.join("library.coc"), library)
}
fn valid_name(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 80
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        || value.starts_with('.')
    {
        bail!("编号请使用 1–80 位字母、数字、横线或下划线");
    }
    let stem = value
        .split('.')
        .next()
        .unwrap_or(value)
        .to_ascii_uppercase();
    if [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ]
    .contains(&stem.as_str())
    {
        bail!("编号是 Windows 保留名称");
    }
    Ok(())
}
fn make_entry(name: String, kind: &str) -> Result<Entry> {
    valid_name(&name)?;
    Ok(Entry {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        kind: kind.into(),
        spec: Value::Null,
        files: Tree::new(),
        source: None,
        enabled: BTreeMap::new(),
        applied: BTreeMap::new(),
        errors: BTreeMap::new(),
        title: String::new(),
        summary: String::new(),
        detail: String::new(),
        tags: Vec::new(),
        repository: String::new(),
    })
}
fn entry_mut<'a>(library: &'a mut Library, id: &str) -> Result<&'a mut Entry> {
    library
        .entries
        .iter_mut()
        .find(|e| e.id == id)
        .context("能力条目已不存在，请刷新")
}
fn ensure_unique(library: &Library, name: &str, kind: &str, except: Option<&str>) -> Result<()> {
    if library.entries.iter().any(|e| {
        e.kind == kind && e.name.eq_ignore_ascii_case(name) && Some(e.id.as_str()) != except
    }) {
        bail!("能力库已有同名条目，请编辑已有条目或使用其他编号");
    }
    Ok(())
}
fn backup_path(root: &Path, target: &Target) -> PathBuf {
    root.join("backups").join(format!(
        "mcp-{}.coc",
        hash(target.config.to_string_lossy().as_bytes())
    ))
}
fn state(entry: &Entry, target: &Target) -> &'static str {
    // 只有 MCP 的工具（Cursor 等）在 Skills 条目上一律算不兼容，避免显示成“已同步”。
    if entry.kind == "skill" && !target.skills_supported {
        return "unsupported";
    }
    match current_hash(entry, target) {
        Ok(current) => {
            if entry.enabled.get(&target.id) == Some(&true) {
                match desired_hash(entry, target) {
                    Ok(Some(want)) if current.as_deref() == Some(want.as_str()) => "synced",
                    Ok(_) => "pending",
                    Err(_) => "unsupported",
                }
            } else if entry.applied.contains_key(&target.id) && current.is_some() {
                "pending"
            } else {
                "off"
            }
        }
        Err(_) => "error",
    }
}
fn snapshot(root: &Path, library: &Library, targets: &[Target], agents: &[Value]) -> Value {
    let rows: Vec<Value> = targets
        .iter()
        .map(|t| {
            json!({
                "id": t.id,
                "name": t.name,
                "configPath": t.config,
                "skillsPath": t.skills,
                "skillsSupported": t.skills_supported,
                "hasBackup": backup_path(root, t).exists(),
            })
        })
        .collect();
    let entries: Vec<Value> = library
        .entries
        .iter()
        .map(|e| {
            let states: BTreeMap<String, &str> = targets
                .iter()
                .map(|t| (t.id.clone(), state(e, t)))
                .collect();
            json!({
                "id": e.id,
                "name": e.name,
                "kind": e.kind,
                "transport": e.spec["type"],
                "fileCount": e.files.len(),
                "source": e.source,
                "enabled": e.enabled,
                "errors": e.errors,
                "states": states,
                "title": e.title,
                "summary": e.summary,
                "detail": e.detail,
                "tags": e.tags,
                "repository": e.repository,
            })
        })
        .collect();
    json!({"targets": rows, "agents": agents, "entries": entries})
}
fn desired_hash(entry: &Entry, target: &Target) -> Result<Option<String>> {
    if entry.enabled.get(&target.id) != Some(&true) {
        return Ok(None);
    }
    Ok(Some(if entry.kind == "mcp" {
        hash(&serde_json::to_vec(&adapters::project(
            &entry.spec,
            &target.tool,
        )?)?)
    } else {
        skills::tree_hash(&entry.files)
    }))
}
fn current_hash(entry: &Entry, target: &Target) -> Result<Option<String>> {
    if entry.kind == "mcp" {
        Ok(adapters::read_servers(target)?
            .get(&entry.name)
            .map(|v| hash(&serde_json::to_vec(v).unwrap_or_default())))
    } else {
        Ok(skills::read_tree(&target.skills.join(&entry.name))?
            .as_ref()
            .map(skills::tree_hash))
    }
}
fn sync_entry(root: &Path, entry: &mut Entry, target: &Target, force: bool) -> Result<()> {
    if entry.kind == "skill" && !target.skills_supported {
        bail!("{} 不支持 Skills 同步", target.name);
    }
    let path = if entry.kind == "mcp" {
        target.config.clone()
    } else {
        target.skills.join(&entry.name)
    };
    if let Some(receipt) = entry.applied.get(&target.id) {
        if receipt.path != path.to_string_lossy() {
            bail!("配置目录已变化，请先从当前工具导入并检查，未修改新目录");
        }
    }
    let current = current_hash(entry, target)?;
    let desired = desired_hash(entry, target)?;
    let owned = entry
        .applied
        .get(&target.id)
        .is_some_and(|r| current.as_deref() == Some(r.hash.as_str()));
    if current != desired {
        if current.is_some() && !owned && !force {
            return Err(anyhow::Error::new(Conflict(
                "同名内容已存在或被外部修改；原内容保留，请先选择如何处理冲突".into(),
            )));
        }
        if entry.kind == "mcp" {
            adapters::sync(root, entry, target, force)?;
        } else {
            skills::sync(root, entry, target, force)?;
        }
    }
    if let Some(hash) = desired {
        entry.applied.insert(
            target.id.clone(),
            Receipt {
                path: path.to_string_lossy().into(),
                hash,
            },
        );
    } else {
        entry.applied.remove(&target.id);
    }
    Ok(())
}
fn sync_selected(
    root: &Path,
    library: &mut Library,
    targets: &[Target],
    id: Option<&str>,
    target: Option<&str>,
) -> Result<Value> {
    let mut results = vec![];
    for index in 0..library.entries.len() {
        if id.is_some_and(|id| library.entries[index].id != id) {
            continue;
        }
        for t in targets {
            if target.is_some_and(|target| t.id != target) {
                continue;
            }
            let entry = &mut library.entries[index];
            if entry.enabled.get(&t.id) != Some(&true) && !entry.applied.contains_key(&t.id) {
                continue;
            }
            let id = entry.id.clone();
            let name = entry.name.clone();
            let failure = sync_entry(root, entry, t, false).err();
            let conflict = failure
                .as_ref()
                .is_some_and(|e| e.downcast_ref::<Conflict>().is_some());
            let error = failure.map(|e| format!("{e:#}"));
            if let Some(error) = &error {
                entry.errors.insert(t.id.clone(), error.clone());
            } else {
                entry.errors.remove(&t.id);
            }
            results.push(json!({
                "entryId": id,
                "targetId": t.id,
                "name": name,
                "target": t.name,
                "ok": error.is_none(),
                "conflict": conflict,
                "error": error,
            }));
            save(root, library)?;
        }
    }
    Ok(json!({"results":results}))
}
fn flatten(value: &Value, prefix: &str, out: &mut BTreeMap<String, String>) {
    if value.is_null() {
        return;
    }
    match value {
        Value::Object(map) if map.is_empty() => {
            out.insert(prefix.into(), "{}".into());
        }
        Value::Object(map) => {
            for (key, value) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(value, &path, out);
            }
        }
        other => {
            out.insert(prefix.into(), other.to_string());
        }
    }
}
fn field_diff(before: &Value, after: &Value) -> Vec<Value> {
    let mut left = BTreeMap::new();
    let mut right = BTreeMap::new();
    flatten(before, "", &mut left);
    flatten(after, "", &mut right);
    let keys: BTreeSet<String> = left.keys().chain(right.keys()).cloned().collect();
    keys.into_iter()
        .filter_map(|key| match (left.get(&key), right.get(&key)) {
            (Some(a), Some(b)) if a == b => None,
            (Some(a), Some(b)) => {
                Some(json!({"field":key,"status":"modified","before":a,"after":b}))
            }
            (Some(a), None) => Some(json!({"field":key,"status":"removed","before":a})),
            (None, Some(b)) => Some(json!({"field":key,"status":"added","after":b})),
            (None, None) => None,
        })
        .take(200)
        .collect()
}
fn tree_bytes(value: &str) -> usize {
    STANDARD.decode(value).map(|v| v.len()).unwrap_or_default()
}
fn file_diff(before: &Tree, after: &Tree) -> Vec<Value> {
    let keys: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    keys.into_iter()
        .filter_map(|path| match (before.get(path), after.get(path)) {
            (Some(a), Some(b)) if a == b => None,
            (Some(a), Some(b)) => Some(json!({
                "path": path,
                "status": "modified",
                "before": tree_bytes(a),
                "after": tree_bytes(b),
            })),
            (Some(a), None) => Some(json!({
                "path": path,
                "status": "removed",
                "before": tree_bytes(a),
            })),
            (None, Some(b)) => Some(json!({
                "path": path,
                "status": "added",
                "after": tree_bytes(b),
            })),
            (None, None) => None,
        })
        .take(512)
        .collect()
}
/// 上游（来源文件夹或来源工具）现在的内容。
struct Upstream {
    hash: String,
    spec: Option<Value>,
    files: Option<Tree>,
}
fn upstream_path(entry: &Entry, source: &Source, targets: &[Target]) -> Result<PathBuf> {
    match source.kind.as_str() {
        "tool" => {
            let target = targets
                .iter()
                .find(|t| t.id == source.path)
                .context("来源工具已不在管理范围内")?;
            if entry.kind == "skill" && !target.skills_supported {
                bail!("{} 不支持 Skills 同步", target.name);
            }
            Ok(if entry.kind == "skill" {
                target.skills.join(&entry.name)
            } else {
                target.config.clone()
            })
        }
        "folder" => Ok(PathBuf::from(&source.path)),
        _ => bail!("来源类型无法识别，请重新导入"),
    }
}
fn upstream(entry: &Entry, targets: &[Target]) -> Result<(Source, Upstream)> {
    let source = entry
        .source
        .clone()
        .context("该条目没有记录来源；用「更新」重新选择文件夹后会重新记录")?;
    if entry.kind == "skill" {
        let path = upstream_path(entry, &source, targets)?;
        let files = skills::read_tree(&path)?.context("来源文件夹不存在或已被移走")?;
        if source.kind != "tool" && !files.contains_key("SKILL.md") {
            bail!("来源已不是有效的 Skill 文件夹（缺少 SKILL.md）");
        }
        let hash = skills::tree_hash(&files);
        return Ok((
            source,
            Upstream {
                hash,
                spec: None,
                files: Some(files),
            },
        ));
    }
    if source.kind != "tool" {
        bail!("MCP 条目只能从工具配置更新");
    }
    let target = targets
        .iter()
        .find(|t| t.id == source.path)
        .context("来源工具已不在管理范围内")?;
    let raw = adapters::read_servers(target)?
        .get(&source.name)
        .cloned()
        .context("来源工具里已没有同名 MCP")?;
    let spec = adapters::unproject(raw, &target.tool)?;
    Ok((
        source,
        Upstream {
            hash: hash(&serde_json::to_vec(&spec)?),
            spec: Some(spec),
            files: None,
        },
    ))
}
fn upstream_changes(
    entry: &Entry,
    source: &Source,
    up: &Upstream,
) -> (Vec<Value>, Vec<Value>, bool) {
    let fields = up
        .spec
        .as_ref()
        .map(|spec| field_diff(&entry.spec, spec))
        .unwrap_or_default();
    let files = up
        .files
        .as_ref()
        .map(|files| file_diff(&entry.files, files))
        .unwrap_or_default();
    let changed = if source.hash.is_empty() {
        !fields.is_empty() || !files.is_empty()
    } else {
        up.hash != source.hash
    };
    (files, fields, changed)
}
fn source_hash(entry: &Entry) -> Result<String> {
    Ok(if entry.kind == "mcp" {
        hash(&serde_json::to_vec(&entry.spec)?)
    } else {
        skills::tree_hash(&entry.files)
    })
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Action {
    SaveMcp {
        id: Option<String>,
        name: String,
        spec: Value,
    },
    ImportSkill {
        path: String,
        name: String,
        id: Option<String>,
    },
    ImportMcp {
        target: String,
        name: String,
        as_name: String,
    },
    Toggle {
        id: String,
        target: String,
        enabled: bool,
    },
    Sync {
        id: Option<String>,
        target: Option<String>,
    },
    Delete {
        id: String,
    },
    UpdateInfo {
        id: String,
        title: String,
        summary: String,
        detail: String,
        tags: Vec<String>,
        repository: String,
    },
    ApplyUpdate {
        id: String,
        expected: String,
    },
    ResolveConflict {
        id: String,
        target: String,
        choice: String,
        name: Option<String>,
    },
    RestoreBackup {
        target: String,
    },
}
fn action(root: &Path, library: &mut Library, targets: &[Target], action: Action) -> Result<Value> {
    match action {
        Action::SaveMcp { id, name, spec } => {
            valid_name(&name)?;
            ensure_unique(library, &name, "mcp", id.as_deref())?;
            let spec = adapters::normalize(spec)?;
            if let Some(id) = &id {
                let entry = entry_mut(library, id)?;
                if entry.kind != "mcp" || entry.name != name {
                    bail!("编辑时不能改变能力类型或编号");
                }
                if entry.spec != spec {
                    // 手工改过内容就与来源脱钩，避免「更新」把手工修改顶掉。
                    entry.source = None;
                }
                entry.spec = spec;
            } else {
                let mut entry = make_entry(name, "mcp")?;
                entry.spec = spec;
                library.entries.push(entry);
            }
            save(root, library)?;
            if let Some(id) = id {
                sync_selected(root, library, targets, Some(&id), None)
            } else {
                Ok(json!({"results":[]}))
            }
        }
        Action::ImportSkill { path, name, id } => {
            valid_name(&name)?;
            ensure_unique(library, &name, "skill", id.as_deref())?;
            let files = skills::read_tree(Path::new(&path))?.context("Skill 文件夹不存在")?;
            if !files.contains_key("SKILL.md") {
                bail!("请选择直接包含 SKILL.md 的文件夹");
            }
            let recorded = Source {
                kind: "folder".into(),
                path,
                name: name.clone(),
                hash: skills::tree_hash(&files),
            };
            if let Some(id) = &id {
                let entry = entry_mut(library, id)?;
                if entry.kind != "skill" || entry.name != name {
                    bail!("更新时必须保留 Skill 编号");
                }
                entry.files = files;
                entry.source = Some(recorded);
            } else {
                let mut entry = make_entry(name, "skill")?;
                entry.files = files;
                entry.source = Some(recorded);
                library.entries.push(entry);
            }
            save(root, library)?;
            if let Some(id) = id {
                sync_selected(root, library, targets, Some(&id), None)
            } else {
                Ok(json!({"results":[]}))
            }
        }
        Action::ImportMcp {
            target,
            name,
            as_name,
        } => {
            valid_name(&as_name)?;
            ensure_unique(library, &as_name, "mcp", None)?;
            let target = targets
                .iter()
                .find(|t| t.id == target)
                .context("未知工具")?;
            let source = adapters::read_servers(target)?
                .get(&name)
                .cloned()
                .context("来源条目已变化，请刷新后重试")?;
            let mut entry = make_entry(as_name, "mcp")?;
            entry.spec = adapters::unproject(source.clone(), &target.tool)?;
            entry.source = Some(Source {
                kind: "tool".into(),
                path: target.id.clone(),
                name: name.clone(),
                hash: source_hash(&entry)?,
            });
            if entry.name == name {
                entry.enabled.insert(target.id.clone(), true);
                entry.applied.insert(
                    target.id.clone(),
                    Receipt {
                        path: target.config.to_string_lossy().into(),
                        hash: hash(&serde_json::to_vec(&source)?),
                    },
                );
            }
            library.entries.push(entry);
            save(root, library)?;
            Ok(json!({"results":[]}))
        }
        Action::Toggle {
            id,
            target,
            enabled,
        } => {
            let selected = targets
                .iter()
                .find(|t| t.id == target)
                .context("未知工具")?;
            let entry = entry_mut(library, &id)?;
            if enabled && entry.kind == "mcp" {
                adapters::project(&entry.spec, &selected.tool)?;
            }
            entry.enabled.insert(target.clone(), enabled);
            if !enabled && !entry.applied.contains_key(&target) {
                entry.errors.remove(&target);
            }
            save(root, library)?;
            sync_selected(root, library, targets, Some(&id), Some(&target))
        }
        Action::Sync { id, target } => {
            sync_selected(root, library, targets, id.as_deref(), target.as_deref())
        }
        Action::Delete { id } => {
            let entry = entry_mut(library, &id)?;
            for value in entry.enabled.values_mut() {
                *value = false;
            }
            save(root, library)?;
            let results = sync_selected(root, library, targets, Some(&id), None)?;
            if entry_mut(library, &id)?.applied.is_empty() {
                library.entries.retain(|e| e.id != id);
                save(root, library)?;
            }
            Ok(results)
        }
        Action::UpdateInfo {
            id,
            title,
            summary,
            detail,
            tags,
            repository,
        } => {
            let title = title.trim();
            let summary = summary.trim();
            let detail = detail.trim();
            let repository = repository.trim();
            if title.chars().count() > 80 {
                bail!("名称最多 80 个字符");
            }
            if summary.chars().count() > 500 {
                bail!("能力说明最多 500 个字符");
            }
            if detail.chars().count() > 8000 {
                bail!("Skill / MCP 详情最多 8000 个字符");
            }
            if repository.chars().count() > 500 {
                bail!("开源项目地址最多 500 个字符");
            }
            if !repository.is_empty()
                && !(repository.starts_with("https://") || repository.starts_with("http://"))
            {
                bail!("开源项目地址需要 http(s) 链接");
            }
            let mut normalized = Vec::new();
            for tag in tags {
                let tag = tag.trim();
                if tag.is_empty() || tag.chars().count() > 32 {
                    continue;
                }
                if !normalized
                    .iter()
                    .any(|known: &String| known.eq_ignore_ascii_case(tag))
                {
                    normalized.push(tag.to_owned());
                }
                if normalized.len() >= 12 {
                    break;
                }
            }
            let entry = entry_mut(library, &id)?;
            // 只改展示元数据；spec / files / source / 各工具副本均保持不变。
            entry.title = title.to_owned();
            entry.summary = summary.to_owned();
            entry.detail = detail.to_owned();
            entry.tags = normalized;
            entry.repository = repository.to_owned();
            save(root, library)?;
            Ok(json!({"results":[]}))
        }
        Action::ApplyUpdate { id, expected } => {
            let index = library
                .entries
                .iter()
                .position(|e| e.id == id)
                .context("能力条目已不存在，请刷新")?;
            let (source, up) = upstream(&library.entries[index], targets)?;
            if up.hash != expected {
                bail!("来源在查看差异后发生变化，请重新查看差异再更新");
            }
            let entry = &mut library.entries[index];
            if let Some(files) = up.files {
                entry.files = files;
            }
            if let Some(spec) = up.spec {
                entry.spec = spec;
            }
            entry.source = Some(Source {
                hash: up.hash,
                ..source
            });
            save(root, library)?;
            sync_selected(root, library, targets, Some(&id), None)
        }
        Action::ResolveConflict {
            id,
            target,
            choice,
            name,
        } => {
            let selected = targets
                .iter()
                .find(|t| t.id == target)
                .context("未知工具")?;
            let index = library
                .entries
                .iter()
                .position(|e| e.id == id)
                .context("能力条目已不存在，请刷新")?;
            match choice.as_str() {
                "adopt" => {
                    adopt_from_tool(selected, &mut library.entries[index])?;
                    library.entries[index].applied.remove(&selected.id);
                    library.entries[index]
                        .enabled
                        .insert(selected.id.clone(), true);
                    let failure = sync_entry(root, &mut library.entries[index], selected, true)
                        .err()
                        .map(|e| format!("{e:#}"));
                    if let Some(error) = &failure {
                        library.entries[index]
                            .errors
                            .insert(selected.id.clone(), error.clone());
                    } else {
                        library.entries[index].errors.remove(&selected.id);
                    }
                    save(root, library)?;
                    failure.map_or_else(|| Ok(json!({"results":[]})), |error| bail!("{error}"))
                }
                "overwrite" => {
                    library.entries[index]
                        .enabled
                        .insert(selected.id.clone(), true);
                    let failure = sync_entry(root, &mut library.entries[index], selected, true)
                        .err()
                        .map(|e| format!("{e:#}"));
                    if let Some(error) = &failure {
                        library.entries[index]
                            .errors
                            .insert(selected.id.clone(), error.clone());
                    } else {
                        library.entries[index].errors.remove(&selected.id);
                    }
                    save(root, library)?;
                    failure.map_or_else(|| Ok(json!({"results":[]})), |error| bail!("{error}"))
                }
                "duplicate" => {
                    let as_name =
                        name.unwrap_or_else(|| format!("{}-local", library.entries[index].name));
                    valid_name(&as_name)?;
                    ensure_unique(library, &as_name, &library.entries[index].kind, None)?;
                    let mut entry =
                        duplicate_from_tool(selected, &library.entries[index], as_name)?;
                    entry.enabled.insert(selected.id.clone(), false);
                    library.entries.push(entry);
                    save(root, library)?;
                    Ok(json!({"results":[]}))
                }
                _ => bail!("未知的冲突处理方式"),
            }
        }
        Action::RestoreBackup { target } => {
            let selected = targets
                .iter()
                .find(|t| t.id == target)
                .context("未知工具")?;
            let path = backup_path(root, selected);
            let backup: Value = read_secret(&path)?.context("该工具没有可还原的首次同步备份")?;
            if backup["path"] != json!(selected.config.to_string_lossy()) {
                bail!("备份记录与当前配置路径不一致，未还原");
            }
            let original = backup["original"]
                .as_str()
                .map(|value| STANDARD.decode(value))
                .transpose()
                .context("备份内容损坏，未还原")?;
            let bytes =
                original.context("首次同步前该工具没有配置文件，无法还原；可手动移除受管条目")?;
            crate::platform::secure_storage::write_auth(&selected.config, &bytes)?;
            fs::remove_file(&path)?;
            for entry in &mut library.entries {
                entry.enabled.insert(selected.id.clone(), false);
                entry.applied.remove(&selected.id);
                entry.errors.remove(&selected.id);
            }
            save(root, library)?;
            Ok(json!({"results":[{
                "entryId": "",
                "targetId": selected.id,
                "name": "首次同步前的配置",
                "target": selected.name,
                "ok": true,
                "conflict": false,
                "error": null
            }]}))
        }
    }
}
fn adopt_from_tool(target: &Target, entry: &mut Entry) -> Result<()> {
    if entry.kind == "mcp" {
        let raw = adapters::read_servers(target)?
            .get(&entry.name)
            .cloned()
            .context("目标里已没有同名 MCP，请刷新")?;
        let spec = adapters::unproject(raw, &target.tool)?;
        entry.spec = spec;
        let hash = source_hash(entry)?;
        entry.source = Some(Source {
            kind: "tool".into(),
            path: target.id.clone(),
            name: entry.name.clone(),
            hash,
        });
        return Ok(());
    }
    let path = target.skills.join(&entry.name);
    entry.files = skills::read_tree(&path)?.context("目标里已没有同名 Skill，请刷新")?;
    let hash = source_hash(entry)?;
    entry.source = Some(Source {
        kind: "tool".into(),
        path: target.id.clone(),
        name: entry.name.clone(),
        hash,
    });
    Ok(())
}
fn duplicate_from_tool(target: &Target, entry: &Entry, as_name: String) -> Result<Entry> {
    let mut copy = make_entry(as_name.clone(), &entry.kind)?;
    copy.source = Some(Source {
        kind: "tool".into(),
        path: target.id.clone(),
        name: entry.name.clone(),
        hash: String::new(),
    });
    if entry.kind == "mcp" {
        let raw = adapters::read_servers(target)?
            .get(&entry.name)
            .cloned()
            .context("目标里已没有同名 MCP，请刷新")?;
        copy.spec = adapters::unproject(raw, &target.tool)?;
        copy.source.as_mut().unwrap().hash = hash(&serde_json::to_vec(&copy.spec)?);
    } else {
        let path = target.skills.join(&entry.name);
        copy.files = skills::read_tree(&path)?.context("目标里已没有同名 Skill，请刷新")?;
        copy.source.as_mut().unwrap().hash = skills::tree_hash(&copy.files);
    }
    Ok(copy)
}
fn allowed(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("能力库仅允许主窗口调用".into())
    }
}
fn root(app: &tauri::AppHandle) -> Result<PathBuf> {
    Ok(app.path().app_data_dir()?.join("capability-library"))
}
/// 用户添加了哪些 Agent：`ai_tool_cards` 展开后的卡片键（默认就含 claude / chatgpt）。
/// 同步目标按这份名单过滤 —— 没添加的工具不会出现在「同步位置」里。
fn enabled_agents(app: &tauri::AppHandle) -> Vec<String> {
    app.state::<crate::config::ConfigMutex>()
        .lock()
        .tool_cards()
}
#[tauri::command]
pub async fn capability_snapshot(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Value, String> {
    allowed(&window)?;
    let state = app.state::<crate::ai::management::State>();
    let _guard = state.operation.lock().await;
    let environment = app.state::<crate::platform::state::EnvironmentOperation>();
    let _environment = environment.lock().await;
    let result = (|| {
        crate::ai::management::require_ready(&app.path().app_data_dir()?)?;
        let root = root(&app)?;
        let enabled = enabled_agents(&app);
        let targets = targets(&user_home()?, true, &enabled)?;
        skills::recover(&root, &targets)?;
        Ok(snapshot(
            &root,
            &load(&root)?,
            &targets,
            &agent_rows(&enabled),
        ))
    })();
    result.map_err(|e: anyhow::Error| format!("{e:#}"))
}
#[tauri::command]
pub async fn capability_action(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    action: Action,
) -> Result<Value, String> {
    allowed(&window)?;
    let state = app.state::<crate::ai::management::State>();
    let _guard = state.operation.lock().await;
    let environment = app.state::<crate::platform::state::EnvironmentOperation>();
    let _environment = environment.lock().await;
    let result = (|| {
        crate::ai::management::require_ready(&app.path().app_data_dir()?)?;
        let root = root(&app)?;
        let enabled = enabled_agents(&app);
        let targets = targets(&user_home()?, true, &enabled)?;
        skills::recover(&root, &targets)?;
        let mut library = load(&root)?;
        let result = action_fn(&root, &mut library, &targets, action)?;
        Ok(
            json!({"snapshot":snapshot(&root,&library,&targets,&agent_rows(&enabled)),"results":result["results"]}),
        )
    })();
    result.map_err(|e: anyhow::Error| format!("{e:#}"))
}
fn action_fn(
    root: &Path,
    library: &mut Library,
    targets: &[Target],
    value: Action,
) -> Result<Value> {
    action(root, library, targets, value)
}
#[tauri::command]
pub async fn capability_sources(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    target: String,
) -> Result<Value, String> {
    allowed(&window)?;
    let state = app.state::<crate::ai::management::State>();
    let _guard = state.operation.lock().await;
    let result = (|| {
        let all = targets(&user_home()?, true, &enabled_agents(&app))?;
        let t = all.iter().find(|t| t.id == target).context("未知工具")?;
        let mut out = vec![];
        match adapters::read_servers(t) {
            Ok(servers) => {
                for (name, _) in servers {
                    out.push(json!({"name":name,"kind":"mcp"}));
                }
            }
            Err(e) => out.push(json!({"kind":"error","name":format!("MCP 读取失败：{e:#}")})),
        }
        if t.skills_supported && t.skills.exists() {
            for child in fs::read_dir(&t.skills)? {
                let path = child?.path();
                if path.join("SKILL.md").is_file() {
                    out.push(json!({"name":path.file_name().unwrap_or_default().to_string_lossy(),"kind":"skill","path":path}));
                }
                if out.len() > 1000 {
                    bail!("来源内容过多，请使用文件夹导入");
                }
            }
        }
        Ok(json!(out))
    })();
    result.map_err(|e: anyhow::Error| format!("{e:#}"))
}
#[tauri::command]
pub async fn capability_mcp_detail(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    id: String,
) -> Result<Value, String> {
    allowed(&window)?;
    let state = app.state::<crate::ai::management::State>();
    let _guard = state.operation.lock().await;
    (|| {
        let mut library = load(&root(&app)?)?;
        let entry = entry_mut(&mut library, &id)?;
        if entry.kind != "mcp" {
            bail!("不是 MCP 条目");
        }
        Ok(entry.spec.clone())
    })()
    .map_err(|e: anyhow::Error| format!("{e:#}"))
}
/// 只看不写：把「来源现在的内容」和库里的条目比一遍，交给界面确认后再更新。
#[tauri::command]
pub async fn capability_diff(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    id: String,
) -> Result<Value, String> {
    allowed(&window)?;
    let state = app.state::<crate::ai::management::State>();
    let _guard = state.operation.lock().await;
    (|| {
        let root = root(&app)?;
        let targets = targets(&user_home()?, true, &enabled_agents(&app))?;
        skills::recover(&root, &targets)?;
        let library = load(&root)?;
        let entry = library
            .entries
            .iter()
            .find(|e| e.id == id)
            .context("能力条目已不存在，请刷新")?;
        let (source, up) = upstream(entry, &targets)?;
        let (files, fields, changed) = upstream_changes(entry, &source, &up);
        Ok(json!({
            "name": entry.name,
            "kind": entry.kind,
            "source": source,
            "changed": changed,
            "hash": up.hash,
            "files": files,
            "fields": fields,
        }))
    })()
    .map_err(|e: anyhow::Error| format!("{e:#}"))
}

/// Skill 牌面数据：文件清单 + SKILL.md 的前若干行；只读，不触碰目标目录。
const PREVIEW_LIMIT: usize = 2400;
fn skill_detail(library: &Library, id: &str) -> Result<Value> {
    let entry = library
        .entries
        .iter()
        .find(|e| e.id == id)
        .context("能力条目已不存在，请刷新")?;
    if entry.kind != "skill" {
        bail!("不是 Skill 条目");
    }
    let files: Vec<Value> = entry
        .files
        .iter()
        .map(|(path, data)| json!({"path": path, "size": tree_bytes(data)}))
        .collect();
    let preview = entry
        .files
        .get("SKILL.md")
        .map(|data| {
            let bytes = STANDARD.decode(data).unwrap_or_default();
            let text = String::from_utf8_lossy(&bytes);
            let mut out: String = text.chars().take(PREVIEW_LIMIT).collect();
            if text.chars().count() > PREVIEW_LIMIT {
                out.push_str("\n…");
            }
            out
        })
        .unwrap_or_default();
    Ok(json!({
        "id": entry.id,
        "name": entry.name,
        "source": entry.source,
        "fileCount": entry.files.len(),
        "files": files,
        "preview": preview,
    }))
}
#[tauri::command]
pub async fn capability_skill_detail(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    id: String,
) -> Result<Value, String> {
    allowed(&window)?;
    let state = app.state::<crate::ai::management::State>();
    let _guard = state.operation.lock().await;
    (|| skill_detail(&load(&root(&app)?)?, &id))().map_err(|e: anyhow::Error| format!("{e:#}"))
}
