//! 出口详情数据层：
//! - 三通道出口追踪（图6：直连 / Cloudflare / OpenAI 的 /cdn-cgi/trace）
//! - 多库位置比对（图3 位置区块：并行查询多个公共 IP 库）
//! - DNS 泄露明细（图4/7：经系统解析器多轮查询 anycast whoami，收集真实解析器并逐个定归属）

use anyhow::Result;
use once_cell::sync::Lazy;
use serde::Serialize;
use std::collections::HashSet;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

#[path = "system_dns.rs"]
pub(crate) mod system_dns;

fn plain_client(no_proxy: bool, proxy_port: Option<u16>) -> Result<reqwest::Client> {
    let mut b = reqwest::Client::builder().timeout(Duration::from_secs(8));
    match (no_proxy, proxy_port) {
        (true, _) => b = b.no_proxy(),
        (_, Some(p)) => b = b.proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{p}"))?),
        _ => {}
    }
    Ok(b.build()?)
}

/// 情报查询专用客户端：RIR / 代理画像库响应普遍在 5-10 秒，超时放宽到 12 秒。
/// PeeringDB 等站点会拒绝没有 User-Agent 的请求，这里统一带上产品标识。
fn intel_client(proxy_port: Option<u16>) -> Result<reqwest::Client> {
    let mut b = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .user_agent("yunkuo-diagnostics/0.2 (+https://localhost)");
    match proxy_port {
        Some(p) => b = b.proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{p}"))?),
        None => b = b.no_proxy(),
    }
    Ok(b.build()?)
}

/// RDAP autnum：ARIN / rdap.org / RIPE 三路并发，先成功者胜出，避免被单个 RIR 拖死。
async fn first_rdap(client: &reqwest::Client, asn: u32) -> Result<serde_json::Value, String> {
    let mut set = tokio::task::JoinSet::new();
    for url in [
        format!("https://rdap.arin.net/registry/autnum/{asn}"),
        format!("https://rdap.org/autnum/{asn}"),
        format!("https://rdap.db.ripe.net/autnum/{asn}"),
    ] {
        let c = client.clone();
        set.spawn(async move { get_json(&c, &url).await });
    }
    let mut last = "RDAP 全部失败".to_string();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(Ok(value)) => {
                set.shutdown().await;
                return Ok(value);
            }
            Ok(Err(e)) => last = e,
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

/// 两个来源竞速：返回第一个拿到 Some 的结果，都失败才是 None。
/// 用于「走代理慢、直连快」的公共库（PeeringDB / StopForumSpam）。
async fn race_opt<T, FA, FB>(a: FA, b: FB) -> Option<T>
where
    FA: std::future::Future<Output = Option<T>>,
    FB: std::future::Future<Output = Option<T>>,
{
    tokio::pin!(a);
    tokio::pin!(b);
    let mut a_done = false;
    let mut b_done = false;
    loop {
        if a_done && b_done {
            return None;
        }
        tokio::select! {
            va = &mut a, if !a_done => {
                a_done = true;
                if va.is_some() {
                    return va;
                }
            }
            vb = &mut b, if !b_done => {
                b_done = true;
                if vb.is_some() {
                    return vb;
                }
            }
        }
    }
}

/// PeeringDB：ASN 自报线路容量 / 网络类型 / 流量比例 / 覆盖范围。
async fn peeringdb_net(client: &reqwest::Client, asn: u32) -> Option<PeerDb> {
    let url = format!("https://www.peeringdb.com/api/net?asn={asn}");
    let v = get_json(client, &url).await.ok()?;
    let net = v.get("data")?.as_array()?.first()?;
    Some(PeerDb {
        name: opt_str(net, &["name"]),
        traffic: opt_str(net, &["info_traffic"]),
        kind: opt_str(net, &["info_type"]),
        ratio: opt_str(net, &["info_ratio"]),
        scope: opt_str(net, &["info_scope"]),
        prefixes4: opt_u64(net, "info_prefixes4"),
    })
}

/// StopForumSpam：无需 key 的公开滥用库，命中时有最近出现时间与频次。
async fn stopforumspam(client: &reqwest::Client, ip: &str) -> Option<(bool, Option<String>)> {
    let url = format!("https://www.stopforumspam.com/api?ip={ip}&json");
    let v = get_json(client, &url).await.ok()?;
    let node = v.get("ip");
    let appears = node
        .and_then(|n| n.get("appears"))
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    if appears == 0 {
        return Some((false, None));
    }
    let freq = node
        .and_then(|n| n.get("frequency"))
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let last = node
        .and_then(|n| n.get("lastseen"))
        .and_then(|x| x.as_str())
        .unwrap_or("未知");
    Some((true, Some(format!("频次 {freq}，最近 {last}"))))
}

/// IPv4 反序，用于 DNSBL 查询：50.7.253.170 → 170.253.7.50
fn reverse_octets(ip: &str) -> Option<String> {
    let parts: Vec<&str> = ip.trim().split('.').collect();
    if parts.len() != 4 || parts.iter().any(|p| p.parse::<u8>().is_err()) {
        return None;
    }
    Some(format!(
        "{}.{}.{}.{}",
        parts[3], parts[2], parts[1], parts[0]
    ))
}

/// DNSBL 查询走阿里 DoH（直连可达，且不受本地污染/fake-ip 影响）。
/// 返回 (zone, Some(true 命中) / Some(false 未命中) / None 查询失败)。
async fn dnsbl_hits(client: &reqwest::Client, ip: &str) -> Vec<(String, Option<bool>)> {
    let Some(rev) = reverse_octets(ip) else {
        return Vec::new();
    };
    let zones = ["dnsbl-1.uceprotect.net", "dnsbl.dronebl.org"];
    let mut tasks = Vec::new();
    for zone in zones {
        let name = format!("{rev}.{zone}");
        let c = client.clone();
        tasks.push(tokio::spawn(async move {
            let url = format!("https://dns.alidns.com/resolve?name={name}&type=A");
            match get_json(&c, &url).await {
                // Status 3 = NXDOMAIN，即未被收录
                Ok(v) if v.get("Status").and_then(|x| x.as_u64()) == Some(3) => {
                    (zone.to_string(), Some(false))
                }
                Ok(v) if v.get("Status").and_then(|x| x.as_u64()) == Some(0) => {
                    let listed = v
                        .get("Answer")
                        .and_then(|x| x.as_array())
                        .map(|arr| {
                            arr.iter().any(|a| {
                                a.get("data")
                                    .and_then(|d| d.as_str())
                                    .map(|d| d.starts_with("127."))
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false);
                    (zone.to_string(), Some(listed))
                }
                _ => (zone.to_string(), None),
            }
        }));
    }
    let mut out = Vec::new();
    for task in tasks {
        if let Ok(row) = task.await {
            out.push(row);
        }
    }
    out
}

/// Tor 出口名单：约 2000 行，缓存 3 小时；拉取失败也缓存 10 分钟，避免每轮都空等。
static TOR_CACHE: Lazy<tokio::sync::Mutex<Option<(Instant, Option<HashSet<String>>)>>> =
    Lazy::new(|| tokio::sync::Mutex::new(None));

async fn tor_exit_set(client: &reqwest::Client) -> Option<HashSet<String>> {
    const OK_TTL: Duration = Duration::from_secs(3 * 3600);
    const FAIL_TTL: Duration = Duration::from_secs(600);
    {
        let guard = TOR_CACHE.lock().await;
        if let Some((at, value)) = guard.as_ref() {
            let ttl = if value.is_some() { OK_TTL } else { FAIL_TTL };
            if at.elapsed() < ttl {
                return value.clone();
            }
        }
    }
    let fetched = match client
        .get("https://check.torproject.org/torbulkexitlist")
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => resp.text().await.ok().map(|text| {
            text.lines()
                .map(|line| line.trim().to_string())
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .collect::<HashSet<String>>()
        }),
        _ => None,
    };
    *TOR_CACHE.lock().await = Some((Instant::now(), fetched.clone()));
    fetched
}

// ---------------- 三通道出口追踪 ----------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TraceExit {
    pub platform: String,
    pub ip: Option<String>,
    pub region: Option<String>,
    pub error: Option<String>,
}

fn parse_trace(text: &str) -> (Option<String>, Option<String>) {
    let mut ip = None;
    let mut loc = None;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("ip=") {
            ip = Some(v.trim().to_string());
        }
        if let Some(v) = line.strip_prefix("loc=") {
            loc = Some(v.trim().to_string());
        }
    }
    (ip, loc)
}

#[tauri::command]
pub async fn trace_exit(app: AppHandle, platform: String) -> Result<TraceExit, String> {
    let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
    let port = cfg.mixed_port;
    match platform.as_str() {
        "direct" => {
            let c = plain_client(true, None).map_err(|e| format!("{e:#}"))?;
            let v: serde_json::Value = c
                .get("https://ipapi.co/json/")
                .send()
                .await
                .map_err(|e| format!("direct: {e}"))?
                .json()
                .await
                .map_err(|e| format!("direct: {e}"))?;
            Ok(TraceExit {
                platform,
                ip: v.get("ip").and_then(|x| x.as_str()).map(String::from),
                region: v
                    .get("country_code")
                    .and_then(|x| x.as_str())
                    .map(String::from),
                error: None,
            })
        }
        "cloudflare" | "openai" => {
            let c = plain_client(false, Some(port)).map_err(|e| format!("{e:#}"))?;
            let url = if platform == "cloudflare" {
                "https://www.cloudflare.com/cdn-cgi/trace"
            } else {
                "https://chat.openai.com/cdn-cgi/trace"
            };
            let text = c
                .get(url)
                .send()
                .await
                .map_err(|e| format!("{platform}: {e}"))?
                .text()
                .await
                .map_err(|e| format!("{platform}: {e}"))?;
            let (ip, loc) = parse_trace(&text);
            Ok(TraceExit {
                platform,
                ip,
                region: loc,
                error: None,
            })
        }
        _ => Err(format!("未知通道: {platform}")),
    }
}

// ---------------- 多库位置比对 ----------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GeoRow {
    pub source: String,
    pub country: String,
    pub city: String,
    pub cc: String,
    pub error: Option<String>,
}

fn json_str(v: &serde_json::Value, keys: &[&str]) -> String {
    for k in keys {
        if let Some(s) = v.get(*k).and_then(|x| x.as_str()) {
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    String::new()
}

/// 并行查询多个公共 IP 库；目标为空时查询当前出口（经代理发问，报告的即代理出口）
#[tauri::command]
pub async fn multi_geo(app: AppHandle, ip: Option<String>) -> Result<Vec<GeoRow>, String> {
    let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
    let client = plain_client(false, Some(cfg.mixed_port)).map_err(|e| format!("{e:#}"))?;
    let target = ip.unwrap_or_default();
    let sources: Vec<(&str, String)> = vec![
        (
            "IP-API",
            format!("http://ip-api.com/json/{target}?fields=status,country,countryCode,city,isp"),
        ),
        ("IPWHO.is", format!("https://ipwho.is/{target}")),
        ("ipapi.co", format!("https://ipapi.co/{target}/json/")),
        ("IP.SB", format!("https://api.ip.sb/geoip/{target}")),
        ("IPInfo", format!("https://ipinfo.io/{target}/json")),
    ];
    let mut jobs = Vec::new();
    for (name, url) in sources {
        let c = client.clone();
        jobs.push(tokio::spawn(async move {
            let v: serde_json::Value = c
                .get(&url)
                .send()
                .await
                .map_err(|e| format!("{e}"))?
                .error_for_status()
                .map_err(|e| format!("{e}"))?
                .json()
                .await
                .map_err(|e| format!("{e}"))?;
            let _ = &v;
            Ok(GeoRow {
                source: name.to_string(),
                country: json_str(&v, &["country", "country_name"]),
                city: json_str(&v, &["city"]),
                cc: json_str(&v, &["countryCode", "country_code"]),
                error: None,
            })
        }));
    }
    let mut rows = Vec::new();
    for j in jobs {
        match j.await {
            Ok(Ok(row)) => rows.push(row),
            Ok(Err(e)) => rows.push(GeoRow {
                source: String::new(),
                country: String::new(),
                city: String::new(),
                cc: String::new(),
                error: Some(e),
            }),
            Err(e) => rows.push(GeoRow {
                source: String::new(),
                country: String::new(),
                city: String::new(),
                cc: String::new(),
                error: Some(format!("{e}")),
            }),
        }
    }
    // 恢复 source 顺序（失败行补名）
    let names = ["IP-API", "IPWHO.is", "ipapi.co", "IP.SB", "IPInfo"];
    for (i, row) in rows.iter_mut().enumerate() {
        if row.source.is_empty() {
            row.source = names.get(i).unwrap_or(&"?").to_string();
        }
        if row.error.is_some() {
            row.country = "查询失败".into();
        }
    }
    Ok(rows)
}

// ---------------- 出口 IP 情报（六面板：场景 / ASN / 技术 / 威胁 / 深度检测） ----------------

#[derive(Serialize, Clone, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct IpIntel {
    pub ip: String,
    /// ok = 主数据源齐全；partial = 部分失败；fail = 全部失败
    pub state: String,
    pub error: Option<String>,

    pub asn: Option<u32>,
    /// ASN 归属（AS 名称），如 FDCSERVERS
    pub asn_org: Option<String>,
    /// ASN 持有人全称，如 FDCSERVERS - FDCservers.net
    pub asn_name: Option<String>,
    pub isp: Option<String>,
    pub org: Option<String>,
    /// 运营商类型：hosting / isp / unknown
    pub asn_type: Option<String>,
    /// IP 标记：dc（机房）/ res（住宅）/ unknown
    pub network_class: Option<String>,
    /// IP 段注册机构：ARIN / RIPE NCC / APNIC / LACNIC / AFRINIC
    pub rir: Option<String>,
    /// 注册机构所在国家码（用于「原生 / 广播 IP」判断）
    pub rir_country: Option<String>,
    /// 该 IP 所属（对齐后）前缀
    pub prefix: Option<String>,

    /// ASN 宣告的 IPv4 地址总量
    pub asn_ipv4_count: Option<u64>,
    /// ASN 注册日期 YYYY-MM-DD
    pub asn_registered: Option<String>,

    pub is_bogon: Option<bool>,
    pub reverse_dns: Option<String>,
    pub mobile: Option<bool>,
    pub hosting: Option<bool>,
    pub proxy: Option<bool>,
    pub vpn: Option<bool>,
    pub tor: Option<bool>,
    pub proxy_type: Option<String>,
    /// 滥用风险 0-100
    pub risk_score: Option<u32>,
    /// RPKI 路由校验：valid / invalid / unknown
    pub rpki: Option<String>,

    /// PeeringDB 自报的线路容量（如 5-10Tbps），比按前缀数折算更接近参考页
    pub peeringdb_traffic: Option<String>,
    /// PeeringDB 自报的网络类型：Content / NSP / Cable/DSL/ISP / Enterprise
    pub peeringdb_type: Option<String>,
    /// PeeringDB 流量比例：Balanced / Heavy Inbound / Heavy Outbound
    pub peeringdb_ratio: Option<String>,
    /// PeeringDB 覆盖范围：Global / Regional / Asia Pacific ...
    pub peeringdb_scope: Option<String>,
    /// PeeringDB 登记的前缀数
    pub peeringdb_prefixes4: Option<u64>,
    pub peeringdb_name: Option<String>,

    /// Tor 出口节点名单命中（None = 名单没拉到，不代表「不是 Tor」）
    pub tor_exit: Option<bool>,
    /// Tor 判定来源：torbulkexitlist / proxycheck.io
    pub tor_source: Option<String>,

    /// 黑名单结论：listed / clean / unavailable
    pub blacklist_state: String,
    /// 命中的黑名单名称
    pub blacklists: Vec<String>,
    /// 命中详情（最近出现时间、频次等）
    pub blacklist_note: Option<String>,

    /// ipapi.is 的滥用分（形如 0.0605 (High)），仅在配置了 API Key 时存在
    pub abuser_score: Option<String>,
    pub is_abuser: Option<bool>,
    pub is_crawler: Option<bool>,
    /// 滥用联系人邮箱，来自 ipapi.is 的 abuse 记录
    pub abuse_email: Option<String>,
    /// ipapi.is 分支状态：ok / failed（配了 Key 但这条路径到不了）/ off（未配 Key）
    pub ipapiis_state: String,

    pub sources: Vec<String>,
}

/// PeeringDB 里与「ASN 自报信息」相关的字段
#[derive(Debug, Clone, Default)]
struct PeerDb {
    name: Option<String>,
    traffic: Option<String>,
    kind: Option<String>,
    ratio: Option<String>,
    scope: Option<String>,
    prefixes4: Option<u64>,
}

fn opt_str(v: &serde_json::Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        match v.get(*k) {
            Some(serde_json::Value::String(s)) if !s.trim().is_empty() => {
                return Some(s.trim().to_string());
            }
            Some(serde_json::Value::Number(n)) => return Some(n.to_string()),
            _ => {}
        }
    }
    None
}

fn opt_bool(v: &serde_json::Value, key: &str) -> Option<bool> {
    v.get(key).and_then(|x| x.as_bool())
}

fn opt_u64(v: &serde_json::Value, key: &str) -> Option<u64> {
    v.get(key).and_then(|x| x.as_u64())
}

/// 多源合并布尔旗标：任一源说 true 就是 true（判定类信号宁严勿松）。
fn merge_flag(slot: &mut Option<bool>, value: Option<bool>) {
    *slot = match (*slot, value) {
        (Some(a), Some(b)) => Some(a || b),
        (a, None) => a,
        (None, b) => b,
    };
}

/// 应用 ipapi.is 的响应：旗标并集 + ASN / RIR / 注册日期兜底。
/// 返回 `asn.created`，供 RDAP 不可达时补 ASN 注册日期。
fn apply_ipapiis(intel: &mut IpIntel, v: &serde_json::Value) -> Option<String> {
    let asn = v.get("asn");
    if intel.asn.is_none() {
        intel.asn = asn.and_then(|a| opt_u64(a, "asn")).map(|x| x as u32);
    }
    if intel.asn_org.is_none() {
        intel.asn_org = asn.and_then(|a| opt_str(a, &["org"]));
    }
    if intel.prefix.is_none() {
        intel.prefix = asn.and_then(|a| opt_str(a, &["route"]));
    }
    if intel.rir.is_none() {
        intel.rir = opt_str(v, &["rir"]);
    }
    merge_flag(&mut intel.hosting, opt_bool(v, "is_datacenter"));
    if let Some(company) = v.get("company") {
        if intel.org.is_none() {
            intel.org = opt_str(company, &["name"]);
        }
        intel.abuser_score = opt_str(company, &["abuser_score"])
            .or_else(|| asn.and_then(|a| opt_str(a, &["abuser_score"])));
    }
    if let Some(abuse) = v.get("abuse") {
        intel.abuse_email = opt_str(abuse, &["email"]);
    }
    intel.is_abuser = opt_bool(v, "is_abuser").or(intel.is_abuser);
    intel.is_crawler = opt_bool(v, "is_crawler").or(intel.is_crawler);

    let is_vpn = opt_bool(v, "is_vpn").unwrap_or(false);
    let is_tor = opt_bool(v, "is_tor").unwrap_or(false);
    let is_proxy = opt_bool(v, "is_proxy").unwrap_or(false);
    merge_flag(&mut intel.proxy, Some(is_proxy || is_vpn || is_tor));
    merge_flag(&mut intel.vpn, Some(is_vpn));
    merge_flag(&mut intel.tor, Some(is_tor));

    asn.and_then(|a| opt_str(a, &["created"]))
}

async fn get_json(client: &reqwest::Client, url: &str) -> Result<serde_json::Value, String> {
    client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| e.to_string())
}

/// RIPEstat 的 block.desc 形如 `ARIN (Status: ALLOCATED)`，取注册机构与其所在国。
fn rir_of(desc: &str) -> (Option<&'static str>, Option<&'static str>) {
    let s = desc.to_ascii_uppercase();
    if s.contains("ARIN") {
        (Some("ARIN"), Some("US"))
    } else if s.contains("RIPE") {
        (Some("RIPE NCC"), Some("NL"))
    } else if s.contains("APNIC") {
        (Some("APNIC"), Some("AU"))
    } else if s.contains("LACNIC") {
        (Some("LACNIC"), Some("BR"))
    } else if s.contains("AFRINIC") {
        (Some("AFRINIC"), Some("ZA"))
    } else {
        (None, None)
    }
}

fn is_public_v4(ip: std::net::Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_unspecified()
        || ip.is_multicast()
        || o[0] == 0
        || ip.is_documentation()
        || (o[0] == 100 && (64..=127).contains(&o[1]))
        || (o[0] == 198 && (o[1] == 18 || o[1] == 19)))
}

fn is_public_addr(addr: &std::net::IpAddr) -> bool {
    match addr {
        std::net::IpAddr::V4(v4) => is_public_v4(*v4),
        std::net::IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00 // 唯一本地地址 fc00::/7
                || (first & 0xffc0) == 0xfe80) // 链路本地 fe80::/10
        }
    }
}

fn parse_asn(text: &str) -> Option<u32> {
    text.trim()
        .trim_start_matches(['A', 'a', 'S', 's'])
        .parse::<u32>()
        .ok()
}

/// 汇总某 ASN 宣告的 IPv4 地址总量（按前缀长度折算地址数）
fn sum_ipv4(prefixes: &serde_json::Value) -> u64 {
    let Some(list) = prefixes.as_array() else {
        return 0;
    };
    let mut total: u64 = 0;
    for item in list {
        let Some(p) = item.get("prefix").and_then(|x| x.as_str()) else {
            continue;
        };
        if p.contains(':') {
            continue;
        }
        let Some((_, len)) = p.split_once('/') else {
            continue;
        };
        let Ok(len) = len.parse::<u32>() else {
            continue;
        };
        if len <= 32 {
            total = total.saturating_add(1u64 << (32 - len));
        }
    }
    total
}

/// 查询一个出口 IP 的完整情报：地理库 + 代理画像 + RIR/ASN 归属 + RPKI。
#[tauri::command]
pub async fn ip_intel(app: AppHandle, ip: Option<String>) -> Result<IpIntel, String> {
    let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
    let client = intel_client(Some(cfg.mixed_port)).map_err(|e| format!("{e:#}"))?;

    let mut target = ip.unwrap_or_default().trim().to_string();
    if target.is_empty() {
        target = get_json(&client, "http://ip-api.com/json/?fields=status,query")
            .await
            .ok()
            .and_then(|v| opt_str(&v, &["query"]))
            .ok_or_else(|| "无法确定待查询的出口 IP".to_string())?;
    }

    Ok(collect_intel(&client, target, cfg.ipapiis_key.trim().to_string()).await)
}

/// 情报采集主体（与 AppHandle 解耦，便于只读网络测试直接调用）。
async fn collect_intel(client: &reqwest::Client, target: String, ipapiis_key: String) -> IpIntel {
    let mut intel = IpIntel {
        ip: target.clone(),
        state: "partial".into(),
        ..Default::default()
    };
    if let Ok(addr) = target.parse::<std::net::IpAddr>() {
        intel.is_bogon = Some(!is_public_addr(&addr));
    }
    let mut sources: Vec<String> = Vec::new();
    // 直连客户端：PeeringDB / StopForumSpam / DoH 在国内直连往往比走代理快得多
    let direct = intel_client(None).unwrap_or_else(|_| client.clone());

    // ---- 第一轮：地区/ASN、代理画像、前缀归属 ----
    let geo_url = format!(
        "http://ip-api.com/json/{target}?fields=status,country,countryCode,city,timezone,\
         isp,org,as,asname,reverse,mobile,proxy,hosting,query"
    );
    let proxy_url = format!("https://proxycheck.io/v2/{target}?vpn=1&asn=1&risk=1");
    let route_url =
        format!("https://stat.ripe.net/data/prefix-overview/data.json?resource={target}");
    let ipapiis_url = (!ipapiis_key.is_empty())
        .then(|| format!("https://api.ipapi.is/?q={target}&key={ipapiis_key}"));
    let (geo, proxyimg, route, ipapi) = tokio::join!(
        get_json(&client, &geo_url),
        get_json(&client, &proxy_url),
        get_json(&client, &route_url),
        async {
            match &ipapiis_url {
                Some(url) => get_json(&client, url).await.ok(),
                None => None,
            }
        }
    );

    // ---- ipapi.is（可选 key）：独立旗标与 ASN 注册日期兜底 ----
    let mut ipapi_created: Option<String> = None;
    if let Some(v) = &ipapi {
        if v.get("ip").and_then(|x| x.as_str()).is_some() {
            ipapi_created = apply_ipapiis(&mut intel, v);
            sources.push("ipapi.is".into());
            intel.ipapiis_state = "ok".into();
        }
    } else if !ipapiis_key.is_empty() {
        // 配了 Key 却拿不到结果：多半是这条网络路径到不了 api.ipapi.is
        intel.ipapiis_state = "failed".into();
    }

    if let Ok(v) = &geo {
        if v.get("status").and_then(|s| s.as_str()) == Some("success") {
            intel.isp = opt_str(v, &["isp"]);
            if intel.org.is_none() {
                intel.org = opt_str(v, &["org"]);
            }
            if let Some(rev) = opt_str(v, &["reverse"]) {
                intel.reverse_dns = Some(rev);
            }
            // 与 ipapi.is 的结论做并集，别让后到的 false 覆盖先到的 true
            merge_flag(&mut intel.hosting, opt_bool(v, "hosting"));
            merge_flag(&mut intel.mobile, opt_bool(v, "mobile"));
            merge_flag(&mut intel.proxy, opt_bool(v, "proxy"));
            if let Some(text) = opt_str(v, &["as"]) {
                // 形如 "AS30058 FDCservers.net"：两段都要取，否则第二段错位
                let mut parts = text.splitn(2, char::is_whitespace);
                let head = parts.next().and_then(parse_asn);
                let tail = parts
                    .next()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                if intel.asn.is_none() {
                    intel.asn = head;
                }
                if intel.asn_name.is_none() {
                    intel.asn_name = tail;
                }
            }
            if intel.asn_org.is_none() {
                intel.asn_org = opt_str(v, &["asname"]);
            }
            sources.push("ip-api".into());
        }
    }

    if let Ok(v) = &proxyimg {
        if let Some(node) = v.get(target.as_str()).filter(|n| n.is_object()) {
            intel.risk_score = opt_u64(node, "risk").map(|x| x.min(100) as u32);
            if let Some(flag) = opt_str(node, &["proxy"]) {
                let on = flag.eq_ignore_ascii_case("yes");
                intel.proxy = Some(intel.proxy.unwrap_or(false) || on);
            }
            intel.proxy_type = opt_str(node, &["type"]);
            if intel.asn.is_none() {
                intel.asn = opt_str(node, &["asn"]).and_then(|s| parse_asn(&s));
            }
            if intel.asn_org.is_none() {
                intel.asn_org = opt_str(node, &["organisation", "provider"]);
            }
            if intel.isp.is_none() {
                intel.isp = opt_str(node, &["provider"]);
            }
            sources.push("proxycheck.io".into());
        }
    }

    if let Ok(v) = &route {
        if let Some(data) = v.get("data").filter(|d| d.is_object()) {
            intel.prefix = opt_str(data, &["resource"]);
            if let Some(first) = data
                .get("asns")
                .and_then(|x| x.as_array())
                .and_then(|x| x.first())
            {
                if intel.asn.is_none() {
                    intel.asn = opt_u64(first, "asn").map(|x| x as u32);
                }
                if intel.asn_name.is_none() {
                    intel.asn_name = opt_str(first, &["holder"]);
                }
            }
            if let Some(block) = data.get("block") {
                let desc = opt_str(block, &["desc"]).unwrap_or_default();
                let (rir, cc) = rir_of(&desc);
                intel.rir = rir.map(String::from);
                intel.rir_country = cc.map(String::from);
            }
            sources.push("RIPEstat".into());
        }
    }

    // ---- 第二轮：ASN 规模 / 注册日期 / RPKI / PeeringDB / 黑名单 / Tor 名单 ----
    let announced = intel.asn.map(|asn| {
        format!("https://stat.ripe.net/data/announced-prefixes/data.json?resource=AS{asn}")
    });
    let rpki_url = match (intel.asn, intel.prefix.clone()) {
        (Some(asn), Some(prefix)) => Some(format!(
            "https://stat.ripe.net/data/rpki-validation/data.json?resource=AS{asn}&prefix={prefix}"
        )),
        _ => None,
    };
    let (ann, registry, rpki, peering, spam, tor_list, dnsbl) = tokio::join!(
        async {
            match &announced {
                Some(url) => get_json(&client, url).await,
                None => Err("no-asn".to_string()),
            }
        },
        async {
            match intel.asn {
                Some(asn) => first_rdap(&client, asn).await,
                None => Err("no-asn".to_string()),
            }
        },
        async {
            match rpki_url {
                Some(url) => get_json(&client, &url).await,
                None => Err("no-prefix".to_string()),
            }
        },
        async {
            match intel.asn {
                Some(asn) => {
                    race_opt(peeringdb_net(&client, asn), peeringdb_net(&direct, asn)).await
                }
                None => None,
            }
        },
        race_opt(
            stopforumspam(&client, &target),
            stopforumspam(&direct, &target)
        ),
        async {
            tokio::time::timeout(Duration::from_secs(6), tor_exit_set(&client))
                .await
                .unwrap_or(None)
        },
        dnsbl_hits(&direct, &target)
    );

    if let Ok(v) = &ann {
        let total = v
            .get("data")
            .map(|d| sum_ipv4(d.get("prefixes").unwrap_or(&serde_json::Value::Null)))
            .unwrap_or(0);
        if total > 0 {
            intel.asn_ipv4_count = Some(total);
            sources.push("RIPEstat ASN".into());
        }
    }

    if let Ok(v) = &registry {
        if let Some(name) = opt_str(v, &["name", "handle"]) {
            if intel.asn_org.is_none() {
                intel.asn_org = Some(name);
            }
        }
        if let Some(events) = v.get("events").and_then(|x| x.as_array()) {
            for ev in events {
                let action = ev.get("eventAction").and_then(|x| x.as_str()).unwrap_or("");
                if action == "registration" {
                    if let Some(date) = ev.get("eventDate").and_then(|x| x.as_str()) {
                        intel.asn_registered = Some(date.chars().take(10).collect());
                        break;
                    }
                }
            }
        }
        sources.push("RDAP".into());
    }

    if let Ok(v) = &rpki {
        if let Some(status) = v
            .get("data")
            .and_then(|d| d.get("status"))
            .and_then(|x| x.as_str())
        {
            // not-found 归入 unknown，避免把「未签 ROA」误报成路由劫持
            intel.rpki = Some(if status == "not-found" {
                "unknown".to_string()
            } else {
                status.to_string()
            });
        }
    }

    if let Some(p) = peering {
        intel.peeringdb_name = p.name;
        intel.peeringdb_traffic = p.traffic;
        intel.peeringdb_type = p.kind;
        intel.peeringdb_ratio = p.ratio;
        intel.peeringdb_scope = p.scope;
        intel.peeringdb_prefixes4 = p.prefixes4;
        sources.push("PeeringDB".into());
    }

    // 黑名单：任一来源命中即 listed；只有所有来源都明确未命中才算 clean
    let mut answered = 0usize;
    match spam {
        Some((true, note)) => {
            answered += 1;
            intel.blacklists.push("StopForumSpam".into());
            intel.blacklist_note = note;
        }
        Some((false, _)) => answered += 1,
        None => {}
    }
    for (zone, verdict) in dnsbl {
        if let Some(listed) = verdict {
            answered += 1;
            if listed {
                intel.blacklists.push(zone);
            }
        }
    }
    intel.blacklist_state = if !intel.blacklists.is_empty() {
        "listed".into()
    } else if answered > 0 {
        "clean".into()
    } else {
        "unavailable".into()
    };
    if answered > 0 {
        sources.push("DNSBL".into());
    }

    // Tor：名单命中直接判定；名单没拉到则退回代理画像库的类型字段
    if let Some(set) = &tor_list {
        let hit = set.contains(&target);
        intel.tor_exit = Some(hit);
        intel.tor_source = Some("torbulkexitlist".into());
        if hit {
            intel.tor = Some(true);
        }
        sources.push("Tor 名单".into());
    }

    if let Some(text) = &intel.proxy_type {
        let t = text.to_ascii_lowercase();
        merge_flag(&mut intel.vpn, Some(t.contains("vpn")));
        if t.contains("tor") {
            merge_flag(&mut intel.tor, Some(true));
            if intel.tor_source.is_none() {
                intel.tor_source = Some("proxycheck.io".into());
            }
        }
    }

    // RDAP 不可达时，用 ipapi.is 的 ASN 注册日期兜底
    if intel.asn_registered.is_none() {
        if let Some(date) = ipapi_created {
            intel.asn_registered = Some(date.chars().take(10).collect());
        }
    }

    intel.network_class = Some(
        match intel.hosting {
            Some(true) => "dc",
            Some(false) => "res",
            None => "unknown",
        }
        .to_string(),
    );
    intel.asn_type = Some(
        match intel.hosting {
            Some(true) => "hosting",
            Some(false) => "isp",
            None => "unknown",
        }
        .to_string(),
    );

    intel.state = if sources.len() >= 3 {
        "ok".into()
    } else if sources.is_empty() {
        "fail".into()
    } else {
        "partial".into()
    };
    if sources.is_empty() {
        intel.error = Some("所有情报源均不可达，请检查代理与 DNS".into());
    }
    intel.sources = sources;
    intel
}

const SCAN_PORTS: &[u16] = &[
    21, 22, 23, 25, 53, 80, 110, 143, 443, 445, 1433, 3306, 3389, 5432, 6379, 8080,
];

/// 手动探测常见端口（只在用户点按钮时执行，直连、单次 1.2 秒超时）
#[tauri::command]
pub async fn scan_ports(ip: String) -> Result<Vec<u16>, String> {
    let addr: std::net::IpAddr = ip
        .trim()
        .parse()
        .map_err(|_| "IP 地址格式不正确".to_string())?;
    if !is_public_addr(&addr) {
        return Err("只能探测公网出口 IP".to_string());
    }
    let open = tokio::task::spawn_blocking(move || {
        let mut handles = Vec::with_capacity(SCAN_PORTS.len());
        for &port in SCAN_PORTS {
            handles.push(std::thread::spawn(move || {
                let sock = std::net::SocketAddr::new(addr, port);
                std::net::TcpStream::connect_timeout(&sock, Duration::from_millis(1200))
                    .ok()
                    .map(|_| port)
            }));
        }
        let mut open: Vec<u16> = handles
            .into_iter()
            .filter_map(|h| h.join().ok().flatten())
            .collect();
        open.sort_unstable();
        open
    })
    .await
    .map_err(|e| format!("端口探测任务失败：{e}"))?;
    Ok(open)
}

// ---------------- DNS 泄露明细（真实系统解析器） ----------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DnsLeakRow {
    pub ip: String,
    pub country: String,
    pub cc: String,
    pub isp: String,
    pub leaked: Option<bool>,
    pub via: String,
}

fn dns_location(v: &serde_json::Value) -> (String, String, String, Option<bool>) {
    let cc = json_str(v, &["countryCode"]);
    if v.get("status").and_then(|s| s.as_str()) != Some("success")
        || cc.len() != 2
        || !cc.bytes().all(|c| c.is_ascii_alphabetic())
    {
        return ("归属地查询失败".into(), String::new(), String::new(), None);
    }
    (
        json_str(v, &["country"]),
        cc.clone(),
        json_str(v, &["isp"]),
        Some(cc.eq_ignore_ascii_case("CN")),
    )
}

/// 通过系统 TXT 身份查询收集解析器；不将 Fake-IP 的 A 记录当作解析器。
#[tauri::command]
pub async fn dns_leak(_app: AppHandle, rounds: Option<u32>) -> Result<Vec<DnsLeakRow>, String> {
    system_dns_rows(rounds).await
}

async fn system_dns_rows(rounds: Option<u32>) -> Result<Vec<DnsLeakRow>, String> {
    let resolvers = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::task::spawn_blocking(move || system_dns::probe(rounds.unwrap_or(2))),
    )
    .await
    .map_err(|_| "系统 DNS TXT 查询超过 10 秒，请检查系统解析器后重试".to_string())?
    .map_err(|e| format!("系统 DNS 查询任务失败：{e}"))??;
    let client = plain_client(true, None).map_err(|e| format!("{e:#}"))?;
    let mut tasks = tokio::task::JoinSet::new();
    for (rip, sources) in resolvers.into_iter().take(16) {
        let client = client.clone();
        tasks.spawn(async move {
            let url = format!("http://ip-api.com/json/{rip}?fields=status,country,countryCode,isp");
            let (country, cc, isp, leaked) = match client
                .get(url)
                .send()
                .await
                .and_then(|r| r.error_for_status())
            {
                Ok(resp) => dns_location(&resp.json().await.unwrap_or_default()),
                Err(_) => ("归属地查询失败".into(), String::new(), String::new(), None),
            };
            DnsLeakRow {
                ip: rip,
                country,
                cc,
                isp,
                leaked,
                via: sources.into_iter().collect::<Vec<_>>().join("；"),
            }
        });
    }
    let mut rows = Vec::new();
    while let Some(result) = tasks.join_next().await {
        rows.push(result.map_err(|e| format!("DNS 归属地查询任务失败：{e}"))?);
    }
    rows.sort_by(|a, b| b.leaked.cmp(&a.leaked).then(a.ip.cmp(&b.ip)));
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_dns_location_is_unknown() {
        for value in [
            serde_json::json!({}),
            serde_json::json!({"status":"fail","countryCode":"US"}),
            serde_json::json!({"status":"success","countryCode":""}),
        ] {
            assert_eq!(dns_location(&value).3, None);
        }
        assert_eq!(
            dns_location(&serde_json::json!({"status":"success","countryCode":"US"})).3,
            Some(false)
        );
        assert_eq!(
            dns_location(&serde_json::json!({"status":"success","countryCode":"CN"})).3,
            Some(true)
        );
    }
    #[tokio::test]
    #[ignore = "需要真实系统 DNS，仅手动运行此只读网络探测"]
    async fn live_system_txt_dns_with_fake_ip() {
        let rows = system_dns_rows(Some(1))
            .await
            .expect("系统 TXT 探测应取得解析器");
        assert!(!rows.is_empty());
        assert!(
            rows.iter()
                .all(|row| system_dns::public_ip(&row.ip).is_some()
                    && row.via.starts_with("系统 DNS"))
        );
        println!(
            "系统 TXT 解析器 {} 个；归属地成功 {} 个；合成地址 0 个",
            rows.len(),
            rows.iter().filter(|row| !row.cc.is_empty()).count()
        );
    }

    #[test]
    fn asn_prefix_and_rir_helpers() {
        assert_eq!(parse_asn("AS30058"), Some(30058));
        assert_eq!(parse_asn(" 30058 "), Some(30058));
        assert_eq!(parse_asn("FDCSERVERS"), None);
        assert_eq!(rir_of("ARIN (Status: ALLOCATED)").0, Some("ARIN"));
        assert_eq!(rir_of("RIPE NCC (Status: ALLOCATED)").1, Some("NL"));
        assert_eq!(rir_of("unknown block").0, None);
        let prefixes = serde_json::json!([
            {"prefix": "50.7.0.0/16"},
            {"prefix": "1.2.3.0/24"},
            {"prefix": "2001:db8::/32"}
        ]);
        assert_eq!(sum_ipv4(&prefixes), 65_536 + 256);
        assert_eq!(sum_ipv4(&serde_json::json!(null)), 0);
        assert!(is_public_v4("50.7.253.170".parse().unwrap()));
        assert!(!is_public_v4("192.168.1.1".parse().unwrap()));
        assert!(!is_public_v4("100.64.0.1".parse().unwrap()));
    }

    #[test]
    fn reverse_octets_for_dnsbl() {
        assert_eq!(
            reverse_octets("50.7.253.170").as_deref(),
            Some("170.253.7.50")
        );
        assert_eq!(reverse_octets(" 1.1.1.1 ").as_deref(), Some("1.1.1.1"));
        assert_eq!(reverse_octets("2001:db8::1"), None);
        assert_eq!(reverse_octets("1.2.3"), None);
        assert_eq!(reverse_octets("1.2.3.999"), None);
    }

    #[test]
    fn ipapiis_payload_maps_flags_and_asn() {
        // 真实响应（免费层 1000 次/天），字段裁剪后固化为用例
        let payload = serde_json::json!({
            "ip": "50.7.253.170",
            "rir": "ARIN",
            "is_datacenter": true,
            "is_tor": false,
            "is_proxy": true,
            "is_vpn": false,
            "is_abuser": true,
            "is_crawler": false,
            "company": {
                "name": "FDCservers.net",
                "abuser_score": "0.0605 (High)",
                "type": "hosting"
            },
            "abuse": { "email": "abuse@fdcservers.net" },
            "asn": {
                "asn": 30058,
                "route": "50.7.252.0/23",
                "org": "FDCservers.net",
                "type": "hosting",
                "created": "2003-07-10"
            }
        });

        let mut intel = IpIntel::default();
        let created = apply_ipapiis(&mut intel, &payload);
        assert_eq!(created.as_deref(), Some("2003-07-10"));
        assert_eq!(intel.asn, Some(30058));
        assert_eq!(intel.asn_org.as_deref(), Some("FDCservers.net"));
        assert_eq!(intel.prefix.as_deref(), Some("50.7.252.0/23"));
        assert_eq!(intel.rir.as_deref(), Some("ARIN"));
        assert_eq!(intel.hosting, Some(true));
        assert_eq!(intel.proxy, Some(true));
        // ipapi.is 说 is_vpn=false，但 is_proxy=true：代理与 VPN 要分开呈现
        assert_eq!(intel.vpn, Some(false));
        assert_eq!(intel.tor, Some(false));
        assert_eq!(intel.is_abuser, Some(true));
        assert_eq!(intel.abuser_score.as_deref(), Some("0.0605 (High)"));
        assert_eq!(intel.abuse_email.as_deref(), Some("abuse@fdcservers.net"));

        // 第二个源说 false 不能推翻已经为 true 的判定
        let mut merged = IpIntel {
            proxy: Some(true),
            vpn: Some(true),
            ..Default::default()
        };
        apply_ipapiis(&mut merged, &payload);
        assert_eq!(merged.proxy, Some(true));
        assert_eq!(merged.vpn, Some(true));
    }

    #[tokio::test]
    #[ignore = "需要真实网络，仅手动运行此只读情报查询"]
    async fn live_ip_intel_matches_rir_data() {
        // ARIN RDAP 在部分网络直连超时，可用 YUNKUO_TEST_PROXY=<mixed_port> 走本机代理
        let proxy = std::env::var("YUNKUO_TEST_PROXY")
            .ok()
            .and_then(|v| v.parse::<u16>().ok());
        let client = intel_client(proxy).expect("构建情报查询客户端");
        // 有 key 时额外校验 ipapi.is 分支（免费层 1000 次/天）
        let key = std::env::var("IPAPIIS_KEY").unwrap_or_default();
        let intel = collect_intel(&client, "50.7.253.170".to_string(), key).await;
        println!("{intel:?}");
        assert_eq!(intel.asn, Some(30058));
        assert_eq!(intel.asn_org.as_deref(), Some("FDCSERVERS"));
        assert_eq!(intel.rir.as_deref(), Some("ARIN"));
        assert!(intel.asn_ipv4_count.unwrap_or(0) > 100_000);
        assert_eq!(intel.network_class.as_deref(), Some("dc"));
        assert_eq!(intel.asn_type.as_deref(), Some("hosting"));
        assert_eq!(intel.proxy, Some(true));
        // PeeringDB 自报容量，必须与参考页一致
        assert_eq!(intel.peeringdb_traffic.as_deref(), Some("5-10Tbps"));
        assert_eq!(intel.peeringdb_prefixes4, Some(30000));
        assert!(!intel.blacklist_state.is_empty());
        // RDAP 可达时才要求注册日期；不可达时前端显示「—」，不应误报成注册于今天
        if let Some(date) = &intel.asn_registered {
            assert_eq!(date, "2003-07-10");
        }
    }
}
