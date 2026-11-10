//! 各模块可写配置的归属与纯校验。磁盘仍保持旧版扁平格式。
use super::AppConfig;
use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Domain {
    Proxy,
    Diagnostics,
    Ai,
    Desktop,
}

const FIELDS: &[(Domain, &[&str])] = &[
    (
        Domain::Proxy,
        &[
            "proxy_engine",
            "mixed_port",
            "external_controller_port",
            "enable_system_proxy",
            "enable_proxy_guard",
            "proxy_guard_interval",
            "proxy_bypass",
            "enable_tun",
            "mode",
            "network_mode",
            "daily_proxy",
            "log_level",
            "allow_lan",
            "ipv6",
        ],
    ),
    (
        Domain::Diagnostics,
        &[
            "tz_auto_sync",
            "tz_restore_on_proxy_off",
            "gate_threshold",
            "require_claude_reachable",
            "blocked_countries",
            "ipapiis_key",
        ],
    ),
    (
        Domain::Ai,
        &[
            "claude_shortcut",
            "chatgpt_shortcut",
            "grok_shortcut",
            "ai_tool_cards",
            "ai_tool_shortcuts",
            "ai_usage_scan",
        ],
    ),
    (
        Domain::Desktop,
        &["enable_auto_launch", "enable_silent_start", "close_to_tray"],
    ),
];

pub(crate) fn owner(field: &str) -> Option<Domain> {
    FIELDS
        .iter()
        .find_map(|(domain, fields)| fields.contains(&field).then_some(*domain))
}

pub(crate) struct ConfigPatch(Map<String, Value>);

impl ConfigPatch {
    pub(crate) fn parse(value: Value) -> Result<Self, String> {
        let Value::Object(fields) = value else {
            return Err("配置修改必须为对象".into());
        };
        for (key, value) in &fields {
            if owner(key).is_none() {
                return Err(format!("配置字段不允许通过此入口修改：{key}"));
            }
            if value.is_null() && !["network_mode", "daily_proxy"].contains(&key.as_str()) {
                return Err(format!("配置字段不能为空：{key}"));
            }
        }
        if fields
            .get("proxy_engine")
            .is_some_and(|v| v.as_str() != Some("mihomo"))
        {
            return Err("不支持此代理引擎".into());
        }
        Ok(Self(fields))
    }

    pub(crate) fn contains(&self, field: &str) -> bool {
        self.0.contains_key(field)
    }

    pub(crate) fn changes_network_mode(&self) -> bool {
        self.contains("network_mode") || self.contains("daily_proxy")
    }

    pub(crate) fn needs_runtime_apply(&self) -> bool {
        [
            "proxy_engine",
            "mode",
            "network_mode",
            "daily_proxy",
            "enable_tun",
            "mixed_port",
            "external_controller_port",
            "allow_lan",
            "ipv6",
            "log_level",
        ]
        .iter()
        .any(|field| self.contains(field))
    }

    /// 只构建新值；全部校验通过之前不改变原配置，也不执行系统副作用。
    pub(crate) fn apply(&self, before: &AppConfig) -> Result<AppConfig, String> {
        let mut fields = self.0.clone();
        if let Some(value) = fields.get_mut("gate_threshold") {
            *value = value
                .as_i64()
                .ok_or("检测阈值必须为整数")?
                .clamp(0, 100)
                .into();
        }
        if let Some(value) = fields.get_mut("proxy_guard_interval") {
            *value = value
                .as_u64()
                .ok_or("守护间隔必须为非负整数")?
                .max(3)
                .into();
        }
        for field in ["mixed_port", "external_controller_port"] {
            if let Some(value) = fields.get(field) {
                if !value
                    .as_u64()
                    .is_some_and(|port| (u64::from(super::MIN_LISTEN_PORT)..=65535).contains(&port))
                {
                    return Err("代理及控制器端口必须为 1–65535".into());
                }
            }
        }
        let mut candidate = serde_json::to_value(before).map_err(|_| "读取配置失败")?;
        candidate
            .as_object_mut()
            .ok_or("配置格式错误")?
            .extend(fields);
        let mut cfg: AppConfig =
            serde_json::from_value(candidate).map_err(|_| "配置字段类型不正确，请刷新后重试")?;
        cfg.patch_tool_cards(&Value::Object(self.0.clone()))?;
        if cfg.mixed_port == cfg.external_controller_port {
            return Err("代理端口与控制器端口不能相同".into());
        }
        // Legacy incomplete network plans must not block unrelated desktop/AI preferences.
        if self.needs_runtime_apply() || self.contains("enable_system_proxy") {
            crate::proxy::policy::Plan::resolve(&cfg).map_err(|e| format!("{e:#}"))?;
        }
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn listener_ports_accept_full_range_and_reject_duplicates() {
        let before = AppConfig::default();
        for field in ["mixed_port", "external_controller_port"] {
            for port in [0, 65536] {
                let patch = ConfigPatch::parse(json!({field:port})).unwrap();
                assert!(patch.apply(&before).is_err(), "{field}: {port}");
            }
        }
        for port in [7890, 10001, 65535] {
            let patch = ConfigPatch::parse(json!({"mixed_port":port})).unwrap();
            assert!(patch.apply(&before).is_ok(), "mixed_port: {port}");
        }
        for port in [10001, 65535] {
            let patch = ConfigPatch::parse(json!({"external_controller_port":port})).unwrap();
            assert!(
                patch.apply(&before).is_ok(),
                "external_controller_port: {port}"
            );
        }
        assert!(
            ConfigPatch::parse(json!({"mixed_port":before.external_controller_port}))
                .unwrap()
                .apply(&before)
                .is_err()
        );
    }

    #[test]
    fn bad_types_or_read_only_fields_cannot_silently_succeed() {
        let before = AppConfig::default();
        for value in [
            json!(null),
            json!([]),
            json!({"enable_tun":"true"}),
            json!({"mixed_port":0}),
            json!({"mixed_port":65536}),
            json!({"mixed_port":-1}),
            json!({"ipv6":null}),
            json!({"blocked_countries":["CN", 1]}),
            json!({"secret":"replacement"}),
            json!({"last_tz_sync":null}),
            json!({"mixed_prt":1234}),
            json!({"proxy_engine":"unsupported"}),
        ] {
            assert!(
                ConfigPatch::parse(value)
                    .and_then(|patch| patch.apply(&before))
                    .is_err()
            );
        }
    }

    #[test]
    fn module_changes_preserve_other_modules_and_normalize_legacy_values() {
        let before = AppConfig {
            chatgpt_shortcut: "C:/Existing.lnk".into(),
            ..Default::default()
        };
        let patch = ConfigPatch::parse(json!({"mixed_port":23456, "gate_threshold":-10,
            "proxy_guard_interval":1, "ai_tool_cards":["glm", "glm"]}))
        .unwrap();
        let after = patch.apply(&before).unwrap();
        assert_eq!(after.mixed_port, 23456);
        assert_eq!(after.chatgpt_shortcut, before.chatgpt_shortcut);
        assert_eq!(after.secret, before.secret);
        assert_eq!(after.gate_threshold, 0);
        assert_eq!(after.proxy_guard_interval, 3);
        assert_eq!(after.ai_tool_cards, ["claude", "chatgpt", "glm"]);
        assert!(patch.needs_runtime_apply());
        assert_eq!(before.mixed_port, super::super::DEFAULT_MIXED_PORT);
    }

    #[test]
    fn null_network_mode_is_an_explicit_clear_and_conflicts_are_rejected() {
        let before = AppConfig {
            network_mode: Some(crate::network_mode::NetworkMode::Daily),
            daily_proxy: Some("selected".into()),
            enable_tun: true,
            enable_system_proxy: true,
            ipv6: false,
            ..Default::default()
        };
        let clear = ConfigPatch::parse(json!({"network_mode":null,"daily_proxy":null})).unwrap();
        assert!(clear.changes_network_mode());
        let after = clear.apply(&before).unwrap();
        assert!(after.network_mode.is_none());
        assert!(after.daily_proxy.is_none());
        assert!(
            ConfigPatch::parse(json!({"ipv6":true}))
                .unwrap()
                .apply(&before)
                .is_err()
        );
        assert_eq!(owner("mixed_port"), Some(Domain::Proxy));
        assert_eq!(owner("gate_threshold"), Some(Domain::Diagnostics));
        assert_eq!(owner("ai_tool_cards"), Some(Domain::Ai));
        assert_eq!(owner("ai_usage_scan"), Some(Domain::Ai));
        assert_eq!(owner("close_to_tray"), Some(Domain::Desktop));
        // AI 用量扫描必须能通过设置页的通用补丁写盘，且默认开启。
        let patched = ConfigPatch::parse(json!({"ai_usage_scan":false}))
            .unwrap()
            .apply(&before)
            .unwrap();
        assert!(!patched.ai_usage_scan);
        assert!(AppConfig::default().ai_usage_scan);
    }
    #[test]
    fn managed_network_switches_cannot_be_changed_by_general_patch() {
        for mode in [
            crate::network_mode::NetworkMode::Daily,
            crate::network_mode::NetworkMode::Strict,
        ] {
            let before = crate::proxy::policy::managed_target(&AppConfig::default(), mode);
            for patch in [
                json!({"enable_tun":false}),
                json!({"enable_system_proxy":false}),
                json!({"mode":"direct"}),
                json!({"ipv6":true}),
            ] {
                assert!(ConfigPatch::parse(patch).unwrap().apply(&before).is_err());
            }
        }
    }
    #[test]
    fn legacy_incomplete_plan_does_not_block_non_network_preferences() {
        let before = AppConfig {
            network_mode: Some(crate::network_mode::NetworkMode::Strict),
            ..Default::default()
        };
        let after = ConfigPatch::parse(json!({"close_to_tray":false}))
            .unwrap()
            .apply(&before)
            .unwrap();
        assert!(!after.close_to_tray);
        assert!(
            ConfigPatch::parse(json!({"log_level":"debug"}))
                .unwrap()
                .apply(&before)
                .is_err()
        );
    }
}
