//! 浏览器泄漏实测：浏览器触发随机域名，后端只经本机代理读取权威观察结果。
use anyhow::{Result, bail};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    net::IpAddr,
    time::{Duration, Instant},
};

pub const ICE_SERVERS: [&str; 3] = [
    "local",
    "stun:stun.cloudflare.com:3478",
    "stun:stun.l.google.com:19302",
];
const BASE: &str = "https://dnsleak.dev";

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IceCandidate {
    pub address: String,
    pub kind: String,
    pub protocol: String,
    pub related_address: Option<String>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IceProbe {
    pub server: String,
    pub outcome: String,
    pub candidates: Vec<IceCandidate>,
    pub error: Option<String>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebRtc {
    pub supported: bool,
    pub probes: Vec<IceProbe>,
}
impl WebRtc {
    pub fn valid(&self) -> bool {
        self.probes.len() <= ICE_SERVERS.len()
            && self.probes.iter().all(|p| {
                ICE_SERVERS.contains(&p.server.as_str())
                    && ["complete", "timeout", "error", "unsupported"].contains(&p.outcome.as_str())
                    && p.error.as_ref().is_none_or(|e| e.len() <= 200)
                    && p.candidates.len() <= 32
                    && p.candidates.iter().all(|c| {
                        c.address.len() <= 100
                            && c.related_address.as_ref().is_none_or(|a| a.len() <= 100)
                            && ["host", "srflx", "prflx", "relay"].contains(&c.kind.as_str())
                            && ["udp", "tcp"].contains(&c.protocol.as_str())
                    })
            })
    }
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolver {
    pub ip: String,
    pub country: String,
    pub operator: String,
    pub transport: String,
    pub saw_a: bool,
    pub saw_q: bool,
    pub ecs: Vec<String>,
}
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DnsReport {
    pub state: String,
    pub detail: String,
    pub provider_status: String,
    pub client_ip: Option<String>,
    pub expected: usize,
    pub observed: usize,
    pub resolvers: Vec<Resolver>,
}
#[derive(Default)]
struct Run {
    id: String,
    started: Option<Instant>,
    challenge: Option<(String, Vec<String>)>,
    result: Option<DnsReport>,
}
#[derive(Default)]
pub struct Session {
    run: Mutex<Run>,
}
impl Session {
    pub fn begin(&self) -> String {
        let id = uuid::Uuid::new_v4().simple().to_string();
        *self.run.lock() = Run {
            id: id.clone(),
            started: Some(Instant::now()),
            ..Default::default()
        };
        id
    }
    pub fn current(&self, id: &str) -> bool {
        let run = self.run.lock();
        !id.is_empty()
            && run.id == id
            && run
                .started
                .is_some_and(|t| t.elapsed() <= Duration::from_secs(90))
    }
    pub fn result(&self, id: &str) -> Option<DnsReport> {
        if !self.current(id) {
            return None;
        }
        self.run.lock().result.clone()
    }
    pub fn start(&self, id: &str, port: u16) -> Result<Value> {
        if !self.current(id) {
            bail!("检测轮次已失效，请重新检测");
        }
        if self.run.lock().challenge.is_some() {
            bail!("当前轮次已分配测试域名，请重新检测");
        }
        let value = provider(port, "/api/new")?;
        let (test_id, hosts) = challenge(&value)?;
        if !self.current(id) {
            bail!("检测轮次已失效");
        }
        self.run.lock().challenge = Some((test_id, hosts.clone()));
        Ok(json!({"hostnames":hosts}))
    }
    pub fn collect(&self, id: &str, port: u16) -> Result<DnsReport> {
        if !self.current(id) {
            bail!("检测轮次已失效，请重新检测");
        }
        let (test_id, hosts) = self
            .run
            .lock()
            .challenge
            .clone()
            .ok_or_else(|| anyhow::anyhow!("尚未分配测试域名"))?;
        self.run.lock().result = None;
        let value = provider(port, &format!("/api/result/{test_id}"))?;
        let report = dns_report(&value, &test_id, hosts.len())?;
        if !self.current(id) {
            bail!("检测轮次已失效");
        }
        self.run.lock().result = Some(report.clone());
        Ok(report)
    }
}
fn provider(port: u16, route: &str) -> Result<Value> {
    if port == 0 {
        bail!("代理端口不可用");
    }
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))?)
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(8))
        .build()?;
    let mut response = client
        .get(format!("{BASE}{route}"))
        .header("Accept", "application/json")
        .send()
        .map_err(|_| anyhow::anyhow!("DNS 权威观察服务不可用，结果未知；请确认代理后重试"))?;
    if response.status().as_u16() == 429 {
        bail!("DNS 观察服务限流，请稍后重新检测");
    }
    if !response.status().is_success() {
        bail!("DNS 观察服务未返回成功结果");
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    (&mut response).take(262145).read_to_end(&mut bytes)?;
    if bytes.len() > 262144 {
        bail!("DNS 观察结果超出大小限制");
    }
    Ok(serde_json::from_slice(&bytes)?)
}
fn challenge(value: &Value) -> Result<(String, Vec<String>)> {
    let id = value["testId"].as_str().unwrap_or_default();
    if !(6..=64).contains(&id.len())
        || !id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    {
        bail!("DNS 服务返回了无效会话");
    }
    let names = value["hostnames"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("DNS 测试域名缺失"))?;
    if !(4..=24).contains(&names.len()) {
        bail!("DNS 测试域名数量不合法");
    }
    let mut hosts = Vec::new();
    for name in names {
        let host = name.as_str().unwrap_or_default();
        let part = host
            .strip_prefix(&format!("{id}-"))
            .and_then(|s| s.strip_suffix(".t.dnsleak.dev"))
            .unwrap_or_default();
        if part.len() != 3
            || !matches!(part.as_bytes()[0], b'a' | b'q')
            || !part.as_bytes()[1..].iter().all(u8::is_ascii_digit)
            || hosts.iter().any(|h| h == host)
        {
            bail!("DNS 测试域名不合法");
        }
        hosts.push(host.to_string());
    }
    if !hosts.iter().any(|h| h.starts_with(&format!("{id}-a")))
        || !hosts.iter().any(|h| h.starts_with(&format!("{id}-q")))
    {
        bail!("DNS 测试缺少 A 或 AAAA 域名组");
    }
    Ok((id.into(), hosts))
}
fn short(value: &Value) -> String {
    value
        .as_str()
        .unwrap_or_default()
        .chars()
        .take(120)
        .collect()
}
fn dns_report(value: &Value, id: &str, expected: usize) -> Result<DnsReport> {
    if value["testId"] != id {
        bail!("DNS 观察结果不属于本轮检测");
    }
    let status = short(&value["verdict"]["status"]);
    let observed = value["lookups"].as_u64().unwrap_or(0).min(1000) as usize;
    let mut report = DnsReport {
        state: "unknown".into(),
        detail: "DNS 权威观察尚未完整，结果未知".into(),
        provider_status: status.clone(),
        client_ip: value["client"]["ip"]
            .as_str()
            .filter(|s| s.parse::<IpAddr>().is_ok())
            .map(str::to_string),
        expected,
        observed,
        ..Default::default()
    };
    let rows = value["resolvers"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("DNS 解析器结果缺失"))?;
    if rows.len() > 64 {
        bail!("DNS 解析器数量超出限制");
    }
    let mut invalid = false;
    // 对应 privacy::harden 中固定的 1.1.1.1 / 1.0.0.1。
    // no_leak 只表示服务未发现分流，不能代替本应用已知的解析器策略。
    let mut expected_resolvers = true;
    for row in rows {
        if row["role"] == "observer" {
            continue;
        }
        let ip = short(&row["ip"]);
        if row["role"] != "resolver" || address_kind(&ip) != "public" {
            invalid = true;
            continue;
        }
        let asn = row["asn"].as_u64() == Some(13335)
            || matches!(row["asn"].as_str(), Some("13335" | "AS13335"));
        expected_resolvers &= row["operator"]
            .as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case("Cloudflare"))
            || asn;
        let mut ecs: Vec<String> = row["ecs"]
            .as_array()
            .map(|a| {
                a.iter()
                    .take(8)
                    .map(short)
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        if let Some(prefix) = row["ecs"].as_str() {
            if !prefix.is_empty() {
                ecs = vec![prefix.chars().take(120).collect()];
            }
        }
        report.resolvers.push(Resolver {
            ip,
            country: {
                let code = short(&row["countryCode"]);
                if code.is_empty() {
                    short(&row["country"])
                } else {
                    code
                }
            },
            operator: {
                let name = short(&row["operator"]);
                if name.is_empty() {
                    short(&row["org"])
                } else {
                    name
                }
            },
            transport: short(&row["transport"]),
            saw_a: row["sawA"] == true,
            saw_q: row["sawQ"] == true,
            ecs,
        });
    }
    if matches!(status.as_str(), "split" | "leak") {
        report.state = "risk".into();
        report.detail = "权威观察服务报告解析路径分流或泄漏风险，请复核解析器".into();
    } else if status == "no_leak" && !expected_resolvers {
        report.detail="观察到的解析器不匹配当前严格配置的 Cloudflare；也可能由代理节点远端解析，请复核，结果未知".into();
    } else if status == "no_leak"
        && observed == expected
        && expected > 0
        && !invalid
        && !report.resolvers.is_empty()
        && report.client_ip.is_some()
        && value["truncated"] == false
    {
        report.state = "pass".into();
        report.detail = "本轮域名观察完整，解析器符合 Cloudflare 策略且未报告解析路径分流".into();
    }
    Ok(report)
}

fn normalized(ip: &str) -> Option<IpAddr> {
    let value: IpAddr = ip.parse().ok()?;
    Some(match value {
        IpAddr::V6(v) => v.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(value),
        _ => value,
    })
}
pub fn same_ip(left: &str, right: &str) -> bool {
    normalized(left).is_some() && normalized(left) == normalized(right)
}
fn address_kind(address: &str) -> &'static str {
    if address.to_ascii_lowercase().ends_with(".local") {
        return "mdns";
    }
    match normalized(address) {
        Some(IpAddr::V4(v)) => {
            let [a, b, c, _] = v.octets();
            if v.is_unspecified() || v.is_multicast() || v.is_broadcast() {
                "ignored"
            } else if a == 10
                || a == 127
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 168)
                || (a == 169 && b == 254)
                || (a == 100 && (64..=127).contains(&b))
                || (a == 198 && (b == 18 || b == 19))
            {
                "private"
            } else if a == 0
                || a >= 240
                || (a == 192 && b == 0 && (c == 0 || c == 2))
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113)
            {
                "invalid"
            } else {
                "public"
            }
        }
        Some(IpAddr::V6(v)) => {
            let s = v.segments();
            if v.is_unspecified() || v.is_multicast() {
                "ignored"
            } else if v.is_loopback() || s[0] & 0xfe00 == 0xfc00 || s[0] & 0xffc0 == 0xfe80 {
                "private"
            } else if s[0] & 0xe000 == 0x2000 && !(s[0] == 0x2001 && s[1] == 0xdb8) {
                "public"
            } else {
                "invalid"
            }
        }
        None => "invalid",
    }
}
pub fn webrtc_verdict(report: Option<&WebRtc>, exit: Option<&str>) -> (&'static str, String) {
    let Some(report) = report.filter(|r| r.valid() && r.supported) else {
        return ("unknown", "WebRTC 不可用或结果缺失，无法核验".into());
    };
    let mut public = std::collections::BTreeSet::new();
    let mut private = std::collections::BTreeSet::new();
    let mut invalid = false;
    for p in &report.probes {
        for c in &p.candidates {
            for (address, relay) in std::iter::once((c.address.as_str(), c.kind == "relay"))
                .chain(c.related_address.as_deref().map(|a| (a, false)))
            {
                match address_kind(address) {
                    "private" => {
                        private.insert(address.to_string());
                    }
                    "public" if !relay => {
                        public.insert(address.to_string());
                    }
                    "invalid" => invalid = true,
                    _ => {}
                }
            }
        }
    }
    if !private.is_empty() {
        return (
            "risk",
            format!(
                "网页可读取局域网/本机地址：{}",
                private.into_iter().collect::<Vec<_>>().join("、")
            ),
        );
    }
    if let Some(exit) = exit.filter(|ip| normalized(ip).is_some()) {
        if public.iter().any(|ip| !same_ip(ip, exit)) {
            return (
                "risk",
                format!(
                    "公网候选与浏览器出口不一致：{}",
                    public.into_iter().collect::<Vec<_>>().join("、")
                ),
            );
        }
        if !public.is_empty()
            && !invalid
            && ICE_SERVERS.iter().all(|server| {
                report
                    .probes
                    .iter()
                    .filter(|p| p.server == *server && p.outcome == "complete")
                    .count()
                    == 1
            })
            && report.probes.len() == ICE_SERVERS.len()
            && report
                .probes
                .iter()
                .filter(|p| p.server != "local")
                .all(|p| {
                    p.candidates.iter().any(|c| {
                        c.kind == "srflx"
                            && address_kind(&c.address) == "public"
                            && same_ip(&c.address, exit)
                    })
                })
        {
            return (
                "pass",
                "本轮公网候选均与浏览器出口一致，未暴露局域网地址".into(),
            );
        }
    }
    (
        "unknown",
        "无公网候选、探测超时或结果不完整；禁用非代理 UDP 时也可能出现，不能据此证明无泄漏".into(),
    )
}
pub fn dns_verdict(report: Option<&DnsReport>, exit: Option<&str>) -> (&'static str, String) {
    let Some(report) = report else {
        return ("unknown", "DNS 权威观察未完成或服务不可用".into());
    };
    if report.state == "risk" {
        return ("risk", report.detail.clone());
    }
    if report.state == "pass"
        && exit
            .zip(report.client_ip.as_deref())
            .is_some_and(|(a, b)| same_ip(a, b))
    {
        return (
            "pass",
            format!(
                "{}；{} / {} 个测试域名，{} 个解析器地址",
                report.detail,
                report.observed,
                report.expected,
                report.resolvers.len()
            ),
        );
    }
    (
        "unknown",
        format!(
            "{}；观察到 {} / {} 个域名，需核对服务出口与浏览器出口",
            report.detail, report.observed, report.expected
        ),
    )
}

#[cfg(test)]
pub(super) fn passing_fixture(ip: &str) -> (DnsReport, WebRtc) {
    let dns = DnsReport {
        state: "pass".into(),
        client_ip: Some(ip.into()),
        expected: 4,
        observed: 4,
        ..Default::default()
    };
    let rtc = WebRtc {
        supported: true,
        probes: ICE_SERVERS
            .iter()
            .map(|s| IceProbe {
                server: s.to_string(),
                outcome: "complete".into(),
                candidates: if *s == "local" {
                    vec![]
                } else {
                    vec![IceCandidate {
                        address: ip.into(),
                        kind: "srflx".into(),
                        protocol: "udp".into(),
                        ..Default::default()
                    }]
                },
                error: None,
            })
            .collect(),
    };
    (dns, rtc)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn observed(status: &str, count: usize) -> Value {
        json!({"testId":"fixtureid","client":{"ip":"8.8.8.8"},"lookups":count,"truncated":false,"verdict":{"status":status,"leaking":false},"resolvers":[{"ip":"1.1.1.1","role":"resolver","operator":"Cloudflare","countryCode":"US","transport":"udp","sawA":true,"sawQ":true}]})
    }
    #[test]
    fn dns_requires_full_authoritative_evidence_and_matching_exit() {
        let report = dns_report(&observed("no_leak", 4), "fixtureid", 4).unwrap();
        assert_eq!(dns_verdict(Some(&report), Some("8.8.8.8")).0, "pass");
        assert_eq!(dns_verdict(Some(&report), Some("9.9.9.9")).0, "unknown");
        assert_eq!(
            dns_report(&observed("no_leak", 3), "fixtureid", 4)
                .unwrap()
                .state,
            "unknown"
        );
        assert_eq!(
            dns_report(&observed("inconclusive", 4), "fixtureid", 4)
                .unwrap()
                .state,
            "unknown"
        );
        assert_eq!(
            dns_report(&observed("new-unknown-status", 4), "fixtureid", 4)
                .unwrap()
                .state,
            "unknown"
        );
        assert_eq!(
            dns_report(&observed("split", 4), "fixtureid", 4)
                .unwrap()
                .state,
            "risk"
        );
        assert!(dns_report(&observed("no_leak", 4), "anotherid", 4).is_err());
        let mut value = observed("no_leak", 4);
        value["resolvers"][0]["role"] = json!("observer");
        assert_eq!(dns_report(&value, "fixtureid", 4).unwrap().state, "unknown");
        value = observed("no_leak", 4);
        value["truncated"] = json!(true);
        assert_eq!(dns_report(&value, "fixtureid", 4).unwrap().state, "unknown");
    }
    #[test]
    fn unexpected_resolvers_and_arbitrary_hosts_are_rejected() {
        let mut unexpected = observed("no_leak", 4);
        unexpected["resolvers"][0]["operator"] = json!("Local ISP");
        assert_eq!(
            dns_report(&unexpected, "fixtureid", 4).unwrap().state,
            "unknown"
        );
        unexpected["resolvers"][0]["asn"] = json!(13335);
        assert_eq!(
            dns_report(&unexpected, "fixtureid", 4).unwrap().state,
            "pass"
        );
        let valid = json!({"testId":"fixtureid","hostnames":["fixtureid-a01.t.dnsleak.dev","fixtureid-q01.t.dnsleak.dev","fixtureid-a02.t.dnsleak.dev","fixtureid-q02.t.dnsleak.dev"],"resultUrl":"http://127.0.0.1/private"});
        assert_eq!(challenge(&valid).unwrap().1.len(), 4);
        for host in [
            "http://127.0.0.1/private",
            "fixtureid-a01.t.dnsleak.dev.attacker.test",
            "different-a01.t.dnsleak.dev",
            "fixtureid-a01.t.dnsleak.dev:443",
        ] {
            let mut bad = valid.clone();
            bad["hostnames"][0] = json!(host);
            assert!(challenge(&bad).is_err());
        }
    }
    #[test]
    fn new_round_revokes_previous_dns_evidence_and_expired_rounds() {
        let session = Session::default();
        let old = session.begin();
        session.run.lock().result = Some(passing_fixture("8.8.8.8").0);
        assert!(session.result(&old).is_some());
        let next = session.begin();
        assert!(!session.current(&old));
        assert!(session.result(&old).is_none());
        assert!(session.result(&next).is_none());
        session.run.lock().started = Some(Instant::now() - Duration::from_secs(91));
        assert!(!session.current(&next));
    }
    #[test]
    fn ice_requires_complete_probes_and_never_promotes_no_candidates() {
        let (_, mut rtc) = passing_fixture("8.8.8.8");
        assert_eq!(webrtc_verdict(Some(&rtc), Some("8.8.8.8")).0, "pass");
        rtc.probes[1].outcome = "timeout".into();
        assert_eq!(webrtc_verdict(Some(&rtc), Some("8.8.8.8")).0, "unknown");
        for p in &mut rtc.probes {
            p.candidates.clear();
            p.outcome = "complete".into();
        }
        assert_eq!(webrtc_verdict(Some(&rtc), Some("8.8.8.8")).0, "unknown");
        rtc.probes[0].candidates.push(IceCandidate {
            address: "hidden.local".into(),
            kind: "host".into(),
            protocol: "udp".into(),
            ..Default::default()
        });
        assert_eq!(webrtc_verdict(Some(&rtc), Some("8.8.8.8")).0, "unknown");
        assert_eq!(webrtc_verdict(None, Some("8.8.8.8")).0, "unknown");
    }
    #[test]
    fn ice_compares_ipv4_ipv6_related_addresses_and_relay_without_false_leaks() {
        let (_, mut rtc) = passing_fixture("8.8.8.8");
        assert_eq!(webrtc_verdict(Some(&rtc), Some("9.9.9.9")).0, "risk");
        rtc.probes[1].candidates[0].related_address = Some("192.168.1.2".into());
        assert_eq!(webrtc_verdict(Some(&rtc), Some("8.8.8.8")).0, "risk");
        let (_, mut rtc) = passing_fixture("2606:4700:4700::1111");
        assert_eq!(
            webrtc_verdict(Some(&rtc), Some("2606:4700:4700:0:0:0:0:1111")).0,
            "pass"
        );
        rtc.probes[0].candidates.push(IceCandidate {
            address: "9.9.9.9".into(),
            kind: "relay".into(),
            protocol: "tcp".into(),
            ..Default::default()
        });
        assert_eq!(
            webrtc_verdict(Some(&rtc), Some("2606:4700:4700::1111")).0,
            "pass"
        );
        assert!(same_ip("::ffff:8.8.8.8", "8.8.8.8"));
        for ip in ["fc00::1", "fe80::1", "::1", "100.64.1.1", "192.168.1.1"] {
            assert_eq!(address_kind(ip), "private");
        }
        assert_eq!(address_kind("203.0.113.1"), "invalid");
        assert_eq!(address_kind("not-an-ip"), "invalid");
    }
    #[test]
    fn proxy_failure_has_no_direct_fallback() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(provider(port, "/api/new").is_err());
    }
}
