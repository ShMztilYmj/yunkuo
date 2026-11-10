//! 多轮浏览器 DNS 采样。只读公开权威观察接口，不更改 DNS、代理或保护设置。
//!
//! 权威站从 bash.ws 换成 dnsleak.dev：bash.ws 的 `/id` 已经变成印尼语首页，
//! 取会话号的接口不存在了，客户端只会把那段 HTML 当成会话号，
//! 深度测试于是永远停在「未返回有效会话」。新接口每次会话发 12 个一次性
//! 域名（6 个 A + 6 个 AAAA），所以按会话分轮：3 轮 × 12 个唯一域名。
use anyhow::{Result, bail};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

const BASE: &str = "https://dnsleak.dev";
/// 测试域名固定挂在 `t.dnsleak.dev` 区下；写死比信任响应里的 zone 字段安全。
const ZONE_SUFFIX: &str = ".t.dnsleak.dev";
/// 观察站每次会话只发 12 个域名，3 轮刚好 36 个唯一域名。
const SESSIONS: usize = 3;
const LIFETIME: Duration = Duration::from_secs(150);
/// 建会话的尝试次数：瞬时失败往往只命中一次，重试比直接报错更有用。
const START_ATTEMPTS: usize = 3;
/// 结果回读的尝试次数：解析器明细是异步写入的，第一次读到空表很正常。
const COLLECT_ATTEMPTS: usize = 3;
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36";
/// dnsleak.dev 按 Accept 做内容协商：只要带 `text/plain` 就返回终端文本。
/// 这里必须只请求 JSON，否则 /api/new 会回 “test id: …” 而不是可解析的会话。
const ACCEPT: &str = "application/json";
const ACCEPT_LANGUAGE: &str = "en-US,en;q=0.9";
static NETWORK: Semaphore = Semaphore::const_new(1);
static RUN: Lazy<Mutex<Option<Run>>> = Lazy::new(|| Mutex::new(None));

/// 一次观察会话：服务端发的会话号 + 这批一次性域名。
#[derive(Clone)]
struct Grant {
    test_id: String,
    hostnames: Vec<String>,
}
struct Run {
    id: String,
    grants: Vec<Grant>,
    started: Instant,
    /// 建会话时用的上游；结果回读必须沿用同一条链路，否则两条路各说各话。
    upstream: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Challenge {
    id: String,
    hostnames: Vec<String>,
    rounds: usize,
    per_round: usize,
    provider: &'static str,
}
#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Resolver {
    ip: String,
    family: &'static str,
    country: String,
    cc: String,
    city: String,
    operator: String,
    asn: String,
}
#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    state: &'static str,
    detail: String,
    resolvers: Vec<Resolver>,
    filtered: usize,
}

/// 跟随 WebView 的出口：系统代理开着就走它，否则直连（TUN 模式下直连同样会进隧道）。
///
/// 旧实现固定 `no_proxy()`：探测经代理、建会话却从真实网络出去，等于把真实 IP
/// 交给第三方观察站，也解释了两条链路为什么会对不上。
fn upstream_endpoint() -> Option<String> {
    crate::proxy::sysproxy::get_sysproxy()
        .ok()
        .filter(|proxy| proxy.enable && !proxy.host.is_empty() && proxy.port != 0)
        .map(|proxy| format!("http://{}:{}", proxy.host, proxy.port))
}

/// 只跟观察站自己的跳转；被引到别处说明链路被改写，停下让状态码暴露出来。
fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        let allowed = matches!(
            attempt.url().host_str(),
            Some("dnsleak.dev") | Some("www.dnsleak.dev")
        );
        if attempt.previous().len() >= 2 || !allowed {
            return attempt.stop();
        }
        attempt.follow()
    })
}
fn client(upstream: Option<&str>) -> Result<reqwest::Client> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::ACCEPT,
        reqwest::header::HeaderValue::from_static(ACCEPT),
    );
    headers.insert(
        reqwest::header::ACCEPT_LANGUAGE,
        reqwest::header::HeaderValue::from_static(ACCEPT_LANGUAGE),
    );
    let builder = reqwest::Client::builder()
        .https_only(true)
        .redirect(redirect_policy())
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(12))
        .user_agent(USER_AGENT)
        .default_headers(headers);
    let builder = match upstream {
        Some(address) => builder.proxy(reqwest::Proxy::all(address)?),
        None => builder.no_proxy(),
    };
    Ok(builder.build()?)
}
/// 回读响应正文和 content-type；只有状态码、没有正文说明不了任何事。
async fn read(client: &reqwest::Client, route: &str) -> Result<(Vec<u8>, String)> {
    let mut response = client
        .get(format!("{BASE}{route}"))
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("权威 DNS 观察服务连接失败或超时，请稍后重试"))?;
    if response.status().as_u16() == 429 {
        bail!("权威 DNS 观察服务限流，请稍后重试");
    }
    if !response.status().is_success() {
        bail!("权威 DNS 观察服务返回 HTTP {}", response.status().as_u16());
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > 262144 {
            bail!("权威 DNS 观察结果超过大小限制");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((bytes, content_type))
}

/// 把正文压成一行：报错时直接看得出是接口变了、被拦了，还是回了空包。
fn snippet(bytes: &[u8], limit: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::with_capacity(limit * 3);
    for (index, ch) in text.chars().enumerate() {
        if index >= limit {
            out.push('…');
            break;
        }
        out.push(match ch {
            '\r' | '\n' | '\t' => ' ',
            control if control.is_control() => '·',
            other => other,
        });
    }
    out.trim().to_string()
}
fn body_hint(bytes: &[u8], content_type: &str) -> String {
    let head = snippet(bytes, 120);
    let kind = if content_type.is_empty() {
        String::new()
    } else {
        format!("，{content_type}")
    };
    if head.is_empty() {
        return format!("响应 {} 字节{kind}，但正文为空", bytes.len());
    }
    format!("响应 {} 字节{kind}，正文开头：{head}", bytes.len())
}

/// 文本接口的响应（例如代理/内容协商把 JSON 降级成 text/plain）：
/// `test id: <id>` 加若干 `  <id>-a01.t.dnsleak.dev` 行。
fn parse_new_text(text: &str) -> Option<(&str, Vec<String>)> {
    let test_id = text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("test id:")
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })?;
    let hostnames = text
        .lines()
        .map(str::trim)
        .filter(|line| line.ends_with(ZONE_SUFFIX))
        .map(str::to_string)
        .collect::<Vec<_>>();
    (!hostnames.is_empty()).then(|| (test_id, hostnames))
}

/// 校验一次会话：会话号必须是纯小写字母数字，域名必须挂在本次会话号和测试区下。
fn validate_new(
    test_id: &str,
    names: &[String],
    bytes: &[u8],
    content_type: &str,
) -> Result<Grant> {
    if !(6..=64).contains(&test_id.len())
        || !test_id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    {
        bail!(
            "权威 DNS 服务返回了无效会话标识；{}",
            body_hint(bytes, content_type)
        );
    }
    if !(4..=64).contains(&names.len()) {
        bail!("权威 DNS 服务返回的测试域名数量不合法");
    }
    let prefix = format!("{test_id}-");
    let a_prefix = format!("{test_id}-a");
    let q_prefix = format!("{test_id}-q");
    let mut hostnames: Vec<String> = Vec::with_capacity(names.len());
    for host in names {
        let part = host
            .strip_prefix(prefix.as_str())
            .and_then(|rest| rest.strip_suffix(ZONE_SUFFIX))
            .unwrap_or_default();
        if part.len() != 3
            || !matches!(part.as_bytes()[0], b'a' | b'q')
            || !part.as_bytes()[1..].iter().all(u8::is_ascii_digit)
            || hostnames.iter().any(|seen| seen == host)
        {
            bail!(
                "权威 DNS 服务返回了不合规的测试域名：{}",
                snippet(host.as_bytes(), 80)
            );
        }
        hostnames.push(host.clone());
    }
    if !hostnames
        .iter()
        .any(|host| host.starts_with(a_prefix.as_str()))
        || !hostnames
            .iter()
            .any(|host| host.starts_with(q_prefix.as_str()))
    {
        bail!("权威 DNS 服务返回的测试域名缺少 A 或 AAAA 组");
    }
    Ok(Grant {
        test_id: test_id.to_string(),
        hostnames,
    })
}

fn parse_new(bytes: &[u8], content_type: &str) -> Result<Grant> {
    if let Ok(value) = serde_json::from_slice::<Value>(bytes) {
        let test_id = value["testId"].as_str().unwrap_or_default();
        let names = value["hostnames"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        return validate_new(test_id, &names, bytes, content_type);
    }
    // 兼容服务端把 JSON 降级为 text/plain 的情况；只接受带 test id 的完整文本。
    let text = String::from_utf8_lossy(bytes);
    if let Some((test_id, names)) = parse_new_text(&text) {
        return validate_new(test_id, &names, bytes, content_type);
    }
    bail!(
        "权威 DNS 服务未返回有效会话，可能是接口变化或网络拦截；{}",
        body_hint(bytes, content_type)
    )
}
/// 摊平成前端要的一轮一组：第 r 轮取 `hostnames[r * per_round ..]`。
fn challenge(id: String, grants: &[Grant]) -> Result<Challenge> {
    let per_round = grants
        .first()
        .map(|grant| grant.hostnames.len())
        .unwrap_or(0);
    if per_round == 0
        || grants
            .iter()
            .any(|grant| grant.hostnames.len() != per_round)
    {
        bail!("权威 DNS 服务返回的域名分组不一致，可能是接口变化");
    }
    Ok(Challenge {
        id,
        hostnames: grants
            .iter()
            .flat_map(|grant| grant.hostnames.iter().cloned())
            .collect(),
        rounds: grants.len(),
        per_round,
        provider: "dnsleak.dev",
    })
}

async fn new_session(http: &reqwest::Client) -> Result<Grant> {
    let mut last = anyhow::anyhow!("权威 DNS 服务未返回有效会话，可能是接口变化或网络拦截");
    for attempt in 0..START_ATTEMPTS {
        match read(http, "/api/new").await {
            Ok((bytes, content_type)) => match parse_new(&bytes, &content_type) {
                Ok(grant) => return Ok(grant),
                Err(error) => last = error,
            },
            Err(error) => {
                // 限流要退避，不该靠重试硬撞；其余失败各给一次机会。
                if format!("{error:#}").contains("限流") {
                    return Err(error);
                }
                last = error;
            }
        }
        if attempt + 1 < START_ATTEMPTS {
            tokio::time::sleep(Duration::from_millis(500 * (attempt as u64 + 1))).await;
        }
    }
    Err(last)
}

#[tauri::command]
pub async fn dns_deep_start() -> Result<Challenge, String> {
    start().await.map_err(|error| format!("{error:#}"))
}
async fn start() -> Result<Challenge> {
    let _permit = NETWORK
        .try_acquire()
        .map_err(|_| anyhow::anyhow!("上一轮 DNS 网络请求尚未结束，请稍后重试"))?;
    *RUN.lock() = None;
    let upstream = upstream_endpoint();
    let http = client(upstream.as_deref())?;
    let mut grants: Vec<Grant> = Vec::with_capacity(SESSIONS);
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for _ in 0..SESSIONS {
        match new_session(&http).await {
            Ok(grant) => {
                for host in &grant.hostnames {
                    // 一次性域名不能重复：重复说明服务端复用了会话，观察结果会串轮。
                    if !seen.insert(host.clone()) {
                        bail!("权威 DNS 服务返回了重复域名，已停止本轮检测");
                    }
                }
                grants.push(grant);
            }
            // 后续会话建不起来（多半是限流）就少跑几轮，别把整轮检测一起废掉。
            Err(error) => {
                if grants.is_empty() {
                    return Err(error);
                }
                break;
            }
        }
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    let plan = challenge(id.clone(), &grants)?;
    *RUN.lock() = Some(Run {
        id,
        grants,
        started: Instant::now(),
        upstream,
    });
    Ok(plan)
}
fn active(id: &str) -> Result<(Vec<Grant>, Option<String>)> {
    RUN.lock()
        .as_ref()
        .filter(|run| run.id == id && run.started.elapsed() < LIFETIME)
        .map(|run| (run.grants.clone(), run.upstream.clone()))
        .ok_or_else(|| anyhow::anyhow!("本轮 DNS 检测已取消或过期，请重新测试"))
}

#[tauri::command]
pub fn dns_deep_cancel(id: String) {
    let mut run = RUN.lock();
    if run.as_ref().is_some_and(|run| run.id == id) {
        *run = None;
    }
}
#[tauri::command]
pub async fn dns_deep_collect(id: String) -> Result<Report, String> {
    collect(&id).await.map_err(|error| format!("{error:#}"))
}
async fn collect(id: &str) -> Result<Report> {
    let _permit = NETWORK
        .try_acquire()
        .map_err(|_| anyhow::anyhow!("DNS 结果正在读取，请稍后重试"))?;
    let (grants, upstream) = active(id)?;
    let http = client(upstream.as_deref())?;
    let mut merged: BTreeMap<String, Resolver> = BTreeMap::new();
    let mut statuses: Vec<String> = Vec::new();
    let mut filtered = 0usize;
    let mut partial = false;
    let mut missing = 0usize;
    let mut first_error: Option<anyhow::Error> = None;
    for grant in &grants {
        match fetch_session(&http, grant).await {
            Ok(scan) => {
                filtered += scan.filtered;
                partial |= scan.incomplete;
                statuses.push(scan.status);
                for row in scan.resolvers {
                    merged.entry(row.ip.clone()).or_insert(row);
                }
            }
            Err(error) => {
                missing += 1;
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    // 全部会话都读不到才把网络错误原样抛出；部分失败降级成「结果不完整」。
    if merged.is_empty() {
        if let Some(error) = first_error {
            return Err(error);
        }
    }
    active(id)?; // 取消或新一轮开始后，不接受旧网络请求的迟到结果。
    Ok(summarize(
        merged.into_values().collect(),
        filtered,
        &statuses,
        missing,
        partial,
    ))
}

struct Scan {
    resolvers: Vec<Resolver>,
    filtered: usize,
    status: String,
    /// 观察站还没记全这批域名；明细可以先看，但不足以当结论。
    incomplete: bool,
}
async fn fetch_session(http: &reqwest::Client, grant: &Grant) -> Result<Scan> {
    let mut last: Option<Scan> = None;
    for attempt in 0..COLLECT_ATTEMPTS {
        let (bytes, content_type) = read(http, &format!("/api/result/{}", grant.test_id)).await?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
            anyhow::anyhow!(
                "权威 DNS 服务未返回有效 JSON 结果；{}",
                body_hint(&bytes, &content_type)
            )
        })?;
        let scan = parse_session(&value, grant)?;
        let ready = !scan.incomplete && !scan.resolvers.is_empty();
        last = Some(scan);
        if ready || attempt + 1 == COLLECT_ATTEMPTS {
            break;
        }
        tokio::time::sleep(Duration::from_millis(700 * (attempt as u64 + 1))).await;
    }
    last.ok_or_else(|| anyhow::anyhow!("权威 DNS 观察结果缺失"))
}

fn short(value: &Value, key: &str) -> String {
    value[key]
        .as_str()
        .unwrap_or_default()
        .chars()
        .take(120)
        .collect()
}
fn parse_session(value: &Value, grant: &Grant) -> Result<Scan> {
    if value["testId"].as_str() != Some(grant.test_id.as_str()) {
        bail!("权威 DNS 观察结果不属于本轮会话");
    }
    let rows = value["resolvers"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("权威 DNS 解析器结果缺失"))?;
    if rows.len() > 128 {
        bail!("权威 DNS 解析器数量超过限制");
    }
    let mut resolvers: Vec<Resolver> = Vec::new();
    let mut filtered = 0usize;
    for row in rows {
        // 观察站自己的出口（role=observer）不是解析器，不计也不判。
        // 老版本没有 role 字段，缺省按解析器处理；显式写了别的值才当成无效行。
        let role = row["role"].as_str().unwrap_or_default();
        if role == "observer" {
            continue;
        }
        let ip = match crate::proxy::ipmap::system_dns::public_ip(&short(row, "ip")) {
            Some(ip) if role.is_empty() || role == "resolver" => ip,
            _ => {
                filtered += 1;
                continue;
            }
        };
        if resolvers.iter().any(|seen| seen.ip == ip) {
            continue;
        }
        let family: &'static str = if ip.contains(':') { "IPv6" } else { "IPv4" };
        let raw_country = short(row, "country");
        let country = if !raw_country.is_empty() && raw_country.len() <= 2 {
            short(row, "countryName")
        } else {
            raw_country
        };
        let mut cc = short(row, "countryCode").to_ascii_uppercase();
        if cc.len() != 2 || !cc.bytes().all(|c| c.is_ascii_alphabetic()) {
            let fallback = short(row, "country").to_ascii_uppercase();
            cc = if fallback.len() == 2 && fallback.bytes().all(|c| c.is_ascii_alphabetic()) {
                fallback
            } else {
                String::new()
            };
        }
        let city = {
            let name = short(row, "city");
            if name.is_empty() {
                short(row, "cityName")
            } else {
                name
            }
        };
        let operator = {
            let name = short(row, "operator");
            if name.is_empty() {
                short(row, "org")
            } else {
                name
            }
        };
        let asn = match row["asn"].as_u64() {
            Some(number) => format!("AS{number}"),
            None => short(row, "asn"),
        };
        resolvers.push(Resolver {
            ip,
            family,
            country,
            cc,
            city,
            operator,
            asn,
        });
    }
    let status: String = value["verdict"]["status"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .take(40)
        .collect();
    let lookups = value["lookups"].as_u64().unwrap_or(0) as usize;
    let truncated = value["truncated"].as_bool().unwrap_or(false);
    // 只把「明显没记全」当不完整：一半以上域名没被观察到，才不足以判断。
    let incomplete = truncated || lookups * 2 < grant.hostnames.len();
    Ok(Scan {
        resolvers,
        filtered,
        status,
        incomplete,
    })
}

/// 汇总：任何一项证据缺失都只能落到 unknown，绝不因为「没看到境内地址」就判安全。
fn summarize(
    resolvers: Vec<Resolver>,
    filtered: usize,
    statuses: &[String],
    missing: usize,
    partial: bool,
) -> Report {
    let split = statuses
        .iter()
        .any(|status| matches!(status.as_str(), "leak" | "split"));
    let undecided = statuses.iter().any(|status| {
        !status.is_empty() && !matches!(status.as_str(), "no_leak" | "leak" | "split")
    });
    let (state, detail) = if resolvers.is_empty() {
        (
            "unknown",
            "尚未观察到有效公网解析器；不代表已加密或无泄漏".to_string(),
        )
    } else if missing > 0 {
        (
            "unknown",
            format!("{missing} 个观察会话读取失败，本轮结果不完整，请重试"),
        )
    } else if partial {
        (
            "unknown",
            "观察站没记全这批域名，本轮结果不完整，请重试".to_string(),
        )
    } else if filtered > 0 {
        (
            "unknown",
            "服务结果含无效地址或错误，结果不完整，请复核".to_string(),
        )
    } else if split {
        (
            "review",
            "观察站报告解析路径分流或泄漏风险，请复核解析器与代理规则".to_string(),
        )
    } else if resolvers.iter().any(|row| row.cc == "CN") {
        (
            "review",
            "本轮观察到境内解析器，请核对是否符合当前预期解析路径".to_string(),
        )
    } else if undecided {
        ("unknown", "观察站未给出明确结论，本轮结果未知".to_string())
    } else if resolvers.iter().any(|row| row.cc.is_empty()) {
        (
            "unknown",
            "已观察到解析器，但部分归属地未知，尚未完成核验".to_string(),
        )
    } else {
        (
            "observed",
            "本轮已观察到公网解析器，未发现境内地址；仍需结合预期代理路径判断".to_string(),
        )
    };
    Report {
        state,
        detail,
        resolvers,
        filtered,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 造一份和 /api/new 同形的会话：6 个 A + 6 个 AAAA。
    fn session_bytes(id: &str) -> Vec<u8> {
        let mut names: Vec<String> = Vec::new();
        for index in 1..=6 {
            names.push(format!("{id}-a{index:02}.t.dnsleak.dev"));
            names.push(format!("{id}-q{index:02}.t.dnsleak.dev"));
        }
        json!({"testId": id, "zone": "t.dnsleak.dev", "hostnames": names})
            .to_string()
            .into_bytes()
    }
    fn clean(value: Value, grant: &Grant) -> Scan {
        parse_session(&value, grant).unwrap()
    }

    #[test]
    fn allocates_unique_hosts_across_sessions_and_rejects_untrusted_names() {
        let grant = parse_new(&session_bytes("abc12345678"), "application/json").unwrap();
        assert_eq!(grant.hostnames.len(), 12);
        let plan = challenge("run".into(), std::slice::from_ref(&grant)).unwrap();
        assert_eq!(plan.hostnames.len(), 12);
        assert_eq!(plan.rounds, 1);
        assert_eq!(plan.per_round, 12);
        assert_eq!(plan.hostnames.iter().collect::<BTreeSet<_>>().len(), 12);

        // 印尼语首页、空包、任意域名都必须当场失败，不能当成会话号。
        assert!(
            parse_new(
                b"<!DOCTYPE html><html lang=\"id\"><head><script>",
                "text/html; charset=UTF-8"
            )
            .is_err()
        );
        assert!(parse_new(b"{}", "application/json").is_err());
        assert!(parse_new(b"   \n", "").is_err());
        let mut value: Value = serde_json::from_slice(&session_bytes("abc12345678")).unwrap();
        for bad in [
            "http://127.0.0.1/private",
            "abc12345678-a01.t.dnsleak.dev.evil.test",
            "abc12345678-a01.t.dnsleak.dev:443",
            "other12345678-a01.t.dnsleak.dev",
            "abc12345678-x01.t.dnsleak.dev",
        ] {
            value["hostnames"][0] = json!(bad);
            assert!(parse_new(value.to_string().as_bytes(), "application/json").is_err());
        }
    }
    #[test]
    fn requests_json_explicitly() {
        // dnsleak.dev 只要在 Accept 里看到 text/plain 就返回终端文本；
        // 这里必须保持单一 JSON 类型，否则启动会话永远解析失败。
        assert_eq!(ACCEPT, "application/json");
    }
    #[test]
    fn parses_text_fallback_when_content_negotiation_returns_plain_text() {
        let body = b"test id: abc12345678\n\nResolve each of these, then fetch the result URL:\n  abc12345678-a01.t.dnsleak.dev\n  abc12345678-q01.t.dnsleak.dev\n  abc12345678-a02.t.dnsleak.dev\n  abc12345678-q02.t.dnsleak.dev\n";
        let grant = parse_new(body, "text/plain; charset=utf-8").unwrap();
        assert_eq!(grant.test_id, "abc12345678");
        assert_eq!(grant.hostnames.len(), 4);
        assert!(
            grant
                .hostnames
                .iter()
                .any(|host| host.ends_with("-q02.t.dnsleak.dev"))
        );
        // 文本里没有合法域名或会话号时仍然失败，不能把任意提示页当会话。
        assert!(parse_new(b"test id: abc12345678\nno hosts here", "text/plain").is_err());
        assert!(
            parse_new(
                b"test id: bad-id\n  bad-id-a01.t.dnsleak.dev\n  bad-id-q01.t.dnsleak.dev\n  bad-id-a02.t.dnsleak.dev\n",
                "text/plain"
            )
            .is_err()
        );
    }
    #[test]
    fn keeps_only_the_head_of_a_body() {
        assert_eq!(snippet(b"a\nb\tc", 8), "a b c");
        assert_eq!(snippet(b"0123456789", 4), "0123…");
        assert_eq!(snippet(b"\r\n  \n", 8), "");
        assert!(body_hint(b"<html>", "text/html; charset=UTF-8").contains("text/html"));
        assert!(body_hint(b"", "").contains("正文为空"));
    }
    #[test]
    fn splits_resolvers_from_observer_and_private_addresses() {
        let grant = parse_new(&session_bytes("abc12345678"), "application/json").unwrap();
        let scan = clean(
            json!({
                "testId": "abc12345678",
                "lookups": 12,
                "truncated": false,
                "verdict": {"status": "no_leak", "leaking": false},
                "resolvers": [
                    {"role": "observer", "ip": "9.9.9.9", "country": "United States", "countryCode": "US"},
                    {"role": "resolver", "ip": "1.1.1.1", "country": "United States", "countryCode": "US", "operator": "Cloudflare", "city": "San Francisco", "asn": 13335},
                    {"role": "resolver", "ip": "::ffff:1.1.1.1", "country": "United States", "countryCode": "US"},
                    {"role": "resolver", "ip": "2606:4700:4700::1111", "country": "United States", "countryCode": "US"},
                    {"role": "resolver", "ip": "10.0.0.1", "country": "China", "countryCode": "CN"}
                ]
            }),
            &grant,
        );
        assert_eq!(scan.resolvers.len(), 2);
        assert_eq!(scan.filtered, 1);
        assert!(!scan.incomplete);
        assert!(scan.resolvers.iter().any(|row| row.family == "IPv6"));
        assert!(
            scan.resolvers
                .iter()
                .any(|row| row.operator == "Cloudflare" && row.asn == "AS13335")
        );
        // 有被过滤的条目就不是完整证据，只能 unknown。
        let report = summarize(
            scan.resolvers.clone(),
            scan.filtered,
            &[scan.status.clone()],
            0,
            scan.incomplete,
        );
        assert_eq!(report.state, "unknown");
    }
    #[test]
    fn never_labels_partial_empty_or_unclear_evidence_as_safe() {
        let grant = parse_new(&session_bytes("abc12345678"), "application/json").unwrap();
        let row = json!({"role": "resolver", "ip": "1.1.1.1", "country": "United States", "countryCode": "US"});
        let cases = [
            (
                json!({"testId": "abc12345678", "lookups": 12, "truncated": false, "verdict": {"status": "no_leak"}, "resolvers": []}),
                "unknown",
            ),
            (
                json!({"testId": "abc12345678", "lookups": 12, "truncated": true, "verdict": {"status": "no_leak"}, "resolvers": [row.clone()]}),
                "unknown",
            ),
            (
                json!({"testId": "abc12345678", "lookups": 12, "truncated": false, "verdict": {"status": "inconclusive"}, "resolvers": [row.clone()]}),
                "unknown",
            ),
            (
                json!({"testId": "abc12345678", "lookups": 12, "truncated": false, "verdict": {"status": "new-unknown-status"}, "resolvers": [row.clone()]}),
                "unknown",
            ),
            (
                json!({"testId": "abc12345678", "lookups": 12, "truncated": false, "verdict": {"status": "split"}, "resolvers": [row.clone()]}),
                "review",
            ),
            (
                json!({"testId": "abc12345678", "lookups": 12, "truncated": false, "verdict": {"status": "no_leak"}, "resolvers": [{"role": "resolver", "ip": "223.5.5.5", "country": "China", "countryCode": "cn"}]}),
                "review",
            ),
            (
                json!({"testId": "abc12345678", "lookups": 12, "truncated": false, "verdict": {"status": "no_leak"}, "resolvers": [row.clone()]}),
                "observed",
            ),
        ];
        for (index, (value, expected)) in cases.into_iter().enumerate() {
            let scan = clean(value, &grant);
            let report = summarize(
                scan.resolvers.clone(),
                scan.filtered,
                &[scan.status.clone()],
                0,
                scan.incomplete,
            );
            assert_eq!(report.state, expected, "case {index}");
        }
        // 会话不匹配、缺解析器数组都要报错，不能当成空结果。
        assert!(parse_session(&json!({"testId": "other12345"}), &grant).is_err());
        assert!(parse_session(&json!({"testId": "abc12345678"}), &grant).is_err());
        // 会话读取失败也降级成 unknown，不解析成安全。
        let scan = clean(
            json!({"testId": "abc12345678", "lookups": 12, "truncated": false, "verdict": {"status": "no_leak"}, "resolvers": [row.clone()]}),
            &grant,
        );
        let report = summarize(scan.resolvers.clone(), 0, &[scan.status.clone()], 1, false);
        assert_eq!(report.state, "unknown");
    }
    #[test]
    fn refuses_sessions_with_different_host_counts() {
        let long = Grant {
            test_id: "a12345678".into(),
            hostnames: vec![
                "a12345678-a01.t.dnsleak.dev".into(),
                "a12345678-q01.t.dnsleak.dev".into(),
            ],
        };
        let short = Grant {
            test_id: "b12345678".into(),
            hostnames: vec!["b12345678-a01.t.dnsleak.dev".into()],
        };
        assert!(challenge("run".into(), &[long, short]).is_err());
        assert!(challenge("run".into(), &[]).is_err());
    }
    #[test]
    fn stale_cancel_cannot_clear_a_new_run_and_old_collect_is_rejected() {
        *RUN.lock() = Some(Run {
            id: "new".into(),
            grants: vec![Grant {
                test_id: "valid12345678".into(),
                hostnames: vec!["valid12345678-a01.t.dnsleak.dev".into()],
            }],
            started: Instant::now(),
            upstream: Some("http://127.0.0.1:7890".into()),
        });
        assert!(active("old").is_err());
        dns_deep_cancel("old".into());
        assert!(active("new").is_ok());
        assert_eq!(
            active("new").unwrap().1,
            Some(String::from("http://127.0.0.1:7890"))
        );
        dns_deep_cancel("new".into());
        assert!(active("new").is_err());
        *RUN.lock() = Some(Run {
            id: "expired".into(),
            grants: vec![],
            started: Instant::now() - LIFETIME,
            upstream: None,
        });
        assert!(active("expired").is_err());
        *RUN.lock() = None;
    }
}
