//! 规则命中解释：用控制器里的规则表与当前代理图景，离线回答
//! 「这个域名会命中哪条规则、进哪个组、最后由谁出去」。
//!
//! 只做能确定回答的部分：域名类规则可以精确匹配；GEOSITE / GEOIP /
//! RULE-SET / 进程类规则依赖内核数据或连接元信息，这里如实标注「跳过」，
//! 不用猜测冒充结论。最终出口取当前 `now` 链，与内核实际选择一致。

use super::api::{Node, Rule};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Match {
    /// 1-based 行号，与规则页表格一致。
    pub index: usize,
    pub kind: String,
    pub payload: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    pub index: usize,
    pub kind: String,
    pub reason: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Explanation {
    pub host: String,
    pub matched: Option<Match>,
    /// 命中之前被跳过的规则；UI 只展示摘要，避免刷屏。
    pub skipped: Vec<Skipped>,
    pub skipped_count: usize,
    /// 从目标开始的选择链，例如 ["GLOBAL", "自动选择", "US-01"]。
    pub chain: Vec<String>,
    /// 末级节点的控制器名；DIRECT/REJECT 或分组未选择时为 None。
    pub final_node: Option<String>,
    /// 末级节点协议或内置策略名。
    pub final_type: Option<String>,
    /// 命中的是兜底 Match。
    pub fallback: bool,
    /// 规则表里没有 Match：内核会按 DIRECT 处理未命中的流量。
    pub unrouted: bool,
}

const SKIP_GEOSITE: &str = "需要内核加载的 geosite 数据";
const SKIP_GEOIP: &str = "需要解析后的 IP 才能判断";
const SKIP_RULESET: &str = "规则集内容不在控制器响应里";
const SKIP_REGEX: &str = "正则规则由内核判定，这里不模拟";
const SKIP_PROCESS: &str = "进程规则与域名无关";
const SKIP_COMPOSITE: &str = "复合规则由内核组合判定";
const SKIP_META: &str = "连接元信息在提交流程里才能确定";

/// 把输入整理成可匹配的主机名。接受 URL、带端口、IPv6 方括号形式。
pub fn normalize_host(raw: &str) -> Option<String> {
    let mut text = raw.trim().trim_matches('"').trim_matches('\'');
    if let Some((_, rest)) = text.split_once("://") {
        text = rest;
    }
    // 去掉路径、查询与片段，只保留 authority。
    if let Some(index) = text.find(['/', '?', '#']) {
        text = &text[..index];
    }
    if let Some(at) = text.rfind('@') {
        text = &text[at + 1..];
    }
    // 方括号 IPv6：[addr]:port 或 [addr]
    if text.starts_with('[') {
        let end = text.find(']')?;
        text = &text[1..end];
    } else if text.matches(':').count() == 1 {
        // host:port；IPv6 至少有 2 个冒号，不会被误切。
        if let Some((host, port)) = text.rsplit_once(':')
            && !host.is_empty()
            && port.chars().all(|c| c.is_ascii_digit())
        {
            text = host;
        }
    }
    let host = text.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || host.len() > 255 {
        return None;
    }
    if host
        .chars()
        .any(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | ':' | '_')))
    {
        return None;
    }
    Some(host)
}

pub fn explain(rules: &[Rule], nodes: &BTreeMap<String, Node>, host: &str) -> Explanation {
    let ip = host.parse::<IpAddr>().ok();
    let mut skipped = Vec::new();
    let mut matched = None;
    for (offset, rule) in rules.iter().enumerate() {
        let index = offset + 1;
        let kind = normalize_kind(&rule.kind);
        let payload = rule.pattern.trim();
        let target = rule.target.trim().to_string();
        match kind.as_str() {
            "match" => {
                matched = Some(Match {
                    index,
                    kind: rule.kind.clone(),
                    payload: payload.to_string(),
                    target,
                });
                break;
            }
            "domain" => {
                if host.eq_ignore_ascii_case(payload) {
                    matched = Some(hit(index, rule, payload, target));
                    break;
                }
            }
            "domainsuffix" => {
                let suffix = payload.trim_start_matches('.').to_ascii_lowercase();
                if host == suffix || host.ends_with(&format!(".{suffix}")) {
                    matched = Some(hit(index, rule, payload, target));
                    break;
                }
            }
            "domainkeyword" => {
                let keyword = payload.to_ascii_lowercase();
                if !keyword.is_empty() && host.contains(&keyword) {
                    matched = Some(hit(index, rule, payload, target));
                    break;
                }
            }
            "ipcidr" | "ipcidr6" => match ip.and_then(|ip| cidr_contains(payload, ip)) {
                Some(true) => {
                    matched = Some(hit(index, rule, payload, target));
                    break;
                }
                Some(false) => {}
                // 域名连接在命中 IP 规则前需要先解析；这里不替内核猜地址。
                None => skipped.push(Skipped {
                    index,
                    kind: rule.kind.clone(),
                    reason: SKIP_GEOIP,
                }),
            },
            "geosite" => skipped.push(Skipped {
                index,
                kind: rule.kind.clone(),
                reason: SKIP_GEOSITE,
            }),
            "geoip" => skipped.push(Skipped {
                index,
                kind: rule.kind.clone(),
                reason: SKIP_GEOIP,
            }),
            "ruleset" | "ruleprovider" => skipped.push(Skipped {
                index,
                kind: rule.kind.clone(),
                reason: SKIP_RULESET,
            }),
            "domainregex" => skipped.push(Skipped {
                index,
                kind: rule.kind.clone(),
                reason: SKIP_REGEX,
            }),
            "processname" | "processpath" | "processpathregex" => skipped.push(Skipped {
                index,
                kind: rule.kind.clone(),
                reason: SKIP_PROCESS,
            }),
            "and" | "or" | "not" | "subrule" => skipped.push(Skipped {
                index,
                kind: rule.kind.clone(),
                reason: SKIP_COMPOSITE,
            }),
            _ => skipped.push(Skipped {
                index,
                kind: rule.kind.clone(),
                reason: SKIP_META,
            }),
        }
    }
    // 只保留命中之前最近的若干条，完整列表对用户没有价值。
    let skipped_count = skipped.len();
    if skipped.len() > 8 {
        skipped = skipped.split_off(skipped.len() - 8);
    }
    let (chain, final_node, final_type) = match matched.as_ref() {
        Some(hit) => resolve(nodes, &hit.target),
        None => (Vec::new(), None, None),
    };
    Explanation {
        host: host.to_string(),
        fallback: matched
            .as_ref()
            .is_some_and(|hit| normalize_kind(&hit.kind) == "match"),
        unrouted: matched.is_none(),
        matched,
        skipped,
        skipped_count,
        chain,
        final_node,
        final_type,
    }
}

fn hit(index: usize, rule: &Rule, payload: &str, target: String) -> Match {
    Match {
        index,
        kind: rule.kind.clone(),
        payload: payload.to_string(),
        target,
    }
}

fn normalize_kind(kind: &str) -> String {
    kind.chars()
        .filter(|c| !matches!(c, '-' | '_' | ' '))
        .collect::<String>()
        .to_ascii_lowercase()
}

fn cidr_contains(cidr: &str, ip: IpAddr) -> Option<bool> {
    let (address, prefix) = cidr.split_once('/')?;
    let base: IpAddr = address.trim().parse().ok()?;
    let prefix: u8 = prefix.trim().parse().ok()?;
    match (base, ip) {
        (IpAddr::V4(base), IpAddr::V4(ip)) if prefix <= 32 => {
            let mask = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - u32::from(prefix))
            };
            Some(u32::from(base) & mask == u32::from(ip) & mask)
        }
        (IpAddr::V6(base), IpAddr::V6(ip)) if prefix <= 128 => {
            let mask = if prefix == 0 {
                0
            } else {
                u128::MAX << (128 - u32::from(prefix))
            };
            Some(u128::from(base) & mask == u128::from(ip) & mask)
        }
        _ => None,
    }
}

/// 顺着 now 找到真正出口；遇到环、缺失或未选择就停在当前层。
fn resolve(
    nodes: &BTreeMap<String, Node>,
    start: &str,
) -> (Vec<String>, Option<String>, Option<String>) {
    // 内置策略本身就是终点，不要求控制器一定把 DIRECT/REJECT 列进 proxies。
    if builtin_name(start) {
        return (vec![start.to_string()], None, Some(start.to_string()));
    }
    let mut chain = Vec::new();
    let mut seen = BTreeSet::new();
    let mut current = start.to_string();
    loop {
        if !seen.insert(current.clone()) {
            break;
        }
        let Some(node) = nodes.get(&current) else {
            chain.push(current);
            break;
        };
        let protocol = node.protocol.clone();
        let is_group = group_type(&protocol);
        chain.push(current.clone());
        if is_builtin(&protocol) {
            return (chain, None, Some(protocol));
        }
        match node.selected.as_deref() {
            Some(next) if !next.is_empty() => current = next.to_string(),
            _ => {
                // 叶子节点：分组没有手动选择时停在分组本身，不虚构出口。
                return (chain, (!is_group).then_some(current), Some(protocol));
            }
        }
    }
    (chain, None, None)
}

fn is_builtin(protocol: &str) -> bool {
    builtin_name(protocol)
        || matches!(
            protocol.to_ascii_lowercase().as_str(),
            "rejectdrop" | "reject-drop"
        )
}

fn builtin_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "direct" | "reject" | "reject-drop" | "pass" | "compatible"
    )
}

fn group_type(protocol: &str) -> bool {
    matches!(
        protocol.to_ascii_lowercase().as_str(),
        "selector" | "urltest" | "fallback" | "loadbalance" | "relay"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(kind: &str, pattern: &str, target: &str) -> Rule {
        Rule {
            kind: kind.into(),
            pattern: pattern.into(),
            target: target.into(),
            size: 0,
        }
    }

    fn nodes() -> BTreeMap<String, Node> {
        let mut map = BTreeMap::new();
        map.insert(
            "GLOBAL".into(),
            Node {
                id: "GLOBAL".into(),
                protocol: "Selector".into(),
                udp: None,
                samples: vec![],
                selected: Some("自动选择".into()),
                choices: vec!["自动选择".into()],
            },
        );
        map.insert(
            "自动选择".into(),
            Node {
                id: "自动选择".into(),
                protocol: "URLTest".into(),
                udp: None,
                samples: vec![],
                selected: Some("US-01".into()),
                choices: vec!["US-01".into()],
            },
        );
        map.insert(
            "US-01".into(),
            Node {
                id: "US-01".into(),
                protocol: "Trojan".into(),
                udp: Some(true),
                samples: vec![],
                selected: None,
                choices: vec![],
            },
        );
        map
    }

    #[test]
    fn normalizes_urls_ports_and_ipv6() {
        assert_eq!(
            normalize_host("https://claude.ai/login?x=1").as_deref(),
            Some("claude.ai")
        );
        assert_eq!(
            normalize_host("CLAUDE.AI:443").as_deref(),
            Some("claude.ai")
        );
        assert_eq!(
            normalize_host("[2606:4700::1]:8443").as_deref(),
            Some("2606:4700::1")
        );
        assert_eq!(normalize_host("claude.ai.").as_deref(), Some("claude.ai"));
        assert!(normalize_host("not a host").is_none());
        assert!(normalize_host("").is_none());
    }

    #[test]
    fn suffix_and_keyword_hits_resolve_the_chain() {
        let rules = vec![
            rule("GeoSite", "cn", "DIRECT"),
            rule("DomainSuffix", "claude.ai", "GLOBAL"),
            rule("Match", "", "GLOBAL"),
        ];
        let explanation = explain(&rules, &nodes(), "api.claude.ai");
        let hit = explanation.matched.unwrap();
        assert_eq!(hit.index, 2);
        assert_eq!(hit.target, "GLOBAL");
        assert_eq!(explanation.skipped_count, 1);
        assert_eq!(explanation.chain, ["GLOBAL", "自动选择", "US-01"]);
        assert_eq!(explanation.final_node.as_deref(), Some("US-01"));
        assert_eq!(explanation.final_type.as_deref(), Some("Trojan"));
        assert!(!explanation.fallback);
    }

    #[test]
    fn fallback_and_unrouted_are_distinguished() {
        let rules = vec![rule("Match", "", "REJECT")];
        let explanation = explain(&rules, &nodes(), "example.test");
        assert!(explanation.fallback);
        assert_eq!(explanation.final_type.as_deref(), Some("REJECT"));
        assert!(explanation.final_node.is_none());

        let rules = vec![rule("Domain", "a.test", "GLOBAL")];
        let explanation = explain(&rules, &nodes(), "b.test");
        assert!(explanation.unrouted);
        assert!(explanation.matched.is_none());
        assert!(explanation.chain.is_empty());
    }

    #[test]
    fn ip_rules_match_literal_addresses_only() {
        let rules = vec![
            rule("IPCIDR", "203.0.113.0/24", "GLOBAL"),
            rule("Match", "", "DIRECT"),
        ];
        let hit = explain(&rules, &nodes(), "203.0.113.7").matched.unwrap();
        assert_eq!(hit.index, 1);
        let skipped = explain(&rules, &nodes(), "claude.ai");
        assert_eq!(skipped.skipped_count, 1);
        assert_eq!(skipped.matched.unwrap().index, 2);
    }

    #[test]
    fn group_without_selection_does_not_invent_an_exit() {
        let mut map = nodes();
        map.get_mut("GLOBAL").unwrap().selected = None;
        let rules = vec![rule("Match", "", "GLOBAL")];
        let explanation = explain(&rules, &map, "x.test");
        assert_eq!(explanation.chain, ["GLOBAL"]);
        assert!(explanation.final_node.is_none());
    }

    #[test]
    fn cycles_stop_instead_of_looping() {
        let mut map = nodes();
        map.get_mut("GLOBAL").unwrap().selected = Some("自动选择".into());
        map.get_mut("自动选择").unwrap().selected = Some("GLOBAL".into());
        let (chain, final_node, _) = resolve(&map, "GLOBAL");
        assert_eq!(chain, ["GLOBAL", "自动选择"]);
        assert!(final_node.is_none());
    }
}
