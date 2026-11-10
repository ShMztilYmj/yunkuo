//! 应用兼容配置契约：保持现有 config.yaml 格式，保护 crate 不再包含 AI 和界面字段。
pub(crate) mod patch;
use parking_lot::Mutex;
pub(crate) use protection::config::{FileStore, atomic_write};
use rand::Rng;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::fs;

// 混合代理端口跟随 Clash / Codex .env 生态的惯例 7890；控制器继续使用高位端口。
pub const DEFAULT_MIXED_PORT: u16 = 7890;
pub const DEFAULT_CONTROLLER_PORT: u16 = 19097;
pub const MIN_LISTEN_PORT: u16 = 1;
pub const MAX_LISTEN_PORT: u16 = 65535;
/// 历史默认值：启动时只迁移这些值，用户自定义端口（含低位）保持不变。
const LEGACY_MIXED_PORTS: [u16; 2] = [17897, 7897];
const LEGACY_CONTROLLER_PORTS: [u16; 1] = [9097];
// 内部旧键保持不变，避免更新工具名称后丢失启动入口和卡片选择。
pub const AI_TOOL_KEYS: [&str; 9] = [
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
const EXTRA_SHORTCUT_KEYS: [&str; 6] = [
    "glm",
    "deepseek",
    "kimi",
    "antigravity",
    "cursor",
    "opencode",
];

fn random_secret() -> String {
    let mut rng = rand::thread_rng();
    (0..24)
        .map(|_| format!("{:x}", rng.gen_range(0..16)))
        .collect()
}

fn native_engine<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let value = String::deserialize(deserializer)?;
    match value.as_str() {
        "native" | "mihomo" | "meow-embedded" => Ok("mihomo".into()),
        _ => Err(serde::de::Error::custom("不支持此代理引擎")),
    }
}

/// 应用主配置（对应数据目录下 config.yaml）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// 旧引擎名称仅用于读取迁移；运行与保存始终为mihomo 代理。
    #[serde(deserialize_with = "native_engine")]
    pub proxy_engine: String,
    /// 原生混合代理端口
    pub mixed_port: u16,
    /// external-controller 端口
    pub external_controller_port: u16,
    /// 控制器密钥
    pub secret: String,
    /// 系统代理开关
    pub enable_system_proxy: bool,
    /// 系统代理守护（防止其他程序篡改）
    pub enable_proxy_guard: bool,
    /// 守护检查间隔（秒）
    pub proxy_guard_interval: u64,
    /// 系统代理绕过列表
    pub proxy_bypass: String,
    /// TUN 模式开关
    pub enable_tun: bool,
    /// 运行模式 rule | global | direct
    pub mode: String,
    /// None 保留旧版运行配置，直到用户明确选择网络模式。
    pub network_mode: Option<crate::network_mode::NetworkMode>,
    pub daily_proxy: Option<String>,
    /// 内核日志等级
    pub log_level: String,
    pub allow_lan: bool,
    pub ipv6: bool,
    /// 开机自启
    pub enable_auto_launch: bool,
    /// 启动时静默（不显示窗口）
    pub enable_silent_start: bool,
    /// 关闭窗口时最小化到托盘。
    ///
    /// 默认开启；老配置里缺少该字段时也按开启填充（结构体级 `serde(default)`），
    /// 只有显式写成 false 才会退化成「点关闭就退出」。
    pub close_to_tray: bool,
    /// 启动内核后自动按 IP 同步时区
    pub tz_auto_sync: bool,
    /// 关闭系统代理时自动还原时区
    pub tz_restore_on_proxy_off: bool,
    /// Claude 启动快捷方式（.lnk / .exe）路径
    pub claude_shortcut: String,
    /// 各 AI 工具启动快捷方式（.lnk / .exe / .bat / .cmd）
    pub chatgpt_shortcut: String,
    pub grok_shortcut: String,
    /// 启动页与悬浮窗共用的卡片清单，默认保留 Claude 与 ChatGPT / Codex。
    pub ai_tool_cards: Vec<String>,
    pub ai_tool_shortcuts: std::collections::BTreeMap<String, String>,

    /// 本地用量扫描：关闭后不再读取各工具的会话文件（省 IO，统计页显示为未开启）。
    pub ai_usage_scan: bool,
    /// 环境检测通过阈值（评分低于该值视为低风险）
    pub gate_threshold: i32,
    /// 启动前要求 claude.ai 可达
    pub require_claude_reachable: bool,
    /// 出口 IP 禁止的国家/地区代码
    pub blocked_countries: Vec<String>,
    /// ipapi.is 免费 API Key（可选，1,000 次/天）：填入后出口情报多一个独立源，
    /// 提供 is_datacenter / is_vpn / is_proxy / is_tor / is_crawler 等旗标。
    pub ipapiis_key: String,
    /// 上次时区同步信息
    pub last_tz_sync: Option<TzSyncInfo>,
    /// 全局代理+TUN 触发默认浏览器切换时的原 http/https ProgId 备份
    pub default_browser_backup: Option<DefaultBrowserBackup>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            proxy_engine: "mihomo".into(),
            mixed_port: DEFAULT_MIXED_PORT,
            external_controller_port: DEFAULT_CONTROLLER_PORT,
            secret: random_secret(),
            enable_system_proxy: false,
            enable_proxy_guard: true,
            proxy_guard_interval: 10,
            proxy_bypass: "localhost;127.*;192.168.*;10.*;172.16.*;<local>".into(),
            enable_tun: false,
            mode: "rule".into(),
            network_mode: None,
            daily_proxy: None,
            log_level: "info".into(),
            allow_lan: false,
            ipv6: false,
            enable_auto_launch: false,
            enable_silent_start: false,
            close_to_tray: true,
            tz_auto_sync: false,
            tz_restore_on_proxy_off: false,
            claude_shortcut: String::new(),
            chatgpt_shortcut: String::new(),
            grok_shortcut: String::new(),
            ai_tool_cards: vec!["claude".into(), "chatgpt".into()],
            ai_tool_shortcuts: Default::default(),
            ai_usage_scan: true,
            gate_threshold: 31,
            require_claude_reachable: true,
            blocked_countries: vec!["CN", "HK", "MO", "RU", "BY", "KP", "IR", "CU", "SY", "VE"]
                .into_iter()
                .map(String::from)
                .collect(),
            ipapiis_key: String::new(),
            last_tz_sync: None,
            default_browser_backup: None,
        }
    }
}

/// 默认浏览器切换备份（原 http/https UserChoice ProgId）
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DefaultBrowserBackup {
    pub http: String,
    pub https: String,
}

impl AppConfig {
    /// 只迁移历史默认端口；用户自定义端口（含 8080 这类低位）保持不变，
    /// 迁移后若两个端口重复则顺延一位，避免内核绑定失败。
    pub(crate) fn migrate_listen_ports(&mut self) -> bool {
        let before = (self.mixed_port, self.external_controller_port);
        if self.mixed_port == 0 || LEGACY_MIXED_PORTS.contains(&self.mixed_port) {
            self.mixed_port = DEFAULT_MIXED_PORT;
        }
        if self.external_controller_port == 0
            || LEGACY_CONTROLLER_PORTS.contains(&self.external_controller_port)
        {
            self.external_controller_port = DEFAULT_CONTROLLER_PORT;
        }
        if self.mixed_port == self.external_controller_port {
            self.external_controller_port = if self.mixed_port < MAX_LISTEN_PORT {
                self.mixed_port + 1
            } else {
                self.mixed_port - 1
            };
        }
        before != (self.mixed_port, self.external_controller_port)
    }

    /// 日常分流与严格保护都固定关闭 IPv6。旧版本给日常分流写过 `ipv6: true`，
    /// 方案校验会拒绝这种组合并让内核起不来，所以加载时按方案归正。
    pub(crate) fn normalize_managed_ipv6(&mut self) -> bool {
        if self.network_mode.is_some() && self.ipv6 {
            self.ipv6 = false;
            return true;
        }
        false
    }

    pub fn tool_cards(&self) -> Vec<String> {
        let mut cards = vec!["claude".to_string(), "chatgpt".to_string()];
        for key in &self.ai_tool_cards {
            if AI_TOOL_KEYS.contains(&key.as_str()) && !cards.contains(key) {
                cards.push(key.clone());
            }
        }
        cards
    }

    pub fn tool_shortcut(&self, key: &str) -> Option<&str> {
        match key {
            "claude" => Some(&self.claude_shortcut),
            "chatgpt" => Some(&self.chatgpt_shortcut),
            "grok" => Some(&self.grok_shortcut),
            key if EXTRA_SHORTCUT_KEYS.contains(&key) => {
                self.ai_tool_shortcuts.get(key).map(String::as_str)
            }
            _ => None,
        }
    }

    pub fn patch_tool_cards(
        &mut self,
        patch: &serde_json::Value,
    ) -> std::result::Result<(), String> {
        let cards: Option<Vec<String>> = patch
            .get("ai_tool_cards")
            .map(|value| {
                serde_json::from_value(value.clone()).map_err(|_| "无效的工具卡片清单".to_string())
            })
            .transpose()?;
        let shortcuts: Option<std::collections::BTreeMap<String, String>> = patch
            .get("ai_tool_shortcuts")
            .map(|value| {
                serde_json::from_value(value.clone()).map_err(|_| "无效的工具启动配置".to_string())
            })
            .transpose()?;
        if cards.as_ref().is_some_and(|values| {
            values.len() > AI_TOOL_KEYS.len()
                || values
                    .iter()
                    .any(|key| !AI_TOOL_KEYS.contains(&key.as_str()))
        }) {
            return Err("工具卡片包含不支持的工具".into());
        }
        if shortcuts.as_ref().is_some_and(|values| {
            values
                .keys()
                .any(|key| !EXTRA_SHORTCUT_KEYS.contains(&key.as_str()))
        }) {
            return Err("工具启动配置包含不支持的工具".into());
        }
        if let Some(cards) = cards {
            self.ai_tool_cards = cards;
            self.ai_tool_cards = self.tool_cards();
        }
        if let Some(shortcuts) = shortcuts {
            self.ai_tool_shortcuts = shortcuts;
        }
        Ok(())
    }

    pub fn controller_addr(&self) -> String {
        format!("127.0.0.1:{}", self.external_controller_port)
    }
}

/// 时区同步记录
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TzSyncInfo {
    pub ip: String,
    pub country: String,
    pub country_code: String,
    pub iana_tz: String,
    pub windows_tz: String,
    pub at: String,
}

/// 时区备份（首次同步前保存原始时区，用于一键还原）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TzBackup {
    pub key_name: String,
    pub display: String,
    pub backed_up_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn low_listener_ports_migrate_once_and_keep_the_original_backup() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("config.yaml");
        let original = "mixed_port: 7897\nexternal_controller_port: 9097\nsecret: fixture-original-secret\nchatgpt_shortcut: C:/existing/Codex.lnk\n";
        fs::write(&path, original).unwrap();
        let cfg = load(&path).unwrap();
        assert_eq!(
            (cfg.mixed_port, cfg.external_controller_port),
            (7890, 19097)
        );
        assert_eq!(cfg.secret, "fixture-original-secret");
        assert_eq!(cfg.chatgpt_shortcut, "C:/existing/Codex.lnk");
        assert_eq!(
            fs::read_to_string(path.with_extension("bak")).unwrap(),
            original
        );
        let persisted: AppConfig = FileStore::new(path.clone()).load().unwrap();
        assert_eq!(persisted.mixed_port, cfg.mixed_port);
        assert_eq!(load(&path).unwrap().external_controller_port, 19097);
        assert_eq!(
            fs::read_to_string(path.with_extension("bak")).unwrap(),
            original
        );
    }
    #[test]
    fn listener_port_migration_preserves_custom_ports_and_avoids_pair_collisions() {
        for (mixed, controller, expected) in [
            (0, 10000, (7890, 10000)),
            (17897, 19097, (7890, 19097)),
            (10001, 65535, (10001, 65535)),
            (8080, 23457, (8080, 23457)),
            (7897, 17897, (7890, 17897)),
            (19097, 9097, (19097, 19098)),
        ] {
            let mut cfg = AppConfig {
                mixed_port: mixed,
                external_controller_port: controller,
                ..Default::default()
            };
            cfg.migrate_listen_ports();
            assert_eq!((cfg.mixed_port, cfg.external_controller_port), expected);
            assert!(!cfg.migrate_listen_ports());
        }
    }
    #[test]
    fn startup_turns_ipv6_off_for_both_managed_network_modes() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("config.yaml");
        let store = FileStore::new(path.clone());
        for mode in [
            crate::network_mode::NetworkMode::Daily,
            crate::network_mode::NetworkMode::Strict,
        ] {
            let cfg = AppConfig {
                network_mode: Some(mode),
                ipv6: true,
                ..Default::default()
            };
            store.save(&cfg).unwrap();
            let loaded = load(&path).unwrap();
            assert!(!loaded.ipv6, "{mode:?}");
            assert!(fs::read_to_string(&path).unwrap().contains("ipv6: false"));
        }
        // 自定义方案保留用户自己的 IPv6 开关。
        let custom = AppConfig {
            ipv6: true,
            ..Default::default()
        };
        store.save(&custom).unwrap();
        assert!(load(&path).unwrap().ipv6);
    }
    #[test]
    fn old_engine_selection_migrates_without_losing_other_configuration() {
        for previous in ["mihomo", "meow-embedded", "native"] {
            let config: AppConfig =
                serde_yaml::from_str(&format!("proxy_engine: {previous}\nmixed_port: 3456\n"))
                    .unwrap();
            assert_eq!(config.proxy_engine, "mihomo");
            assert_eq!(config.mixed_port, 3456);
            assert!(
                serde_yaml::to_string(&config)
                    .unwrap()
                    .contains("proxy_engine: mihomo")
            );
        }
    }
    #[test]
    fn legacy_configs_default_to_two_cards_without_losing_shortcuts() {
        let cfg: AppConfig = serde_yaml::from_str("grok_shortcut: 'C:/tools/Grok.lnk'").unwrap();
        assert_eq!(cfg.tool_cards(), ["claude", "chatgpt"]);
        assert_eq!(cfg.tool_shortcut("grok"), Some("C:/tools/Grok.lnk"));
        assert_eq!(cfg.proxy_engine, "mihomo");
    }
    #[test]
    fn closing_to_tray_is_on_by_default_and_only_off_when_explicit() {
        assert!(AppConfig::default().close_to_tray);
        // 老配置没有这个字段时按默认开启填充，不能退化成「点关闭就退出」。
        let legacy: AppConfig = serde_yaml::from_str("mode: rule\n").unwrap();
        assert!(legacy.close_to_tray);
        // 只有显式关闭才生效。
        let explicit: AppConfig = serde_yaml::from_str("close_to_tray: false\n").unwrap();
        assert!(!explicit.close_to_tray);
    }
    #[test]
    fn nine_agents_persist_with_new_paths_and_legacy_entries_intact() {
        let mut cfg = AppConfig::default();
        cfg.chatgpt_shortcut = "C:/existing/Codex.lnk".into();
        cfg.grok_shortcut = "C:/existing/Grok.lnk".into();
        let cards = [
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
        cfg.patch_tool_cards(&serde_json::json!({"ai_tool_cards": cards, "ai_tool_shortcuts": {"cursor":"C:/tools/Cursor.exe", "opencode":"C:/tools/opencode.cmd", "glm":"C:/tools/ZCode.exe", "deepseek":"C:/tools/dsh.cmd"}})).unwrap();
        let serialized = serde_yaml::to_string(&cfg).unwrap();
        let loaded: AppConfig = serde_yaml::from_str(&serialized).unwrap();
        assert_eq!(loaded.tool_cards(), cards);
        for (key, path) in [
            ("cursor", "C:/tools/Cursor.exe"),
            ("opencode", "C:/tools/opencode.cmd"),
            ("glm", "C:/tools/ZCode.exe"),
            ("deepseek", "C:/tools/dsh.cmd"),
            ("chatgpt", "C:/existing/Codex.lnk"),
        ] {
            assert_eq!(loaded.tool_shortcut(key), Some(path));
        }
    }
    #[test]
    fn cards_normalize_and_persist_without_deleting_launch_paths() {
        let fixture = crate::test_support::FixtureDir::new();
        let store = FileStore::new(fixture.path().join("config.yaml"));
        let mut cfg = AppConfig::default();
        cfg.patch_tool_cards(&serde_json::json!({"ai_tool_cards": ["kimi", "claude", "kimi", "grok"], "ai_tool_shortcuts": {"kimi": "C:/tools/Kimi.exe"}})).unwrap();
        assert_eq!(cfg.tool_cards(), ["claude", "chatgpt", "kimi", "grok"]);
        cfg.patch_tool_cards(&serde_json::json!({"ai_tool_cards": []}))
            .unwrap();
        store.save(&cfg).unwrap();
        let mut loaded = store.load::<AppConfig>().unwrap();
        assert_eq!(loaded.tool_cards(), ["claude", "chatgpt"]);
        assert_eq!(loaded.tool_shortcut("kimi"), Some("C:/tools/Kimi.exe"));
        loaded
            .patch_tool_cards(&serde_json::json!({"ai_tool_cards": ["kimi"]}))
            .unwrap();
        assert_eq!(loaded.tool_shortcut("kimi"), Some("C:/tools/Kimi.exe"));
    }
    #[test]
    fn invalid_card_patches_do_not_partially_apply() {
        let mut cfg = AppConfig::default();
        let before = serde_json::to_value(&cfg).unwrap();
        for patch in [
            serde_json::json!({"ai_tool_cards": ["unknown"]}),
            serde_json::json!({"ai_tool_cards": null}),
            serde_json::json!({"ai_tool_cards": ["kimi"], "ai_tool_shortcuts": {"unknown": "bad.exe"}}),
            serde_json::json!({"ai_tool_shortcuts": {"kimi": 123}}),
        ] {
            assert!(cfg.patch_tool_cards(&patch).is_err());
            assert_eq!(serde_json::to_value(&cfg).unwrap(), before);
        }
    }
    #[test]
    fn damaged_config_is_not_defaulted_or_overwritten() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("config.yaml");
        let bad = b"mixed_port: [unfinished";
        fs::write(&path, bad).unwrap();
        assert!(FileStore::new(path.clone()).load::<AppConfig>().is_err());
        assert_eq!(fs::read(path).unwrap(), bad);
    }
    #[test]
    fn atomic_saves_keep_last_valid_backup_and_recover_corruption() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("config.yaml");
        let store = FileStore::new(path.clone());
        let first = AppConfig {
            mixed_port: 1234,
            ..Default::default()
        };
        let second = AppConfig {
            mixed_port: 4321,
            ..first.clone()
        };
        store.save(&first).unwrap();
        store.save(&second).unwrap();
        assert_eq!(store.load::<AppConfig>().unwrap().mixed_port, 4321);
        fs::write(path, b"invalid: [").unwrap();
        assert_eq!(store.load::<AppConfig>().unwrap().mixed_port, 1234);
    }
}

/// 启动迁移经由 FileStore 原子保存；原配置保留在 config.bak 中。
pub(crate) fn load(path: &std::path::Path) -> anyhow::Result<AppConfig> {
    let store = FileStore::new(path.to_path_buf());
    let mut cfg: AppConfig = store.load()?;
    // 两次迁移都要执行：短路会让后一次归正悄悄丢失。
    let ports = cfg.migrate_listen_ports();
    let ipv6 = cfg.normalize_managed_ipv6();
    if ports || ipv6 || !path.exists() {
        store.save(&cfg)?;
    }
    Ok(cfg)
}

pub type ConfigMutex = Mutex<AppConfig>;
