//! 原生网络方案：只解析与编译配置，不改动系统网络，也不调用服务。
use super::network_mode::{self, NetworkMode, RoutingMode};
use crate::config::AppConfig;
use anyhow::{Result, anyhow, bail};
use serde_yaml::{Mapping, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scheme {
    Custom,
    Daily,
    Strict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DnsStrategy {
    Subscription,
    DailySplit,
    ProtectedGlobal,
}

impl DnsStrategy {
    fn for_scheme(scheme: Scheme) -> Self {
        match scheme {
            Scheme::Custom => Self::Subscription,
            Scheme::Daily => Self::DailySplit,
            Scheme::Strict => Self::ProtectedGlobal,
        }
    }
}

pub(crate) struct Plan {
    pub(crate) scheme: Scheme,
    pub(crate) routing: RoutingMode,
    pub(crate) dns: DnsStrategy,
    pub(crate) execution_policy: proxy_runtime::ExecutionPolicy,
    config: AppConfig,
}

impl Plan {
    /// 所有调用方都从同一份扁平配置推导出同一套方案。
    pub(crate) fn resolve(cfg: &AppConfig) -> Result<Self> {
        let routing = match cfg.mode.as_str() {
            "global" => RoutingMode::Global,
            "rule" => RoutingMode::Rule,
            "direct" => RoutingMode::Direct,
            _ => bail!("路由方式不合法，请选择全局、规则或直连"),
        };
        let scheme = match cfg.network_mode {
            None => Scheme::Custom,
            Some(NetworkMode::Daily) => Scheme::Daily,
            Some(NetworkMode::Strict) => Scheme::Strict,
        };
        if scheme != Scheme::Custom {
            let daily = scheme == Scheme::Daily;
            let expected = if daily {
                RoutingMode::Rule
            } else {
                RoutingMode::Global
            };
            if routing != expected || cfg.ipv6 || cfg.allow_lan {
                bail!(
                    "网络方案固定关闭 IPv6 并统一管理路由与监听范围，请重新应用方案或先切换自定义"
                );
            }
            if !cfg.enable_tun || !cfg.enable_system_proxy {
                bail!(
                    "日常分流与严格保护要求 TUN 和系统代理同时开启，请先切换自定义再调整接管开关"
                );
            }
        }
        Ok(Self {
            scheme,
            routing,
            dns: DnsStrategy::for_scheme(scheme),
            execution_policy: if scheme == Scheme::Strict {
                proxy_runtime::ExecutionPolicy::Protected
            } else {
                proxy_runtime::ExecutionPolicy::Standard
            },
            config: cfg.clone(),
        })
    }

    /// 订阅只解析一次；所有改动落在同一棵树上，最后统一校验并序列化。
    #[allow(dead_code, reason = "单元测试直接验证方案编译，不经过覆写")]
    pub(crate) fn compile(&self, subscription: Option<&str>) -> Result<String> {
        self.compile_with(subscription, &super::overlay::Overlay::default())
    }

    /// 覆写只改变规则、分组与 DNS 策略；其余字段仍由方案与订阅共同决定。
    pub(crate) fn compile_with(
        &self,
        subscription: Option<&str>,
        overlay: &super::overlay::Overlay,
    ) -> Result<String> {
        if self.scheme == Scheme::Strict && !overlay.is_empty() {
            bail!("严格保护不接受订阅覆写，请先切换到日常分流或自定义方案再保存");
        }
        let mut value: Value = match subscription {
            Some(text) => {
                serde_yaml::from_str(text).map_err(|e| anyhow!("订阅 YAML 解析失败：{e}"))?
            }
            None => Value::Mapping(Mapping::new()),
        };
        // 方案与 DNS 策略成对确定；只改其中一个会静默降级，这里直接拒绝。
        if self.dns != DnsStrategy::for_scheme(self.scheme) {
            bail!("网络方案与 DNS 策略不一致，请重新应用方案");
        }
        let cfg = &self.config;
        let map = value
            .as_mapping_mut()
            .ok_or_else(|| anyhow!("订阅配置根节点必须是映射"))?;
        overwrite_owned(map, cfg, self.routing);
        strip_foreign(map);
        // 记住上次选择：内核重启后仍回到用户手动选中的节点，而不是订阅里的第一项。
        // 订阅显式写了 false 才关闭，避免静默覆盖订阅作者的意图。
        mapping(map, "profile")?
            .entry("store-selected".into())
            .or_insert(true.into());
        match self.dns {
            DnsStrategy::DailySplit => {
                network_mode::daily(map, cfg.daily_proxy.as_deref())?;
                // AI 规则块必须留在最前面：覆写插在它之后，用户规则生效但无法绕过保护。
                super::overlay::apply(map, overlay, network_mode::AI_RULE_COUNT)?;
            }
            DnsStrategy::ProtectedGlobal => {
                crate::privacy::harden(map)?;
                protected_global(map)?;
            }
            DnsStrategy::Subscription => {
                if cfg.enable_tun {
                    custom_tun(map, cfg)?;
                } else {
                    // 关闭 TUN 时必须显式写 false，否则订阅里的 TUN 段会继续接管流量。
                    mapping(map, "tun")?.insert("enable".into(), false.into());
                }
                // 自定义配置由用户负责，覆写规则放在最前面。
                super::overlay::apply(map, overlay, 0)?;
            }
        }
        proxy_runtime::validate_and_render(&value, self.execution_policy)
    }
}

fn mapping<'a>(map: &'a mut Mapping, key: &str) -> Result<&'a mut Mapping> {
    let entry = map
        .entry(key.into())
        .or_insert_with(|| Value::Mapping(Mapping::new()));
    entry
        .as_mapping_mut()
        .ok_or_else(|| anyhow!("订阅 {key} 必须是映射"))
}

/// 订阅不可信：入口、控制器、认证与全局开关只认本程序配置。
fn overwrite_owned(map: &mut Mapping, cfg: &AppConfig, routing: RoutingMode) {
    for (key, value) in [
        ("mixed-port", Value::from(cfg.mixed_port)),
        ("external-controller", Value::from(cfg.controller_addr())),
        ("secret", Value::from(cfg.secret.clone())),
        ("mode", Value::from(routing.as_str())),
        ("log-level", Value::from(cfg.log_level.clone())),
        ("allow-lan", Value::from(cfg.allow_lan)),
        ("ipv6", Value::from(cfg.ipv6)),
        // 数据面参数同样不能由订阅决定：机场常在订阅里关掉并发拨号，节点域名有
        // 多条 A 记录时就退化成逐个 IP 串行试探，撞上不可达的 IP 要等满超时才换
        // 下一个，表现为“首次连接很慢、长连接被掐”。这三项与 FlClash 生成的配置
        // 取值一致，订阅更新也不会再被改回去。
        ("tcp-concurrent", Value::from(true)),
        ("unified-delay", Value::from(true)),
        ("keep-alive-interval", Value::from(30)),
    ] {
        map.insert(key.into(), value);
    }
}

/// 这些字段能另开监听、把域名解析交给系统，或让 TUN 静默不接管网段。
/// privacy::harden 只覆盖日常/严格方案，自定义方案也要剥一遍才不会漏。
fn strip_foreign(map: &mut Mapping) {
    for key in [
        "external-ui",
        "external-ui-url",
        "external-ui-name",
        "log-file",
        "external-controller-unix",
        "external-controller-pipe",
        "listeners",
        "hosts",
        "interface-name",
        "routing-mark",
        "authentication",
        "port",
        "socks-port",
        "redir-port",
        "tproxy-port",
    ] {
        map.remove(key);
    }
    if let Some(dns) = map.get_mut("dns").and_then(Value::as_mapping_mut) {
        // 本机 DNS 入口归应用管理，不能被订阅恢复为低位或其他冲突端口。
        dns.insert("listen".into(), crate::privacy::DNS_LISTEN_ADDR.into());
        // 系统 hosts 会越过 nameserver-policy，等于订阅能指定解析结果。
        dns.remove("use-system-hosts");
    }
    if let Some(tun) = map.get_mut("tun").and_then(Value::as_mapping_mut) {
        // 路由排除地址会让 TUN 不接管订阅指定的网段。
        for key in ["route-exclude-address", "route-address"] {
            tun.remove(key);
        }
    }
}

/// 自定义 TUN 只负责接管流量：DNS 沿用订阅，缺省才补可直连的解析器。
fn custom_tun(map: &mut Mapping, cfg: &AppConfig) -> Result<()> {
    let tun = mapping(map, "tun")?;
    for (key, value) in [
        ("enable", true.into()),
        ("stack", "mixed".into()),
        ("auto-detect-interface", true.into()),
        ("strict-route", true.into()),
        ("device", crate::privacy::TUN_NAME.into()),
        ("auto-route", true.into()),
        (
            "dns-hijack",
            serde_yaml::to_value(["any:53", "tcp://any:53"])?,
        ),
        // 接管 v6 的网卡地址：与 IPv6 开关解耦，开关只管内核解析与出站。
        (
            "inet6-address",
            serde_yaml::to_value(["fdfe:dcba:9876::1/126"])?,
        ),
    ] {
        tun.insert(key.into(), value);
    }
    let dns = mapping(map, "dns")?;
    dns.insert("enable".into(), true.into());
    dns.insert("listen".into(), crate::privacy::DNS_LISTEN_ADDR.into());
    dns.insert("ipv6".into(), cfg.ipv6.into());
    for (key, value) in [
        ("enhanced-mode", "fake-ip".into()),
        ("fake-ip-range", "198.18.0.1/16".into()),
    ] {
        dns.entry(key.into()).or_insert(value);
    }
    // 空列表在 mihomo 里等于“没有上游”，必须按缺失处理，否则 TUN 起来后解析全失败。
    fill_resolvers(
        dns,
        "nameserver",
        [
            "https://223.5.5.5/dns-query#DIRECT",
            "https://1.1.1.1/dns-query#DIRECT",
        ],
    )?;
    fill_resolvers(
        dns,
        "default-nameserver",
        ["https://223.5.5.5/dns-query", "https://1.1.1.1/dns-query"],
    )?;
    // 不注入 proxy-server-nameserver：节点与直连解析仍然沿用订阅的
    // nameserver-policy 和 fallback，新开专用解析器等于绕过它们。
    Ok(())
}

/// 字段缺失或为空列表才补默认值，订阅自身的非空解析器原样保留。
fn fill_resolvers(dns: &mut Mapping, key: &str, servers: [&str; 2]) -> Result<()> {
    let missing = dns
        .get(key)
        .is_none_or(|value| value.as_sequence().is_some_and(|rows| rows.is_empty()));
    if missing {
        dns.insert(key.into(), serde_yaml::to_value(servers)?);
    }
    Ok(())
}

/// 严格方案的 GLOBAL 不收订阅分组，也不给直连回落。
fn protected_global(map: &mut Mapping) -> Result<()> {
    let nodes: Vec<String> = map
        .get(Value::from("proxies"))
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
        .filter(|node| node["type"].as_str().is_some_and(network_mode::proxy_type))
        .filter_map(|node| node["name"].as_str())
        .filter(|name| !network_mode::info_node(name))
        .map(str::to_string)
        .collect();
    if nodes.iter().any(|name| {
        ["GLOBAL", "DIRECT", "REJECT", "PASS", "COMPATIBLE"]
            .iter()
            .any(|reserved| name.eq_ignore_ascii_case(reserved))
    }) {
        bail!("严格方案节点使用了保留名称，请先重命名节点");
    }
    let providers = map
        .get(Value::from("proxy-providers"))
        .and_then(Value::as_mapping)
        .is_some_and(|p| !p.is_empty());
    if nodes.is_empty() && !providers {
        bail!("严格保护需要代理节点，请先导入有效订阅");
    }
    let groups = map
        .entry("proxy-groups".into())
        .or_insert_with(|| Value::Sequence(vec![]))
        .as_sequence_mut()
        .ok_or_else(|| anyhow!("订阅 proxy-groups 必须是列表"))?;
    // 全局路由不需要订阅自带的分组和规则下载。
    groups.clear();
    groups.push(serde_yaml::to_value(serde_json::json!({
        "name":"GLOBAL", "type":"select", "proxies":nodes, "include-all-providers":true,
        "exclude-type":"Direct|Reject|RejectDrop|Pass|Compatible",
        "exclude-filter": network_mode::INFO_NODE_FILTER
    }))?);
    map.insert("rules".into(), serde_yaml::to_value(["MATCH,GLOBAL"])?);
    map.remove("rule-providers");
    Ok(())
}

pub(crate) fn managed_target(before: &AppConfig, mode: NetworkMode) -> AppConfig {
    let mut target = before.clone();
    target.network_mode = Some(mode);
    target.mode = if mode == NetworkMode::Daily {
        "rule"
    } else {
        "global"
    }
    .into();
    target.enable_tun = true;
    target.enable_system_proxy = true;
    // 两种方案都固定关闭 IPv6 开关（内核解析与出站）；TUN 的 v6 地址不随它变化。
    target.ipv6 = false;
    target.allow_lan = false;
    if mode == NetworkMode::Strict || before.network_mode != Some(NetworkMode::Daily) {
        target.daily_proxy = None;
    }
    target
}

#[cfg(test)]
mod tests {
    use super::*;
    const NODE: &str = "proxies:\n- {name: US, type: trojan, server: 203.0.113.10, port: 443, password: fixture}\n";
    const HOSTILE: &str = "\
mixed-port: 1
external-controller: 0.0.0.0:2
secret: weak
mode: direct
log-level: debug
allow-lan: true
ipv6: true
port: 7890
socks-port: 7891
redir-port: 7892
tproxy-port: 7893
listeners: [{name: sneak, type: socks, port: 7999}]
hosts: {'claude.ai': 127.0.0.1}
authentication: ['user:pass']
interface-name: eth9
routing-mark: 7
log-file: fixture.log
external-ui: ui
external-ui-url: 'https://example.test/ui.zip'
external-ui-name: ui
external-controller-unix: fixture.sock
external-controller-pipe: fixture-pipe
tun:
  enable: false
  route-address: [0.0.0.0/1]
  route-exclude-address: [0.0.0.0/0, '::/0']
dns:
  use-system-hosts: true
  nameserver: ['https://223.5.5.5/dns-query#DIRECT']
";

    fn compiled(plan: &Plan, subscription: &str) -> Value {
        serde_yaml::from_str(&plan.compile(Some(subscription)).unwrap()).unwrap()
    }

    #[test]
    fn overlay_is_rejected_in_strict_mode_and_inserted_after_ai_rules_in_daily() {
        let overlay = crate::proxy::overlay::parse(
            "rules:\n- DOMAIN-SUFFIX,corp.example,DIRECT\nproxy-groups:\n- {name: corp, type: select, proxies: [US]}\n",
        )
        .unwrap();
        let strict =
            Plan::resolve(&managed_target(&AppConfig::default(), NetworkMode::Strict)).unwrap();
        assert!(strict.compile_with(Some(NODE), &overlay).is_err());
        let daily =
            Plan::resolve(&managed_target(&AppConfig::default(), NetworkMode::Daily)).unwrap();
        let value: Value =
            serde_yaml::from_str(&daily.compile_with(Some(NODE), &overlay).unwrap()).unwrap();
        let rules = value["rules"].as_sequence().unwrap();
        // AI 规则块仍在最前面：覆写只能补充，不能把 AI 流量挤回直连。
        assert!(rules[0].as_str().unwrap().contains("claude.ai"));
        assert_eq!(
            rules[crate::proxy::network_mode::AI_RULE_COUNT].as_str(),
            Some("DOMAIN-SUFFIX,corp.example,DIRECT")
        );
        assert!(
            value["proxy-groups"]
                .as_sequence()
                .unwrap()
                .iter()
                .any(|group| group["name"] == "corp")
        );
    }

    #[test]
    fn overlay_rules_lead_in_custom_mode() {
        let overlay =
            crate::proxy::overlay::parse("rules:\n- DOMAIN,internal.test,DIRECT\n").unwrap();
        let plan = Plan::resolve(&AppConfig::default()).unwrap();
        let value: Value = serde_yaml::from_str(
            &plan
                .compile_with(Some(&format!("{NODE}rules: ['MATCH,DIRECT']\n")), &overlay)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            value["rules"][0].as_str(),
            Some("DOMAIN,internal.test,DIRECT")
        );
        assert_eq!(value["rules"][1].as_str(), Some("MATCH,DIRECT"));
    }

    #[test]
    fn daily_groups_keep_global_first_when_overlay_adds_one() {
        let overlay = crate::proxy::overlay::parse(
            "proxy-groups:\n- {name: corp, type: select, proxies: [US]}\n",
        )
        .unwrap();
        let plan =
            Plan::resolve(&managed_target(&AppConfig::default(), NetworkMode::Daily)).unwrap();
        let value: Value =
            serde_yaml::from_str(&plan.compile_with(Some(NODE), &overlay).unwrap()).unwrap();
        assert_eq!(value["proxy-groups"][0]["name"], "GLOBAL");
        assert_eq!(
            value["proxy-groups"].as_sequence().unwrap().last().unwrap()["name"],
            "corp"
        );
    }

    #[test]
    fn every_scheme_uses_own_listener_ports_even_with_low_subscription_dns() {
        let custom_tun = AppConfig {
            enable_tun: true,
            ..Default::default()
        };
        let subscription =
            format!("{NODE}dns:\n  enable: true\n  listen: 0.0.0.0:53\n  nameserver: [1.1.1.1]\n");
        for cfg in [
            AppConfig::default(),
            custom_tun,
            managed_target(&AppConfig::default(), NetworkMode::Daily),
            managed_target(&AppConfig::default(), NetworkMode::Strict),
        ] {
            let value = compiled(&Plan::resolve(&cfg).unwrap(), &subscription);
            assert_eq!(
                value["mixed-port"],
                u64::from(crate::config::DEFAULT_MIXED_PORT)
            );
            assert_eq!(value["external-controller"], "127.0.0.1:19097");
            assert_eq!(value["dns"]["listen"], "127.0.0.1:11053");
        }
    }

    #[test]
    fn both_managed_schemes_turn_the_kernel_ipv6_switch_off() {
        for mode in [NetworkMode::Daily, NetworkMode::Strict] {
            let cfg = managed_target(&AppConfig::default(), mode);
            assert!(!cfg.ipv6, "{mode:?}");
            // 订阅里写死 ipv6: true 也不能把方案改回来。
            let value = compiled(&Plan::resolve(&cfg).unwrap(), &format!("{NODE}{HOSTILE}"));
            assert_eq!(value["ipv6"], false, "{mode:?}");
            assert_eq!(value["dns"]["ipv6"], false, "{mode:?}");
        }
        // TUN 的 v6 地址与开关解耦：日常分流和自定义都写固定地址，严格保护由 WFP 拦 v6 故不写。
        let daily = compiled(
            &Plan::resolve(&managed_target(&AppConfig::default(), NetworkMode::Daily)).unwrap(),
            NODE,
        );
        assert_eq!(daily["tun"]["inet6-address"][0], "fdfe:dcba:9876::1/126");
        let strict = compiled(
            &Plan::resolve(&managed_target(&AppConfig::default(), NetworkMode::Strict)).unwrap(),
            NODE,
        );
        assert!(strict["tun"].get("inet6-address").is_none());
        // 自定义方案：开关只决定解析与出站，地址照写，开与关都一样。
        for ipv6 in [true, false] {
            let custom = compiled(
                &Plan::resolve(&AppConfig {
                    enable_tun: true,
                    ipv6,
                    ..Default::default()
                })
                .unwrap(),
                NODE,
            );
            assert_eq!(custom["ipv6"], ipv6);
            assert_eq!(custom["tun"]["inet6-address"][0], "fdfe:dcba:9876::1/126");
        }
    }
    #[test]
    fn managed_fields_are_one_plan_and_cannot_be_disabled_individually() {
        for mode in [NetworkMode::Daily, NetworkMode::Strict] {
            let cfg = managed_target(&AppConfig::default(), mode);
            assert!(Plan::resolve(&cfg).is_ok());
            for field in ["tun", "proxy", "routing", "ipv6", "lan"] {
                let mut invalid = cfg.clone();
                match field {
                    "tun" => invalid.enable_tun = false,
                    "proxy" => invalid.enable_system_proxy = false,
                    "routing" => invalid.mode = "direct".into(),
                    "ipv6" => invalid.ipv6 = !cfg.ipv6,
                    _ => invalid.allow_lan = true,
                }
                assert!(Plan::resolve(&invalid).is_err(), "{mode:?}: {field}");
            }
        }
    }
    #[test]
    fn custom_tun_honors_dns_ipv6_and_lan_without_strict_global_inheritance() {
        let cfg = AppConfig {
            enable_tun: true,
            mode: "direct".into(),
            ipv6: true,
            allow_lan: true,
            ..Default::default()
        };
        let yaml = Plan::resolve(&cfg).unwrap().compile(Some("dns:\n  nameserver: ['https://223.5.5.5/dns-query#DIRECT']\n  nameserver-policy: {'+.example.test': ['https://1.1.1.1/dns-query#DIRECT']}\n")).unwrap();
        let value: Value = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(value["mode"], "direct");
        assert_eq!(value["allow-lan"], true);
        assert_eq!(value["ipv6"], true);
        assert_eq!(value["dns"]["ipv6"], true);
        assert_eq!(
            value["dns"]["nameserver"][0],
            "https://223.5.5.5/dns-query#DIRECT"
        );
        assert!(value["dns"]["nameserver-policy"].is_mapping());
        assert!(!yaml.contains("#GLOBAL"));
        // TUN 的 v6 地址固定写，不随 IPv6 开关变化。
        assert_eq!(value["tun"]["inet6-address"][0], "fdfe:dcba:9876::1/126");
        let off = compiled(
            &Plan::resolve(&AppConfig {
                ipv6: false,
                ..cfg.clone()
            })
            .unwrap(),
            "tun:\n  inet6-address: ['fdfe:dcba:9876::1/126']\n",
        );
        assert_eq!(off["tun"]["inet6-address"][0], "fdfe:dcba:9876::1/126");
    }
    #[test]
    fn custom_tun_preserves_subscription_node_dns_inheritance() {
        let cfg = AppConfig {
            enable_tun: true,
            ..Default::default()
        };
        let subscription = "dns:\n  nameserver: ['https://223.5.5.5/dns-query']\n  default-nameserver: [223.5.5.5]\n  nameserver-policy: {'proxy.fixture.invalid': ['https://resolver.fixture.invalid/dns-query']}\n";
        let original: Value = serde_yaml::from_str(subscription).unwrap();
        let plan = Plan::resolve(&cfg).unwrap();
        let output = compiled(&plan, subscription);
        for key in [
            "nameserver",
            "default-nameserver",
            "nameserver-policy",
            "proxy-server-nameserver",
            "direct-nameserver",
        ] {
            assert_eq!(
                output["dns"][key], original["dns"][key],
                "TUN must preserve subscription DNS inheritance: {key}"
            );
        }
        let explicit = format!(
            "{subscription}  proxy-server-nameserver: ['https://223.6.6.6/dns-query']\n  direct-nameserver: ['https://doh.pub/dns-query']\n"
        );
        let original: Value = serde_yaml::from_str(&explicit).unwrap();
        let output = compiled(&plan, &explicit);
        for key in ["proxy-server-nameserver", "direct-nameserver"] {
            assert_eq!(output["dns"][key], original["dns"][key]);
        }
    }
    #[test]
    fn custom_tun_treats_empty_resolver_lists_as_missing() {
        let cfg = AppConfig {
            enable_tun: true,
            ..Default::default()
        };
        let plan = Plan::resolve(&cfg).unwrap();
        let value = compiled(&plan, "dns:\n  nameserver: []\n  default-nameserver: []\n");
        assert_eq!(
            value["dns"]["nameserver"][0],
            "https://223.5.5.5/dns-query#DIRECT"
        );
        assert_eq!(
            value["dns"]["default-nameserver"][0],
            "https://223.5.5.5/dns-query"
        );
        let value = compiled(
            &plan,
            "dns:\n  nameserver: ['https://resolver.fixture.invalid/dns-query']\n",
        );
        assert_eq!(
            value["dns"]["nameserver"],
            serde_yaml::to_value(["https://resolver.fixture.invalid/dns-query"]).unwrap()
        );
    }
    #[test]
    fn custom_without_tun_cannot_keep_subscription_interception() {
        let plan = Plan::resolve(&AppConfig::default()).unwrap();
        let value = compiled(&plan, "tun:\n  enable: true\n  stack: gvisor\n");
        assert_eq!(value["tun"]["enable"], false);
    }
    #[test]
    fn selection_memory_survives_restarts_unless_subscription_opts_out() {
        let plan = Plan::resolve(&AppConfig::default()).unwrap();
        // 订阅没写 profile 时补 true：重启后仍回到上次手动选择的节点。
        let value = compiled(&plan, NODE);
        assert_eq!(value["profile"]["store-selected"].as_bool(), Some(true));
        // 订阅显式声明时仍尊重订阅。
        let explicit = format!("{NODE}profile:\n  store-selected: false\n");
        assert_eq!(
            compiled(&plan, &explicit)["profile"]["store-selected"].as_bool(),
            Some(false)
        );
    }
    #[test]
    fn subscription_cannot_own_ports_ui_or_resolver_bypasses() {
        let plans = [
            Plan::resolve(&AppConfig {
                enable_tun: true,
                ..Default::default()
            })
            .unwrap(),
            Plan::resolve(&managed_target(&AppConfig::default(), NetworkMode::Daily)).unwrap(),
            Plan::resolve(&managed_target(&AppConfig::default(), NetworkMode::Strict)).unwrap(),
        ];
        for plan in plans {
            let value = compiled(&plan, &format!("{NODE}{HOSTILE}"));
            let map = value.as_mapping().unwrap();
            for key in [
                "external-ui",
                "external-ui-url",
                "external-ui-name",
                "log-file",
                "external-controller-unix",
                "external-controller-pipe",
                "listeners",
                "hosts",
                "interface-name",
                "routing-mark",
                "authentication",
                "port",
                "socks-port",
                "redir-port",
                "tproxy-port",
            ] {
                assert!(!map.contains_key(key), "{:?}: {key}", plan.scheme);
            }
            assert!(value["tun"].get("route-exclude-address").is_none());
            assert!(value["tun"].get("route-address").is_none());
            assert_eq!(
                value["mixed-port"].as_u64(),
                Some(u64::from(plan.config.mixed_port))
            );
            assert_eq!(
                value["external-controller"].as_str(),
                Some(plan.config.controller_addr().as_str())
            );
            assert_eq!(value["secret"].as_str(), Some(plan.config.secret.as_str()));
            assert_eq!(value["mode"].as_str(), Some(plan.routing.as_str()));
            assert_eq!(
                value["log-level"].as_str(),
                Some(plan.config.log_level.as_str())
            );
            assert_eq!(value["allow-lan"].as_bool(), Some(plan.config.allow_lan));
            assert_eq!(value["ipv6"].as_bool(), Some(plan.config.ipv6));
            match plan.scheme {
                // 自定义方案不经过 harden，必须同样不保留系统 hosts 回落。
                Scheme::Custom => assert!(value["dns"].get("use-system-hosts").is_none()),
                Scheme::Daily | Scheme::Strict => {
                    assert_eq!(value["dns"]["use-system-hosts"].as_bool(), Some(false))
                }
            }
        }
    }
    #[test]
    fn data_path_keys_are_owned_and_match_the_flclash_profile() {
        let cases = [
            AppConfig::default(),
            AppConfig {
                enable_tun: true,
                ..Default::default()
            },
            managed_target(&AppConfig::default(), NetworkMode::Daily),
            managed_target(&AppConfig::default(), NetworkMode::Strict),
        ];
        for cfg in cases {
            let plan = Plan::resolve(&cfg).unwrap();
            // 订阅想关掉并发拨号、改回旧拨号语义也不能得手。
            let text = plan
                .compile(Some(&format!(
                    "{NODE}tcp-concurrent: false\nunified-delay: false\nkeep-alive-interval: 1\n"
                )))
                .unwrap();
            // 覆盖而不是追加：重复键在内核里是配置错误。
            for key in ["tcp-concurrent:", "unified-delay:", "keep-alive-interval:"] {
                assert_eq!(text.matches(key).count(), 1, "{key}");
            }
            let value: Value = serde_yaml::from_str(&text).unwrap();
            assert_eq!(value["tcp-concurrent"].as_bool(), Some(true));
            assert_eq!(value["unified-delay"].as_bool(), Some(true));
            assert_eq!(value["keep-alive-interval"].as_u64(), Some(30));
        }
    }
    #[test]
    fn protected_plan_removes_direct_choices_and_uses_protected_dns() {
        let cfg = managed_target(&AppConfig::default(), NetworkMode::Strict);
        let plan = Plan::resolve(&cfg).unwrap();
        assert_eq!(
            plan.execution_policy,
            proxy_runtime::ExecutionPolicy::Protected
        );
        let text = format!(
            "{NODE}proxy-groups:\n- {{name: GLOBAL, type: select, proxies: [DIRECT, US]}}\nrule-providers:\n  offline: {{type: http, url: 'https://example.test/rules.yaml', path: rules.yaml}}\n"
        );
        let value = compiled(&plan, &text);
        let global = value["proxy-groups"]
            .as_sequence()
            .unwrap()
            .iter()
            .find(|g| g["name"] == "GLOBAL")
            .unwrap();
        assert_eq!(global["proxies"], serde_yaml::to_value(["US"]).unwrap());
        assert_eq!(
            value["dns"]["nameserver"][0],
            "https://1.1.1.1/dns-query#GLOBAL"
        );
        assert_eq!(value["mode"], "global");
        assert_eq!(value["ipv6"], false);
        assert_eq!(value["allow-lan"], false);
        assert_eq!(value["dns"]["use-system-hosts"], false);
        assert_eq!(
            value["rules"],
            serde_yaml::to_value(["MATCH,GLOBAL"]).unwrap()
        );
        assert!(value.get("rule-providers").is_none());
    }
    #[test]
    fn protected_global_drops_info_nodes_from_the_selector() {
        let plan =
            Plan::resolve(&managed_target(&AppConfig::default(), NetworkMode::Strict)).unwrap();
        let text = "proxies:\n\
            - {name: 日本-优化, type: trojan, server: 203.0.113.10, port: 443, password: fixture}\n\
            - {name: '剩余流量：64.6 GB', type: trojan, server: 203.0.113.11, port: 443, password: fixture}\n\
            - {name: '套餐到期：长期有效', type: trojan, server: 203.0.113.12, port: 443, password: fixture}\n";
        let value = compiled(&plan, text);
        let global = value["proxy-groups"]
            .as_sequence()
            .unwrap()
            .iter()
            .find(|g| g["name"] == "GLOBAL")
            .unwrap();
        assert_eq!(
            global["proxies"],
            serde_yaml::to_value(["日本-优化"]).unwrap()
        );
    }
    #[test]
    fn malformed_subscription_fails_before_runtime_or_network_operations() {
        let plan = Plan::resolve(&AppConfig::default()).unwrap();
        for text in ["[]", "profile: []", "tun: false"] {
            assert!(plan.compile(Some(text)).is_err());
        }
        let cfg = managed_target(&AppConfig::default(), NetworkMode::Strict);
        assert!(Plan::resolve(&cfg).unwrap().compile(None).is_err());
    }
}
