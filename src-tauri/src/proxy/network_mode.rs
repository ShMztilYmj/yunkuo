//! 网络模式只覆盖运行配置，订阅原文件保持不变。
use crate::config::AppConfig;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};

/// 日常分流直接使用内核全局选择器：不再额外建组，规则与 DNS 都指向 GLOBAL。
pub const GLOBAL_GROUP: &str = "GLOBAL";
#[allow(
    dead_code,
    reason = "共享网络策略：备份文件仅由主程序管理，安全服务不读写用户备份"
)]
pub const BACKUP_FILE: &str = "network-mode-backup.yaml";
pub(crate) const AI_DOMAINS: &[&str] = &[
    "claude.ai",
    "claude.com",
    "anthropic.com",
    "openai.com",
    "chatgpt.com",
    "oaistatic.com",
    "oaiusercontent.com",
    "grok.com",
    "x.ai",
    "gemini.google.com",
    "generativelanguage.googleapis.com",
];
/// 日常分流的规则表以 AI 规则开头（每条域名生成 UDP 拒绝 + 域名分流两条）。
/// 覆写插在 AI 规则之后：用户规则能生效，又不会把 AI 流量挤回直连。
pub(crate) const AI_RULE_COUNT: usize = AI_DOMAINS.len() * 2;

/// 覆写里的 DNS 策略不能改写 AI 域名的解析路径。
pub(crate) fn ai_owned_domain(policy_key: &str) -> bool {
    let key = policy_key
        .trim()
        .trim_start_matches("+.")
        .trim_start_matches('.')
        .to_ascii_lowercase();
    AI_DOMAINS.iter().any(|domain| {
        key == *domain
            || key.ends_with(&format!(".{domain}"))
            || domain.ends_with(&format!(".{key}"))
    })
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMode {
    Daily,
    Strict,
}

/// Three kernel routing choices, separate from software-managed network plans.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoutingMode {
    Global,
    Rule,
    Direct,
}

impl RoutingMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Rule => "rule",
            Self::Direct => "direct",
        }
    }
}

pub fn routing_target(
    before: &AppConfig,
    mode: RoutingMode,
    armed: bool,
    allow_release: bool,
) -> Result<AppConfig> {
    if armed && !allow_release {
        bail!("切换运行模式将解除严格保护，请先关闭 AI 客户端后确认");
    }
    let mut target = before.clone();
    target.network_mode = None;
    target.daily_proxy = None;
    target.mode = mode.as_str().into();
    Ok(target)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModeBackup {
    pub config: AppConfig,
    pub armed: bool,
    pub core_running: bool,
    pub created_at: String,
}

impl ModeBackup {
    /// 只还原此功能管理的字段，避免撤销后来修改的快捷方式、端口或其他偏好。
    pub fn restore_into(&self, current: &AppConfig) -> AppConfig {
        let mut cfg = current.clone();
        cfg.network_mode = self.config.network_mode;
        cfg.daily_proxy = self.config.daily_proxy.clone();
        cfg.mode = self.config.mode.clone();
        cfg.enable_tun = self.config.enable_tun;
        cfg.enable_system_proxy = self.config.enable_system_proxy;
        cfg.ipv6 = self.config.ipv6;
        cfg.allow_lan = self.config.allow_lan;
        // 旧备份可能来自「日常分流开着 IPv6」的版本；方案校验不允许这个组合。
        cfg.normalize_managed_ipv6();
        cfg
    }
}

pub fn proxy_type(kind: &str) -> bool {
    matches!(
        kind.to_ascii_lowercase().as_str(),
        "ss" | "shadowsocks"
            | "ssr"
            | "shadowsocksr"
            | "vmess"
            | "vless"
            | "trojan"
            | "hysteria"
            | "hysteria2"
            | "tuic"
            | "wireguard"
            | "socks5"
            | "http"
            | "snell"
            | "mieru"
            | "anytls"
            | "ssh"
    )
}

/// 机场常在 `proxies` 里塞“信息节点”（名字本身就是文案），它们不是真出口。
/// 这些项的协议类型通常是 ss/vmess，光靠类型过滤拦不住，只能按名字剔除。
const INFO_NODE_MARKERS: &[&str] = &[
    "剩余流量",
    "流量",
    "套餐",
    "到期",
    "过期",
    "过滤掉",
    "已过滤",
    "距离下次",
    "重置",
    "官网",
    "网址",
    "购买",
    "续费",
    "客服",
    "公告",
    "traffic",
    "expire",
    "expiry",
    "remaining",
];

/// 与 INFO_NODE_MARKERS 同义的 Go 正则，供内核过滤 provider 里的同类节点。
pub const INFO_NODE_FILTER: &str = "(?i)(剩余流量|流量|套餐|到期|过期|过滤掉|已过滤|距离下次|重置|官网|网址|购买|续费|客服|公告|traffic|expire|expiry|remaining)";

pub fn info_node(name: &str) -> bool {
    let lower = name.to_lowercase();
    INFO_NODE_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

pub fn selected_proxy(payload: &serde_json::Value, start: &str) -> Option<String> {
    let mut name = start;
    let mut seen = std::collections::BTreeSet::new();
    while seen.insert(name) {
        let node = &payload["proxies"][name];
        if let Some(next) = node["now"].as_str() {
            name = next;
            continue;
        }
        return node["type"]
            .as_str()
            .filter(|kind| proxy_type(kind))
            .map(|_| name.to_string());
    }
    None
}

fn local_rule(rule: &str) -> bool {
    let parts: Vec<_> = rule.split(',').map(str::trim).collect();
    if parts.len() < 3 || parts[0] != "IP-CIDR" || parts[2] != "DIRECT" {
        return false;
    }
    let Some((ip, prefix)) = parts[1].split_once('/') else {
        return false;
    };
    let (Ok(ip), Ok(prefix)) = (ip.parse::<std::net::Ipv4Addr>(), prefix.parse::<u8>()) else {
        return false;
    };
    let octets = ip.octets();
    prefix <= 32
        && ((octets[0] == 10 && prefix >= 8)
            || (octets[0] == 172 && (16..=31).contains(&octets[1]) && prefix >= 12)
            || (octets[0] == 192 && octets[1] == 168 && prefix >= 16)
            || (octets[0] == 127 && prefix >= 8)
            || (octets[0] == 169 && octets[1] == 254 && prefix >= 16))
}

pub fn daily(map: &mut Mapping, preferred: Option<&str>) -> Result<()> {
    // 同名 GLOBAL 组会被下面的定义整条替换，只拦节点占用内置名。
    for key in ["proxies", "proxy-groups"] {
        for node in map
            .get(Value::String(key.into()))
            .and_then(Value::as_sequence)
            .into_iter()
            .flatten()
        {
            let Some(name) = node["name"].as_str() else {
                continue;
            };
            if key == "proxy-groups" && name.eq_ignore_ascii_case(GLOBAL_GROUP) {
                continue;
            }
            if [
                GLOBAL_GROUP,
                "DIRECT",
                "REJECT",
                "REJECT-DROP",
                "PASS",
                "COMPATIBLE",
            ]
            .iter()
            .any(|reserved| name.eq_ignore_ascii_case(reserved))
            {
                bail!("订阅使用了保留名称「{name}」，请先重命名该项");
            }
        }
    }
    let nodes: Vec<String> = map
        .get(Value::String("proxies".into()))
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .filter(|node| node["type"].as_str().is_some_and(proxy_type))
        .filter_map(|node| node["name"].as_str())
        .filter(|name| !info_node(name))
        .filter(|name| {
            ![
                "DIRECT",
                "REJECT",
                "REJECT-DROP",
                "PASS",
                "COMPATIBLE",
                "GLOBAL",
            ]
            .iter()
            .any(|built_in| name.eq_ignore_ascii_case(built_in))
        })
        .map(str::to_string)
        .collect();
    let providers: Vec<String> = map
        .get(Value::String("proxy-providers".into()))
        .and_then(Value::as_mapping)
        .into_iter()
        .flatten()
        .filter_map(|(name, _)| name.as_str().map(str::to_string))
        .collect();
    if nodes.is_empty() && providers.is_empty() {
        bail!("日常分流需要代理节点，请先导入含有效节点的订阅");
    }
    let original_lan: Vec<String> = map
        .get(Value::String("rules".into()))
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|rule| local_rule(rule))
        .map(str::to_string)
        .collect();

    // 使用已有严格 TUN 基线，但日常模式允许国内业务经 DIRECT 出站。
    crate::privacy::harden(map)?;
    let mut group_nodes = nodes;
    if let Some(index) = preferred.and_then(|name| group_nodes.iter().position(|n| n == name)) {
        let selected = group_nodes.remove(index);
        group_nodes.insert(0, selected);
    }
    group_nodes.push("REJECT".into()); // provider 未加载或没有可用节点时，不能落到 COMPATIBLE/DIRECT。
    let global = serde_yaml::to_value(serde_json::json!({
        "name": GLOBAL_GROUP, "type": "select", "proxies": group_nodes, "use": providers,
        "exclude-type": "Direct|Compatible|Pass",
        // provider 里的节点不过 proxies 表，名字过滤要再交给内核做一遍。
        "exclude-filter": INFO_NODE_FILTER
    }))?;
    let groups = map
        .entry(Value::String("proxy-groups".into()))
        .or_insert_with(|| Value::Sequence(vec![]))
        .as_sequence_mut()
        .ok_or_else(|| anyhow::anyhow!("订阅 proxy-groups 必须是列表"))?;
    // 订阅自带的 GLOBAL 就地替换，避免内核加载重复组名。
    groups.retain(|group| group["name"].as_str() != Some(GLOBAL_GROUP));
    groups.insert(0, global);
    // AI 的 UDP 先拒绝，避免不支持 UDP 的节点导致流量落入国内或局域网直连。
    let mut rules: Vec<Value> = AI_DOMAINS
        .iter()
        .flat_map(|domain| {
            [
                format!("AND,((NETWORK,udp),(DOMAIN-SUFFIX,{domain})),REJECT").into(),
                format!("DOMAIN-SUFFIX,{domain},{GLOBAL_GROUP}").into(),
            ]
        })
        .collect();
    rules.extend(original_lan.into_iter().map(Value::String));
    for network in [
        "127.0.0.0/8",
        "10.0.0.0/8",
        "172.16.0.0/12",
        "192.168.0.0/16",
        "169.254.0.0/16",
    ] {
        rules.push(format!("IP-CIDR,{network},DIRECT,no-resolve").into());
    }
    for network in ["::1/128", "fc00::/7", "fe80::/10"] {
        rules.push(format!("IP-CIDR6,{network},DIRECT,no-resolve").into());
    }
    rules.extend([
        "GEOSITE,CN,DIRECT".into(),
        "GEOIP,CN,DIRECT,no-resolve".into(),
        format!("MATCH,{GLOBAL_GROUP}").into(),
        "MATCH,REJECT".into(),
    ]);
    map.insert("rules".into(), Value::Sequence(rules));
    map.insert("mode".into(), "rule".into());
    map.insert("geodata-mode".into(), false.into());
    // IPv6 开关只关内核解析与出站；TUN 的 v6 地址单独写，和开关取值无关。
    map.insert("ipv6".into(), false.into());
    map.get_mut(Value::String("tun".into())).unwrap()["inet6-address"] =
        serde_yaml::to_value(["fdfe:dcba:9876::1/126"])?;
    let dns = map
        .get_mut(Value::String("dns".into()))
        .unwrap()
        .as_mapping_mut()
        .unwrap();
    let overseas = vec![
        format!("https://1.1.1.1/dns-query#{GLOBAL_GROUP}"),
        format!("https://1.0.0.1/dns-query#{GLOBAL_GROUP}"),
    ];
    let domestic = vec![
        "https://223.5.5.5/dns-query#DIRECT",
        "https://223.6.6.6/dns-query#DIRECT",
    ];
    dns.insert("nameserver".into(), serde_yaml::to_value(&overseas)?);
    dns.insert("direct-nameserver".into(), serde_yaml::to_value(&domestic)?);
    let mut policy = Mapping::new();
    for domain in AI_DOMAINS {
        policy.insert(
            format!("+.{domain}").into(),
            serde_yaml::to_value(&overseas)?,
        );
    }
    policy.insert("geosite:cn".into(), serde_yaml::to_value(&domestic)?);
    dns.insert("nameserver-policy".into(), Value::Mapping(policy));
    // 节点域名必须由国内直连解析：harden 留下的 1.1.1.1 DoH 在部分线路直连不通，
    // 节点域名解析失败后连测速都无法建立连接。这里只解析节点服务器，不参与业务解析。
    dns.insert(
        "proxy-server-nameserver".into(),
        serde_yaml::to_value(["https://223.5.5.5/dns-query", "https://223.6.6.6/dns-query"])?,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routing_choices_require_confirmation_and_preserve_network_switches() {
        let before = AppConfig {
            network_mode: Some(NetworkMode::Strict),
            daily_proxy: Some("US".into()),
            mode: "global".into(),
            enable_tun: true,
            enable_system_proxy: true,
            mixed_port: 9000,
            claude_shortcut: "keep.lnk".into(),
            ..Default::default()
        };
        for mode in [RoutingMode::Global, RoutingMode::Rule, RoutingMode::Direct] {
            assert!(routing_target(&before, mode, true, false).is_err());
            let target = routing_target(&before, mode, true, true).unwrap();
            assert_eq!(target.mode, mode.as_str());
            assert!(target.network_mode.is_none() && target.daily_proxy.is_none());
            assert!(target.enable_tun && target.enable_system_proxy);
            assert_eq!(target.mixed_port, 9000);
            assert_eq!(target.claude_shortcut, "keep.lnk");
            assert_eq!(before.network_mode, Some(NetworkMode::Strict));
        }
        assert!(serde_json::from_str::<RoutingMode>("\"invalid\"").is_err());
    }
    fn fixture() -> Mapping {
        serde_yaml::from_str("proxies:\n- {name: US, type: ss, server: 203.0.113.1, port: 443, cipher: aes-128-gcm, password: fixture}\n- {name: bypass, type: direct}\nproxy-groups:\n- {name: old, type: select, proxies: [DIRECT, US]}\nrules: [MATCH,DIRECT]\n").unwrap()
    }
    #[test]
    fn daily_rebuilds_global_and_keeps_subscription_groups() {
        let mut map = fixture();
        let original_nodes = map["proxies"].clone();
        daily(&mut map, Some("US")).unwrap();
        assert_eq!(map["proxies"], original_nodes);
        assert_eq!(map["proxy-groups"].as_sequence().unwrap().len(), 2);
        let group = &map["proxy-groups"][0];
        assert_eq!(group["name"], GLOBAL_GROUP);
        assert_eq!(
            group["proxies"],
            serde_yaml::to_value(["US", "REJECT"]).unwrap()
        );
        assert_eq!(group["proxies"][0], "US");
        assert!(group.get("default-selected").is_none());
        let rules = map["rules"].as_sequence().unwrap();
        assert!(rules[0].as_str().unwrap().contains("claude.ai"));
        assert_eq!(
            rules[rules.len() - 2].as_str(),
            Some(format!("MATCH,{GLOBAL_GROUP}").as_str())
        );
        assert_eq!(rules.last().unwrap().as_str(), Some("MATCH,REJECT"));
        assert_eq!(map["ipv6"], false);
        assert_eq!(map["tun"]["inet6-address"][0], "fdfe:dcba:9876::1/126");
        assert_eq!(map["dns"]["ipv6"], false);
        assert_eq!(
            map["dns"]["nameserver"][0],
            format!("https://1.1.1.1/dns-query#{GLOBAL_GROUP}")
        );
        assert_eq!(
            map["dns"]["nameserver-policy"]["+.claude.ai"][0],
            format!("https://1.1.1.1/dns-query#{GLOBAL_GROUP}")
        );
        assert_eq!(
            map["dns"]["nameserver-policy"]["geosite:cn"][0],
            "https://223.5.5.5/dns-query#DIRECT"
        );
        // 节点域名不能被锁在直连不通的 1.1.1.1 DoH 上，否则日常分流全灭。
        assert_eq!(
            map["dns"]["proxy-server-nameserver"][0],
            "https://223.5.5.5/dns-query"
        );
        assert_eq!(
            map["dns"]["proxy-server-nameserver"][1],
            "https://223.6.6.6/dns-query"
        );
    }
    #[test]
    fn providers_have_reject_fallback_and_direct_type_exclusion() {
        let mut map: Mapping = serde_yaml::from_str(
            "proxy-providers:\n  subscription: {type: http, url: 'https://example.test/nodes'}\n",
        )
        .unwrap();
        daily(&mut map, None).unwrap();
        let group = &map["proxy-groups"][0];
        assert_eq!(group["name"], GLOBAL_GROUP);
        assert_eq!(group["proxies"][0], "REJECT");
        assert_eq!(group["use"][0], "subscription");
        assert_eq!(group["exclude-type"], "Direct|Compatible|Pass");
    }
    #[test]
    fn subscription_global_is_replaced_instead_of_duplicated() {
        let mut map: Mapping = serde_yaml::from_str(
            "proxies:\n- {name: US, type: ss, server: 203.0.113.1, port: 443, cipher: aes-128-gcm, password: fixture}\nproxy-groups:\n- {name: GLOBAL, type: select, proxies: [DIRECT]}\n- {name: keep, type: select, proxies: [US]}\n",
        )
        .unwrap();
        daily(&mut map, None).unwrap();
        let groups = map["proxy-groups"].as_sequence().unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0]["name"], GLOBAL_GROUP);
        assert_eq!(groups[0]["proxies"][0], "US");
        assert_eq!(groups[1]["name"], "keep");
    }
    #[test]
    fn info_nodes_are_kept_out_of_global() {
        let mut map: Mapping = serde_yaml::from_str(
            "proxies:\n\
             - {name: 日本-优化, type: ss, server: 203.0.113.1, port: 443, cipher: aes-128-gcm, password: fixture}\n\
             - {name: '剩余流量：64.6 GB', type: ss, server: 203.0.113.2, port: 443, cipher: aes-128-gcm, password: fixture}\n\
             - {name: '套餐到期：长期有效', type: ss, server: 203.0.113.3, port: 443, cipher: aes-128-gcm, password: fixture}\n\
             - {name: '过滤掉14条线路', type: ss, server: 203.0.113.4, port: 443, cipher: aes-128-gcm, password: fixture}\n",
        )
        .unwrap();
        daily(&mut map, Some("日本-优化")).unwrap();
        assert_eq!(
            map["proxy-groups"][0]["proxies"],
            serde_yaml::to_value(["日本-优化", "REJECT"]).unwrap()
        );
        assert!(info_node("Traffic: 12.4 GB"));
        assert!(info_node("Expire: 2027-01-01"));
        assert!(!info_node("US-01"));
        assert!(!info_node("日本-优化"));
    }
    #[test]
    fn invalid_subscription_and_reserved_names_are_rejected() {
        assert!(daily(&mut Mapping::new(), None).is_err());
        let mut map = fixture();
        map["proxies"][0]["name"] = GLOBAL_GROUP.into();
        assert!(daily(&mut map, None).is_err());
        map["proxies"][0]["name"] = "REJECT".into();
        assert!(daily(&mut map, None).is_err());
        assert!(local_rule("IP-CIDR,192.168.5.0/24,DIRECT"));
        assert!(!local_rule("IP-CIDR,192.168.0.0/0,DIRECT"));
        assert!(!local_rule("IP-CIDR,8.8.8.8/32,DIRECT"));
    }
    #[test]
    fn selected_proxy_never_inherits_direct_or_cyclic_groups() {
        let mut payload = serde_json::json!({"proxies":{"GLOBAL":{"now":"old"},"old":{"now":"US"},"US":{"type":"Shadowsocks"}}});
        assert_eq!(selected_proxy(&payload, "GLOBAL").as_deref(), Some("US"));
        payload["proxies"]["US"] = serde_json::json!({"type":"Direct"});
        assert!(selected_proxy(&payload, "GLOBAL").is_none());
        payload["proxies"]["old"]["now"] = "GLOBAL".into();
        assert!(selected_proxy(&payload, "GLOBAL").is_none());
    }
    #[test]
    fn restore_preserves_unrelated_changes_and_legacy_mode() {
        let backup = ModeBackup {
            config: AppConfig::default(),
            ..Default::default()
        };
        let mut now = AppConfig::default();
        now.network_mode = Some(NetworkMode::Daily);
        now.enable_tun = true;
        now.mixed_port = 9000;
        now.claude_shortcut = "new.lnk".into();
        let restored = backup.restore_into(&now);
        assert_eq!(restored.network_mode, None);
        assert!(!restored.enable_tun);
        assert_eq!(restored.mixed_port, 9000);
        assert_eq!(restored.claude_shortcut, "new.lnk");
    }
}
