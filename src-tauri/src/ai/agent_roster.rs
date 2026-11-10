//! 唯一 Agent 名单。
//!
//! `src-tauri/agents.json` 是唯一的名单来源：存储键、协作运行时 ID、显示名、全部别名、
//! 安装线索、官方 hooks 配置路径与用量引擎客户端名都只在那里维护。任何模块都不应再
//! 写死工具名或别名表；新增一款 Agent 只改 JSON。
//!
//! 这里只依赖 serde 与 once_cell，不引入 Tauri，任何进程都能安全链接。
//!
//! 名单里有一部分字段目前只被前端和随包宿主脚本使用（HUD 名、简称、说明、用量客户端名），
//! 等这些调用点迁移完成后再去掉这里的豁免。
#![allow(dead_code)]

use once_cell::sync::Lazy;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDef {
    /// 已持久化的存储键（`chatgpt`、`glm` 等），绝不能改写。
    pub key: String,
    /// 协作工作台运行时 ID，与存储键可能不同（如 `chatgpt` → `codex`）。
    pub runtime: Option<String>,
    pub name: String,
    pub hud_name: String,
    /// 聊天头像 ID；与运行时不同的是 `claude` 用 `claude-code`。
    pub avatar: Option<String>,
    pub short: String,
    pub hint: String,
    /// 协作协议名，例如 `Claude Agent SDK`、`Codex app-server`、`ACP`。
    pub protocol: Option<String>,
    /// 界面上显示的短协议名，例如 `Agent SDK`、`serve`。
    pub protocol_short: Option<String>,
    /// 协作工作台里是否按“外部 Agent”处理。
    pub external: bool,
    /// 退役的旧卡片：仍然可读、可启动，但不再出现在新增列表里。
    pub retired: bool,
    /// 进程名、CLI 名与前端别名，全部小写。
    pub aliases: Vec<String>,
    /// 在本机 PATH 与安装目录里探测时使用的文件名。
    pub install_names: Vec<String>,
    /// npm 包路径特征，用于从解释器参数里认出 CLI 入口。
    pub npm_markers: Vec<String>,
    /// `[环境变量, 相对路径]` 形式的安装目录线索。
    pub install_dirs: Vec<(String, String)>,
    /// 官方 hooks 配置相对路径；为空表示该工具没有官方 hooks。
    pub hooks: Option<String>,
    /// 能力同步（MCP / Skills）的配置位置；为空表示该工具还没接入。
    pub capability: Option<CapabilityDef>,
    /// 用量引擎里可能出现的客户端名。
    pub usage_clients: Vec<String>,
}

/// 能力同步位置：`home` 相对用户目录，环境变量优先（`env` + `envHome`）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityPath {
    pub home: String,
    pub env: Option<String>,
    pub env_home: Option<String>,
}

/// 一款 Agent 的同步面：MCP 配置必有，Skills 目录可选（例如 Cursor 只有 MCP）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDef {
    /// 同步位置在界面里的顺序（越小越靠前），和名单顺序解耦。
    #[serde(default = "default_order")]
    pub order: u8,
    pub mcp: Option<CapabilityPath>,
    pub skills: Option<CapabilityPath>,
}
fn default_order() -> u8 {
    u8::MAX
}

#[derive(Debug, Deserialize)]
struct Roster {
    version: u32,
    #[serde(rename = "defaultRuntime")]
    default_runtime: String,
    agents: Vec<AgentDef>,
}

static ROSTER: Lazy<Roster> = Lazy::new(|| {
    let roster: Roster = serde_json::from_str(include_str!("../../agents.json"))
        .expect("agents.json 不是有效的名单");
    assert_eq!(roster.version, 1, "agents.json 版本不受支持");
    roster
});

static BY_KEY: Lazy<BTreeMap<&'static str, &'static AgentDef>> = Lazy::new(|| {
    let mut map = BTreeMap::new();
    for agent in &ROSTER.agents {
        assert!(
            map.insert(agent.key.as_str(), agent).is_none(),
            "agents.json 出现重复的存储键：{}",
            agent.key
        );
    }
    map
});

static BY_RUNTIME: Lazy<BTreeMap<&'static str, &'static AgentDef>> = Lazy::new(|| {
    let mut map = BTreeMap::new();
    for agent in &ROSTER.agents {
        if let Some(runtime) = agent.runtime.as_deref() {
            assert!(
                map.insert(runtime, agent).is_none(),
                "agents.json 出现重复的运行时 ID：{runtime}"
            );
        }
    }
    map
});

/// 别名 → 存储键。别名冲突时先出现的条目优先。
static BY_ALIAS: Lazy<BTreeMap<&'static str, &'static str>> = Lazy::new(|| {
    let mut map = BTreeMap::new();
    for agent in &ROSTER.agents {
        for alias in &agent.aliases {
            map.entry(alias.as_str()).or_insert(agent.key.as_str());
        }
    }
    map
});

/// 用量引擎客户端名 → 存储键。`antigravity-cli` 一类变体也折叠到同一个键。
static BY_CLIENT: Lazy<BTreeMap<&'static str, &'static str>> = Lazy::new(|| {
    let mut map = BTreeMap::new();
    for agent in &ROSTER.agents {
        for client in &agent.usage_clients {
            map.entry(client.as_str()).or_insert(agent.key.as_str());
        }
    }
    map
});

pub fn all() -> &'static [AgentDef] {
    &ROSTER.agents
}

/// 全部存储键，保持名单顺序。
pub fn keys() -> Vec<&'static str> {
    ROSTER
        .agents
        .iter()
        .map(|agent| agent.key.as_str())
        .collect()
}

/// 全部协作运行时 ID，保持名单顺序（退役工具没有运行时，会被跳过）。
pub fn runtime_ids() -> Vec<&'static str> {
    ROSTER
        .agents
        .iter()
        .filter_map(|agent| agent.runtime.as_deref())
        .collect()
}

pub fn get(key: &str) -> Option<&'static AgentDef> {
    BY_KEY.get(key).copied()
}

pub fn by_runtime(runtime: &str) -> Option<&'static AgentDef> {
    BY_RUNTIME.get(runtime).copied()
}

pub fn key_for_runtime(runtime: &str) -> Option<&'static str> {
    by_runtime(runtime).map(|agent| agent.key.as_str())
}

pub fn runtime_for(key: &str) -> Option<&'static str> {
    get(key).and_then(|agent| agent.runtime.as_deref())
}

/// 任意别名（进程名、CLI 名、运行时 ID、存储键）转存储键。
pub fn key_for_alias(name: &str) -> Option<&'static str> {
    let normalized = name.trim().to_ascii_lowercase();
    BY_ALIAS.get(normalized.as_str()).copied()
}

/// 按存储键或运行时 ID 取定义，方便调用方两种写法都能用。
pub fn find(key_or_runtime: &str) -> Option<&'static AgentDef> {
    get(key_or_runtime).or_else(|| by_runtime(key_or_runtime))
}

pub fn is_runtime(id: &str) -> bool {
    BY_RUNTIME.contains_key(id)
}

/// 协作运行时是否按外部 Agent 处理。
pub fn is_external(id: &str) -> bool {
    is_runtime(id) && by_runtime(id).is_some_and(|agent| agent.external)
}

pub fn protocol_for_runtime(id: &str) -> Option<&'static str> {
    by_runtime(id).and_then(|agent| agent.protocol.as_deref())
}

pub fn label_for_runtime(id: &str) -> Option<&'static str> {
    by_runtime(id).map(|agent| agent.name.as_str())
}

pub fn install_names(key: &str) -> &'static [String] {
    get(key).map_or(&[], |agent| agent.install_names.as_slice())
}

/// 官方 hooks 配置相对路径；`None` 表示没有官方 hooks。
pub fn hook_config(key: &str) -> Option<&'static str> {
    get(key).and_then(|agent| agent.hooks.as_deref())
}

/// 能力同步位置；`None` 表示这个 Agent 还没接入 MCP / Skills 同步。
pub fn capability(key: &str) -> Option<&'static CapabilityDef> {
    get(key).and_then(|agent| agent.capability.as_ref())
}

/// 同步目标的展示顺序：按运行时 ID 或存储键都能查回来。
pub fn capability_order(id: &str) -> u8 {
    ROSTER
        .agents
        .iter()
        .find(|agent| agent.key == id || agent.runtime.as_deref() == Some(id))
        .and_then(|agent| agent.capability.as_ref())
        .map_or(u8::MAX, |capability| capability.order)
}

/// 支持官方 hooks 的存储键，保持名单顺序。
pub fn hook_keys() -> Vec<&'static str> {
    ROSTER
        .agents
        .iter()
        .filter(|agent| agent.hooks.is_some())
        .map(|agent| agent.key.as_str())
        .collect()
}

/// 全部 npm 包路径特征与所属存储键。
pub fn npm_markers() -> Vec<(&'static str, &'static str)> {
    ROSTER
        .agents
        .iter()
        .flat_map(|agent| {
            agent
                .npm_markers
                .iter()
                .map(move |marker| (marker.as_str(), agent.key.as_str()))
        })
        .collect()
}

/// 全部安装目录线索。
pub fn install_dirs() -> Vec<(&'static str, &'static str)> {
    ROSTER
        .agents
        .iter()
        .flat_map(|agent| {
            agent
                .install_dirs
                .iter()
                .map(move |(variable, relative)| (variable.as_str(), relative.as_str()))
        })
        .collect()
}

/// 全部别名与所属存储键，用于前端与宿主脚本生成映射。
pub fn aliases() -> Vec<(&'static str, &'static str)> {
    ROSTER
        .agents
        .iter()
        .flat_map(|agent| {
            agent
                .aliases
                .iter()
                .map(move |alias| (alias.as_str(), agent.key.as_str()))
        })
        .collect()
}

/// 某个工具在用量引擎里使用的客户端名。
pub fn usage_clients(key: &str) -> Vec<&'static str> {
    get(key).map_or_else(Vec::new, |agent| {
        agent.usage_clients.iter().map(String::as_str).collect()
    })
}

/// 用量引擎客户端名 → 存储键。
pub fn key_for_client(client: &str) -> Option<&'static str> {
    BY_CLIENT.get(client).copied()
}

/// 新成员的默认协作运行时。
pub fn default_runtime() -> &'static str {
    &ROSTER.default_runtime
}

/// 头像 ID 是否有效；空值与 `auto` 由调用方自行处理。
pub fn is_avatar(value: &str) -> bool {
    ROSTER
        .agents
        .iter()
        .any(|agent| agent.avatar.as_deref() == Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    /// 已安装用户的数据依赖这十个键名，改动会让卡片与启动入口全部失效。
    const EXPECTED: [&str; 9] = [
        "claude",
        "chatgpt",
        "cursor",
        "antigravity",
        "opencode",
        "kimi",
        "grok",
        "glm",
        "deepseek",
    ];

    #[test]
    fn roster_keeps_every_persisted_key() {
        assert_eq!(keys(), EXPECTED);
        assert_eq!(all().len(), EXPECTED.len());
    }

    #[test]
    fn storage_keys_and_runtime_ids_round_trip() {
        assert_eq!(runtime_for("chatgpt"), Some("codex"));
        assert_eq!(key_for_runtime("codex"), Some("chatgpt"));
        assert_eq!(runtime_for("glm"), Some("glm"));
        assert_eq!(runtime_ids().len(), 9, "九款工具都有运行时");
        assert_eq!(key_for_runtime("deepseek"), Some("deepseek"));
    }

    #[test]
    fn aliases_resolve_to_storage_keys() {
        assert_eq!(key_for_alias("Codex"), Some("chatgpt"));
        assert_eq!(key_for_alias("claude-code"), Some("claude"));
        assert_eq!(key_for_alias("zcode-cli"), Some("glm"));
        assert_eq!(key_for_alias("DeepSeek Harness"), Some("deepseek"));
        assert_eq!(key_for_alias("grok-build"), Some("grok"));
        assert_eq!(key_for_alias("kimi_cli"), Some("kimi"));
        assert_eq!(key_for_alias("unknown-agent"), None);
    }

    #[test]
    fn hooks_only_cover_tools_with_official_support() {
        assert_eq!(hook_keys(), ["cursor", "antigravity", "grok"]);
        assert_eq!(
            hook_config("grok"),
            Some(".grok/hooks/yun-kuo-monitor.json")
        );
        assert_eq!(hook_config("claude"), None);
    }

    #[test]
    fn external_agents_exclude_native_runtimes() {
        assert!(!is_external("claude"));
        assert!(!is_external("codex"));
        assert!(!is_external("opencode"));
        assert!(is_external("kimi"));
        assert!(is_external("glm"));
    }

    #[test]
    fn install_hints_cover_every_tool_that_probes_path() {
        assert_eq!(install_names("chatgpt"), ["codex"]);
        assert_eq!(install_names("glm"), ["zcode", "zcode-cli"]);
        assert!(
            npm_markers()
                .iter()
                .any(|(m, key)| *m == "/@openai/codex/" && *key == "chatgpt")
        );
        assert!(
            install_dirs()
                .iter()
                .any(|(variable, path)| *variable == "LOCALAPPDATA" && *path == "Programs/ZCode")
        );
    }

    #[test]
    fn usage_clients_fold_back_to_one_storage_key() {
        assert_eq!(usage_clients("chatgpt"), ["codex"]);
        assert_eq!(
            usage_clients("antigravity"),
            ["antigravity", "antigravity-cli", "antigravity-extension"]
        );
        assert_eq!(key_for_client("antigravity-cli"), Some("antigravity"));
        assert_eq!(key_for_client("zcode"), Some("glm"));
        assert_eq!(key_for_client("dsh"), Some("deepseek"));
        assert_eq!(key_for_client("unknown-client"), None);
    }

    #[test]
    fn aliases_are_unique_per_tool() {
        let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
        for (alias, key) in aliases() {
            if let Some(previous) = seen.insert(alias, key) {
                assert_eq!(previous, key, "别名 {alias} 同时属于 {previous} 与 {key}");
            }
        }
    }
}
