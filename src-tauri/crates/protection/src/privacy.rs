//! 严格保护配置与出口选择判定。网络拦截由服务中的 WFP 模块独立执行。
use anyhow::{Result, anyhow, bail};
use serde_yaml::{Mapping, Value};
use std::collections::BTreeSet;
use std::net::Ipv4Addr;

pub const TUN_NAME: &str = "云阔TUN";
pub const DNS_LISTEN_ADDR: &str = "127.0.0.1:11053";
#[allow(dead_code, reason = "共享保护策略：WFP 引导端点仅在安全服务中使用")]
pub const BOOTSTRAP_IPS: [&str; 2] = ["1.1.1.1", "1.0.0.1"];

pub fn harden(map: &mut Mapping) -> Result<()> {
    // 覆盖整个 DNS/TUN 段，订阅中的分流、路由排除和系统 DNS 不能削弱保护。
    let policy: Value = serde_yaml::from_str(&format!(
        r#"
ipv6: false
allow-lan: false
bind-address: 127.0.0.1
tun:
  enable: true
  device: 云阔TUN
  stack: mixed
  auto-detect-interface: true
  strict-route: true
  auto-route: true
  # mihomo 接管 TUN 与 DNS 劫持，退出时由内核撤销接口和路由。
  dns-hijack: [any:53, 'tcp://any:53']
dns:
  enable: true
  listen: {DNS_LISTEN_ADDR}
  ipv6: false
  enhanced-mode: fake-ip
  fake-ip-range: 198.18.0.1/16
  use-hosts: false
  use-system-hosts: false
  # 节点域名使用固定 IP 的加密 DNS 引导，业务 DNS 经 GLOBAL。
  default-nameserver: ['https://1.1.1.1/dns-query']
  proxy-server-nameserver: ['https://1.1.1.1/dns-query']
  nameserver: ['https://1.1.1.1/dns-query#GLOBAL', 'https://1.0.0.1/dns-query#GLOBAL']
  direct-nameserver: ['https://1.1.1.1/dns-query#GLOBAL', 'https://1.0.0.1/dns-query#GLOBAL']
"#
    ))?;
    for (key, value) in policy.as_mapping().unwrap() {
        map.insert(key.clone(), value.clone());
    }
    // 订阅可以定义额外的入站监听，严格模式不接受这类旁路。
    for key in [
        "listeners",
        "hosts",
        "interface-name",
        "routing-mark",
        "external-controller-unix",
        "external-controller-pipe",
    ] {
        map.remove(Value::String(key.into()));
    }
    Ok(())
}

#[allow(
    dead_code,
    reason = "共享保护策略：实际配置核验供安全服务及主程序回归测试使用"
)]
pub fn validate(config: &str) -> Result<Value> {
    let value: Value = serde_yaml::from_str(config)?;
    if value["mode"].as_str() != Some("global") {
        bail!("保护要求全局代理模式");
    }
    let mut expected = value
        .as_mapping()
        .ok_or_else(|| anyhow!("配置必须是映射"))?
        .clone();
    harden(&mut expected)?;
    for key in ["ipv6", "allow-lan", "bind-address", "tun", "dns"] {
        if value[key] != expected[Value::String(key.into())] {
            bail!("严格网络配置未生效，请重新启动代理");
        }
    }
    Ok(value)
}

/// 严格保护认可的节点类型；组、DIRECT、DNS 与未知类型都不能作为代理链终点。
fn protected_kind(kind: &str) -> bool {
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

/// 返回 GLOBAL 代理链末端真正承接流量的节点名，供体检说明使用。
pub fn selected_proxy(proxies: &serde_json::Value) -> Option<&str> {
    let mut name = "GLOBAL";
    let mut seen = BTreeSet::new();
    while seen.insert(name) {
        let node = proxies.get(name)?;
        if let Some(next) = node.get("now").and_then(|v| v.as_str()) {
            name = next;
            continue;
        }
        let kind = node.get("type").and_then(|v| v.as_str()).unwrap_or("");
        return protected_kind(kind).then_some(name);
    }
    None
}

/// 检查实际选中的整个代理链，拒绝 DIRECT、DNS、未知节点及循环引用。
pub fn proxy_selected(proxies: &serde_json::Value) -> bool {
    selected_proxy(proxies).is_some()
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[allow(dead_code, reason = "共享保护策略：WFP 端点类型仅在安全服务中使用")]
pub struct Endpoint {
    pub ip: Ipv4Addr,
    pub port: u16,
}

/// WFP consumes the actual frozen runtime graph, never mutable provider caches.
pub fn runtime_servers(value: &serde_json::Value) -> Result<Vec<(String, u16)>> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Server {
        server: String,
        port: u16,
    }
    let entries = value["endpoints"]
        .as_array()
        .ok_or_else(|| anyhow!("受保护节点快照缺失"))?;
    if entries.is_empty() || entries.len() > 512 {
        bail!("受保护固定节点数量须为 1–512");
    }
    let mut servers = BTreeSet::new();
    for entry in entries {
        let node: Server =
            serde_json::from_value(entry.clone()).map_err(|_| anyhow!("受保护节点快照格式无效"))?;
        if node.server.is_empty()
            || node.server.len() > 253
            || node.port == 0
            || node.server.chars().any(|c| {
                c.is_whitespace() || c.is_control() || ['/', '\\', '@', '?', '#'].contains(&c)
            })
        {
            bail!("受保护节点快照包含无效地址或端口");
        }
        servers.insert((node.server, node.port));
    }
    Ok(servers.into_iter().collect())
}

/// 仅从声明的节点提取服务器；不将网站域名、规则或任意 URL 加入放行名单。
#[allow(
    dead_code,
    reason = "共享保护策略：节点白名单提取供安全服务及回归测试使用"
)]
pub fn node_servers(value: &Value) -> Result<Vec<(String, u16)>> {
    let mut servers = BTreeSet::new();
    if let Some(nodes) = value["proxies"].as_sequence() {
        for node in nodes {
            let kind = node["type"].as_str().unwrap_or("");
            if matches!(
                kind.to_ascii_lowercase().as_str(),
                "direct" | "reject" | "reject-drop" | "dns"
            ) {
                continue;
            }
            let host = node["server"]
                .as_str()
                .filter(|h| !h.is_empty())
                .ok_or_else(|| anyhow!("节点缺少服务器地址"))?;
            let port = node["port"]
                .as_u64()
                .filter(|p| *p > 0 && *p <= 65535)
                .ok_or_else(|| anyhow!("严格保护仅支持具有固定 server/port 的节点"))?;
            // 端口跳跃无法用固定端口白名单约束，明确拒绝而不是扩大放行范围。
            if node.get("ports").is_some() {
                bail!("严格保护暂不支持端口跳跃节点，请选择固定端口节点");
            }
            servers.insert((host.to_string(), port as u16));
        }
    }
    Ok(servers.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_snapshot_is_the_only_fixed_endpoint_input() {
        let snapshot = serde_json::json!({"endpoints":[{"server":"proxy.example.test","port":443},{"server":"proxy.example.test","port":443}]});
        assert_eq!(
            runtime_servers(&snapshot).unwrap(),
            [("proxy.example.test".to_owned(), 443)]
        );
        for value in [
            serde_json::json!({}),
            serde_json::json!({"endpoints":[]}),
            serde_json::json!({"endpoints":[{"server":"a","port":0}]}),
            serde_json::json!({"endpoints":[{"server":"user:pass@a","port":443}]}),
            serde_json::json!({"endpoints":[{"server":"a","port":65536}]}),
            serde_json::json!({"endpoints":[{"server":"a","port":443,"ports":"1-65535"}]}),
        ] {
            assert!(runtime_servers(&value).is_err());
        }
    }
    #[test]
    fn subscription_cannot_restore_dns_or_route_bypasses() {
        let mut value: Value = serde_yaml::from_str("mode: global\nipv6: true\ntun:\n  route-exclude-address: [0.0.0.0/0]\ndns:\n  nameserver: [223.5.5.5]\n  nameserver-policy:\n    '+.claude.ai': system\n").unwrap();
        harden(value.as_mapping_mut().unwrap()).unwrap();
        assert!(validate(&serde_yaml::to_string(&value).unwrap()).is_ok());
        assert!(value["tun"].get("route-exclude-address").is_none());
        assert_eq!(value["dns"]["listen"], "127.0.0.1:11053");
        assert!(value["dns"].get("nameserver-policy").is_none());
        assert_eq!(value["tun"]["dns-hijack"].as_sequence().unwrap().len(), 2);
        value["dns"]["nameserver"] = serde_yaml::from_str("[system]").unwrap();
        assert!(validate(&serde_yaml::to_string(&value).unwrap()).is_err());
    }
    #[test]
    fn proxy_chain_rejects_direct_unknown_and_cycles() {
        let selected = serde_json::json!({"GLOBAL":{"now":"group"},"group":{"now":"node"},"node":{"type":"Trojan"}});
        assert_eq!(selected_proxy(&selected), Some("node"));
        assert!(proxy_selected(&selected));
        for proxies in [
            serde_json::json!({"GLOBAL":{"now":"DIRECT"},"DIRECT":{"type":"Direct"}}),
            serde_json::json!({"GLOBAL":{"now":"GLOBAL"}}),
            serde_json::json!({"GLOBAL":{"now":"missing"}}),
        ] {
            assert!(!proxy_selected(&proxies));
        }
    }
    #[test]
    fn endpoint_allowlist_does_not_include_rules_or_websites() {
        let value: Value = serde_yaml::from_str("proxies:\n- {name: node, type: trojan, server: 203.0.113.4, port: 443}\n- {name: DIRECT, type: direct}\nrules: [MATCH,DIRECT]\n").unwrap();
        assert_eq!(
            node_servers(&value).unwrap(),
            vec![("203.0.113.4".into(), 443)]
        );
    }
}
