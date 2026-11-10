//! 受控覆写：订阅由程序生成，用户经常只缺一条规则或一个分组。
//! 直接改订阅文件会在下次更新时被覆盖，所以单独存一份 overlay，
//! 只接受白名单字段，并且在合成后与其他配置走同一条校验。
//!
//! 覆写只做三件事：前插/插入规则、追加代理组、补充 DNS 解析策略。
//! 严格保护不接受覆写：保护依赖固定的路由与 DNS 图景，任何用户侧改写
//! 都会让 WFP 放行名单与内核实际配置脱节。

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use serde_yaml::{Mapping, Value};
use std::collections::BTreeSet;
use std::path::Path;

use super::network_mode;

pub const FILE_NAME: &str = "proxy-overlay.yaml";
/// 覆写只承载少量手工条目，超过这个体积基本可以断定不是正常用法。
const MAX_BYTES: u64 = 64 * 1024;

const ROOT_KEYS: [&str; 3] = ["rules", "proxy-groups", "dns"];
/// 只放行能被域名/网段直接判定的规则类型；MATCH 由程序统一兜底，
/// 复合规则与进程规则留给订阅作者，避免覆写把分流图景改得不可解释。
const RULE_TYPES: [&str; 15] = [
    "DOMAIN",
    "DOMAIN-SUFFIX",
    "DOMAIN-KEYWORD",
    "DOMAIN-REGEX",
    "IP-CIDR",
    "IP-CIDR6",
    "GEOSITE",
    "GEOIP",
    "RULE-SET",
    "SRC-IP-CIDR",
    "DST-PORT",
    "SRC-PORT",
    "NETWORK",
    "PROCESS-NAME",
    "PROCESS-PATH",
];
const GROUP_KEYS: [&str; 17] = [
    "name",
    "type",
    "proxies",
    "use",
    "url",
    "interval",
    "tolerance",
    "lazy",
    "timeout",
    "strategy",
    "disable-udp",
    "filter",
    "exclude-filter",
    "exclude-type",
    "include-all",
    "include-all-providers",
    "hidden",
];
const GROUP_TYPES: [&str; 5] = ["select", "url-test", "fallback", "load-balance", "relay"];
const BUILT_IN: [&str; 5] = ["DIRECT", "REJECT", "REJECT-DROP", "PASS", "COMPATIBLE"];
const RESERVED: [&str; 6] = [
    "GLOBAL",
    "DIRECT",
    "REJECT",
    "REJECT-DROP",
    "PASS",
    "COMPATIBLE",
];

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Overlay {
    pub rules: Vec<String>,
    pub groups: Vec<Value>,
    pub dns_policy: Vec<(String, Vec<String>)>,
}

impl Overlay {
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty() && self.groups.is_empty() && self.dns_policy.is_empty()
    }

    fn group_names(&self) -> BTreeSet<String> {
        self.groups
            .iter()
            .filter_map(|group| group["name"].as_str().map(str::to_owned))
            .collect()
    }
}

/// 前端只读视图：原文、条目数与分组名，保存后用同一次解析结果回答。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub text: String,
    pub rules: usize,
    pub groups: Vec<String>,
    pub dns_policy: Vec<String>,
}

impl View {
    fn of(text: String, overlay: Overlay) -> Self {
        Self {
            text,
            rules: overlay.rules.len(),
            groups: overlay
                .groups
                .iter()
                .filter_map(|group| group["name"].as_str().map(str::to_owned))
                .collect(),
            dns_policy: overlay
                .dns_policy
                .iter()
                .map(|(key, _)| key.clone())
                .collect(),
        }
    }
}

pub fn path(dir: &Path) -> std::path::PathBuf {
    dir.join(FILE_NAME)
}

/// 缺失即空覆写：首次使用不需要先创建一个空文件。
pub fn read_text(dir: &Path) -> Result<String> {
    let path = path(dir);
    if !path.is_file() {
        return Ok(String::new());
    }
    let meta = std::fs::metadata(&path).context("读取覆写文件失败")?;
    if meta.len() > MAX_BYTES {
        bail!("覆写文件超过 64 KiB，请删减后再保存");
    }
    std::fs::read_to_string(&path).context("覆写文件不是 UTF-8 文本")
}

pub fn load(dir: &Path) -> Result<Overlay> {
    parse(&read_text(dir)?)
}

/// 先校验再落盘：语法错误不应该先写坏文件再报错。
pub fn save(dir: &Path, text: &str) -> Result<Overlay> {
    let overlay = parse(text)?;
    crate::config::atomic_write(&path(dir), text.as_bytes())?;
    Ok(overlay)
}

pub fn view(dir: &Path) -> Result<View> {
    let text = read_text(dir)?;
    let overlay = parse(&text)?;
    Ok(View::of(text, overlay))
}

pub fn parse(text: &str) -> Result<Overlay> {
    if text.trim().is_empty() {
        return Ok(Overlay::default());
    }
    let value: Value = serde_yaml::from_str(text).context("覆写不是合法 YAML")?;
    // 只有注释的文件解析成 null：按空覆写处理，不强迫用户先删注释。
    if value.is_null() {
        return Ok(Overlay::default());
    }
    let map = value
        .as_mapping()
        .context("覆写根节点必须是映射（不能是列表或标量）")?;
    for key in map.keys() {
        let name = key.as_str().unwrap_or_default();
        if !ROOT_KEYS.contains(&name) {
            bail!("覆写不支持字段「{name}」，只允许 rules、proxy-groups、dns");
        }
    }
    let rules = match map.get("rules") {
        None | Some(Value::Null) => Vec::new(),
        Some(value) => value
            .as_sequence()
            .context("覆写 rules 必须是列表")?
            .iter()
            .map(|row| {
                let text = row.as_str().context("覆写 rules 的每一项必须是字符串")?;
                rule_text(text)
            })
            .collect::<Result<Vec<_>>>()?,
    };
    let groups = match map.get("proxy-groups") {
        None | Some(Value::Null) => Vec::new(),
        Some(value) => value
            .as_sequence()
            .context("覆写 proxy-groups 必须是列表")?
            .iter()
            .map(group_entry)
            .collect::<Result<Vec<_>>>()?,
    };
    let dns_policy = match map.get("dns") {
        None | Some(Value::Null) => Vec::new(),
        Some(value) => dns_policy(value)?,
    };
    let overlay = Overlay {
        rules,
        groups,
        dns_policy,
    };
    ensure_unique(&overlay)?;
    Ok(overlay)
}

fn rule_text(text: &str) -> Result<String> {
    let line = text.trim();
    if line.is_empty() || line.starts_with('#') {
        bail!("覆写规则不能为空或以 # 开头");
    }
    if line.contains('\n') || line.contains('\r') {
        bail!("一条覆写规则只能写在一行里");
    }
    let parts: Vec<&str> = line.split(',').map(str::trim).collect();
    if parts.len() < 3 {
        bail!("规则「{line}」至少需要「类型,内容,策略」三段");
    }
    let kind = parts[0].to_ascii_uppercase();
    if !RULE_TYPES.contains(&kind.as_str()) {
        bail!("覆写不支持 {kind} 规则；MATCH 与其他复合规则请交给订阅或网络方案");
    }
    if parts[1].is_empty() || parts[2].is_empty() {
        bail!("规则「{line}」的内容或策略不能为空");
    }
    for part in parts.iter().skip(1) {
        if part.chars().any(char::is_whitespace) {
            bail!("规则「{line}」不能包含空白字符");
        }
    }
    Ok(parts.join(","))
}

fn group_entry(value: &Value) -> Result<Value> {
    let map = value
        .as_mapping()
        .context("proxy-groups 的每一项必须是映射")?;
    for key in map.keys() {
        let name = key.as_str().context("proxy-groups 的键名必须是字符串")?;
        if !GROUP_KEYS.contains(&name) {
            bail!("代理组不支持字段「{name}」");
        }
    }
    let name = map
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .context("代理组缺少 name")?;
    if name.chars().count() > 64 || name.chars().any(char::is_control) {
        bail!("代理组名称「{name}」过长或包含控制字符");
    }
    if RESERVED
        .iter()
        .any(|value| name.eq_ignore_ascii_case(value))
    {
        bail!("代理组不能使用保留名称「{name}」");
    }
    let kind = map
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase)
        .context("代理组缺少 type")?;
    if !GROUP_TYPES.contains(&kind.as_str()) {
        bail!("代理组「{name}」的 type 只能是 select、url-test、fallback、load-balance 或 relay");
    }
    for key in ["proxies", "use"] {
        if let Some(value) = map.get(key) {
            let rows = value
                .as_sequence()
                .with_context(|| format!("代理组「{name}」的 {key} 必须是列表"))?;
            for row in rows {
                if row
                    .as_str()
                    .is_none_or(|item| item.trim().is_empty() || item.contains(','))
                {
                    bail!("代理组「{name}」的 {key} 必须是非空、不含逗号的名称列表");
                }
            }
        }
    }
    let has_include_all = map
        .get("include-all")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || map
            .get("include-all-providers")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    if !map.contains_key("proxies") && !map.contains_key("use") && !has_include_all {
        bail!("代理组「{name}」需要 proxies、use 或 include-all 中的至少一项");
    }
    Ok(Value::Mapping(map.clone()))
}

fn dns_policy(value: &Value) -> Result<Vec<(String, Vec<String>)>> {
    let map = value.as_mapping().context("覆写 dns 必须是映射")?;
    for key in map.keys() {
        let name = key.as_str().unwrap_or_default();
        if name != "nameserver-policy" {
            bail!("覆写 dns 只支持 nameserver-policy，不支持「{name}」");
        }
    }
    let Some(policy) = map
        .get("nameserver-policy")
        .filter(|value| !value.is_null())
    else {
        return Ok(Vec::new());
    };
    let policy = policy
        .as_mapping()
        .context("dns.nameserver-policy 必须是映射")?;
    let mut rows = Vec::with_capacity(policy.len());
    for (key, servers) in policy {
        let key = key
            .as_str()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .context("nameserver-policy 的域名键必须是字符串")?;
        if !(key.starts_with("+.") || key.starts_with('.') || key.contains('.')) {
            bail!("nameserver-policy 的键「{key}」不是有效域名");
        }
        if network_mode::ai_owned_domain(key) {
            bail!("「{key}」属于 AI 域名，解析策略由网络方案统一管理，不能在覆写里改写");
        }
        let servers = servers
            .as_sequence()
            .with_context(|| format!("nameserver-policy「{key}」的值必须是列表"))?
            .iter()
            .map(|server| {
                let text = server
                    .as_str()
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .with_context(|| format!("nameserver-policy「{key}」包含无效解析器"))?;
                if text.eq_ignore_ascii_case("system")
                    || text.to_ascii_lowercase().starts_with("dhcp://")
                {
                    bail!("覆写不接受系统/DHCP 解析器「{text}」，请填写加密 DNS 地址");
                }
                Ok(text.to_string())
            })
            .collect::<Result<Vec<_>>>()?;
        rows.push((key.to_string(), servers));
    }
    Ok(rows)
}

fn ensure_unique(overlay: &Overlay) -> Result<()> {
    let mut seen = BTreeSet::new();
    for name in overlay.group_names() {
        if !seen.insert(name.clone()) {
            bail!("覆写里代理组「{name}」重复定义");
        }
    }
    let mut keys = BTreeSet::new();
    for (key, _) in &overlay.dns_policy {
        if !keys.insert(key.to_ascii_lowercase()) {
            bail!("覆写里 DNS 策略「{key}」重复定义");
        }
    }
    Ok(())
}

/// 把覆写合成进已经过方案处理的配置。`rule_insert` 由调用方给出：
/// 自定义方案放最前（用户配置优先），日常分流放在 AI 规则之后（AI 保护优先）。
pub fn apply(map: &mut Mapping, overlay: &Overlay, rule_insert: usize) -> Result<()> {
    if overlay.is_empty() {
        return Ok(());
    }
    install_rules(map, overlay, rule_insert)?;
    install_groups(map, overlay)?;
    merge_dns(map, overlay)?;
    Ok(())
}

fn install_rules(map: &mut Mapping, overlay: &Overlay, insert: usize) -> Result<()> {
    if overlay.rules.is_empty() {
        return Ok(());
    }
    let entry = map
        .entry(Value::String("rules".into()))
        .or_insert_with(|| Value::Sequence(Vec::new()));
    let rows = entry
        .as_sequence_mut()
        .ok_or_else(|| anyhow!("订阅 rules 必须是列表"))?;
    let at = insert.min(rows.len());
    for (offset, rule) in overlay.rules.iter().enumerate() {
        rows.insert(at + offset, Value::String(rule.clone()));
    }
    Ok(())
}

fn install_groups(map: &mut Mapping, overlay: &Overlay) -> Result<()> {
    if overlay.groups.is_empty() {
        return Ok(());
    }
    let mut known: BTreeSet<String> = BUILT_IN.iter().map(|name| (*name).to_string()).collect();
    let mut providers: BTreeSet<String> = BTreeSet::new();
    if let Some(nodes) = map.get("proxies").and_then(Value::as_sequence) {
        for node in nodes {
            if node["type"].as_str().is_some_and(network_mode::proxy_type)
                && let Some(name) = node["name"].as_str()
            {
                known.insert(name.to_string());
            }
        }
    }
    if let Some(entries) = map.get("proxy-providers").and_then(Value::as_mapping) {
        for name in entries.keys().filter_map(Value::as_str) {
            providers.insert(name.to_string());
            known.insert(name.to_string());
        }
    }
    if let Some(groups) = map.get("proxy-groups").and_then(Value::as_sequence) {
        for group in groups {
            if let Some(name) = group["name"].as_str() {
                known.insert(name.to_string());
            }
        }
    }
    // 覆写组之间允许互相引用，先把名字全部登记，再逐项校验引用。
    let overlay_names = overlay.group_names();
    known.extend(overlay_names.iter().cloned());

    for group in &overlay.groups {
        let name = group["name"].as_str().unwrap_or_default();
        if let Some(values) = group.get("proxies").and_then(Value::as_sequence) {
            for value in values {
                let target = value.as_str().unwrap_or_default();
                if !known.contains(target) {
                    bail!("覆写组「{name}」引用了不存在的节点或分组「{target}」");
                }
            }
        }
        if let Some(values) = group.get("use").and_then(Value::as_sequence) {
            for value in values {
                let provider = value.as_str().unwrap_or_default();
                if !providers.contains(provider) {
                    bail!("覆写组「{name}」引用了不存在的 provider「{provider}」");
                }
            }
        }
    }

    let entry = map
        .entry(Value::String("proxy-groups".into()))
        .or_insert_with(|| Value::Sequence(Vec::new()));
    let groups = entry
        .as_sequence_mut()
        .ok_or_else(|| anyhow!("订阅 proxy-groups 必须是列表"))?;
    for group in &overlay.groups {
        let name = group["name"].as_str().unwrap_or_default();
        if groups
            .iter()
            .any(|existing| existing["name"].as_str() == Some(name))
        {
            bail!("覆写组「{name}」与订阅里已有分组重名");
        }
        groups.push(group.clone());
    }
    Ok(())
}

fn merge_dns(map: &mut Mapping, overlay: &Overlay) -> Result<()> {
    if overlay.dns_policy.is_empty() {
        return Ok(());
    }
    let entry = map
        .entry(Value::String("dns".into()))
        .or_insert_with(|| Value::Mapping(Mapping::new()));
    let dns = entry
        .as_mapping_mut()
        .ok_or_else(|| anyhow!("订阅 dns 必须是映射"))?;
    let policy = dns
        .entry(Value::String("nameserver-policy".into()))
        .or_insert_with(|| Value::Mapping(Mapping::new()))
        .as_mapping_mut()
        .ok_or_else(|| anyhow!("订阅 dns.nameserver-policy 必须是映射"))?;
    for (key, servers) in &overlay.dns_policy {
        if policy.contains_key(Value::String(key.clone())) {
            bail!("覆写 DNS 策略「{key}」与订阅里已有条目冲突");
        }
        policy.insert(Value::String(key.clone()), serde_yaml::to_value(servers)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Mapping {
        let value: Value = serde_yaml::from_str(
            "proxies:\n\
             - {name: US, type: ss, server: 203.0.113.1, port: 443, cipher: aes-128-gcm, password: fixture}\n\
             proxy-groups:\n\
             - {name: select, type: select, proxies: [US]}\n\
             rules: ['MATCH,select']\n",
        )
        .unwrap();
        value.as_mapping().unwrap().clone()
    }

    #[test]
    fn empty_or_blank_overlay_is_a_no_op() {
        assert!(parse("").unwrap().is_empty());
        assert!(parse("  \n# 注释\n").unwrap().is_empty());
        let mut map = fixture();
        apply(&mut map, &Overlay::default(), 0).unwrap();
        assert_eq!(map["rules"].as_sequence().unwrap().len(), 1);
    }

    #[test]
    fn rules_are_inserted_at_the_requested_boundary() {
        let overlay =
            parse("rules:\n- DOMAIN-SUFFIX,corp.example,DIRECT\n- IP-CIDR,203.0.113.0/24,DIRECT\n")
                .unwrap();
        let mut map = fixture();
        let existing: Vec<Value> = map["rules"].as_sequence().unwrap().clone();
        apply(&mut map, &overlay, 0).unwrap();
        let rules = map["rules"].as_sequence().unwrap();
        assert_eq!(rules[0].as_str(), Some("DOMAIN-SUFFIX,corp.example,DIRECT"));
        assert_eq!(rules[2], existing[0]);
    }

    #[test]
    fn groups_can_reference_nodes_groups_and_each_other() {
        let overlay = parse(
            "proxy-groups:\n\
             - {name: corp, type: select, proxies: [US, DIRECT]}\n\
             - {name: corp-auto, type: fallback, proxies: [corp], url: 'https://example.test', interval: 300}\n",
        )
        .unwrap();
        let mut map = fixture();
        apply(&mut map, &overlay, 1).unwrap();
        let names: Vec<&str> = map["proxy-groups"]
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(|group| group["name"].as_str())
            .collect();
        assert_eq!(names, ["select", "corp", "corp-auto"]);
    }

    #[test]
    fn unknown_targets_and_duplicate_groups_are_rejected() {
        let mut map = fixture();
        let overlay =
            parse("proxy-groups:\n- {name: ghost, type: select, proxies: [missing]}\n").unwrap();
        assert!(apply(&mut map, &overlay, 0).is_err());
        let overlay =
            parse("proxy-groups:\n- {name: select, type: select, proxies: [US]}\n").unwrap();
        assert!(apply(&mut map, &overlay, 0).is_err());
    }

    #[test]
    fn reserved_names_and_unknown_group_fields_are_rejected() {
        for text in [
            "proxy-groups:\n- {name: GLOBAL, type: select, proxies: [US]}\n",
            "proxy-groups:\n- {name: group, type: magic, proxies: [US]}\n",
            "proxy-groups:\n- {name: group, type: select, proxies: [US], script: x}\n",
            "proxy-groups:\n- {name: group, type: select}\n",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn dns_policy_rejects_ai_domains_and_system_resolvers() {
        for text in [
            "dns:\n  nameserver-policy:\n    '+.claude.ai': ['https://223.5.5.5/dns-query']\n",
            "dns:\n  nameserver-policy:\n    '+.corp.example': [system]\n",
            "dns:\n  nameserver-policy:\n    '+.corp.example': ['dhcp://auto']\n",
            "dns:\n  enhanced-mode: redir-host\n",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn dns_policy_is_merged_without_overwriting_subscription_entries() {
        let mut map = fixture();
        let mut dns = Mapping::new();
        dns.insert(
            "nameserver-policy".into(),
            serde_yaml::from_str("{'+.corp.example': ['https://1.1.1.1/dns-query']}").unwrap(),
        );
        map.insert("dns".into(), Value::Mapping(dns));
        let overlay = parse(
            "dns:\n  nameserver-policy:\n    '+.corp.example': ['https://223.5.5.5/dns-query']\n",
        )
        .unwrap();
        assert!(apply(&mut map, &overlay, 0).is_err());
    }

    #[test]
    fn oversized_or_malformed_overlay_is_rejected() {
        assert!(parse("rules: 'DOMAIN,a.com,DIRECT'").is_err());
        assert!(parse("rules:\n- MATCH,DIRECT\n").is_err());
        assert!(parse("rules:\n- DOMAIN,a.com\n").is_err());
        assert!(parse("rules:\n- UNKNOWN,a.com,DIRECT\n").is_err());
        assert!(parse("tun:\n  enable: true\n").is_err());
        assert!(parse("[]").is_err());
    }
}
