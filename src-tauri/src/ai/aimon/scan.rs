//! 本地会话扫描：Claude Code 与 Codex 的 jsonl 逐行累计、去重与窗口聚合。

use super::model::*;
use crate::ai::aimon::activity::{AiSession, SessionFacts};
use chrono::Datelike;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt};
pub(crate) fn home() -> PathBuf {
    PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default())
}

pub(crate) fn window_bounds() -> (i64, i64, i64) {
    let now = chrono::Local::now();
    let now_ts = now.timestamp();
    let today0 = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(now.timezone())
        .map(|t| t.timestamp())
        .single()
        .unwrap_or(now_ts);
    let mon0 = (now.date_naive()
        - chrono::Duration::days(now.weekday().num_days_from_monday() as i64))
    .and_hms_opt(0, 0, 0)
    .unwrap()
    .and_local_timezone(now.timezone())
    .map(|t| t.timestamp())
    .single()
    .unwrap_or(now_ts);
    (today0, mon0, now_ts)
}

pub(crate) fn rfc3339_to_unix(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|d| d.timestamp())
        .unwrap_or(0)
}

// ---------- base64url（无外部依赖，解析 JWT claims 用） ----------

pub(crate) const B64TAB: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
pub(crate) fn b64url_decode(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0u32;
    for ch in input.bytes() {
        let v = B64TAB.iter().position(|&t| t == ch)? as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xFF) as u8);
        }
    }
    Some(out)
}

pub(crate) fn jwt_claims(token: &str) -> Option<serde_json::Value> {
    let mid = token.split('.').nth(1)?;
    let bytes = b64url_decode(mid)?;
    serde_json::from_slice(&bytes).ok()
}

pub(crate) fn claims_str(claims: Option<&serde_json::Value>, path: &[&str]) -> Option<String> {
    let mut cur = claims?;
    for k in path {
        cur = cur.get(k)?;
    }
    cur.as_str().map(|s| s.to_string())
}

/// Engine-backed tools only need lifecycle evidence from native readers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum ScanMode {
    #[default]
    Usage,
    Activity,
}
impl ScanMode {
    pub(crate) fn counts(self) -> bool {
        self == Self::Usage
    }
}
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct NativePlan(
    pub(crate) std::collections::HashSet<String>,
    pub(crate) crate::ai::agent_tools::MonitoringScope,
);
impl NativePlan {
    pub(crate) fn mode(&self, key: &str) -> ScanMode {
        if self.0.contains(key) {
            ScanMode::Activity
        } else {
            ScanMode::Usage
        }
    }
    pub(crate) fn enabled(&self, key: &str) -> bool {
        self.1.contains(key)
    }
}

// ---------- 通用遍历：收集一组 jsonl，逐行回调 ----------

#[derive(Clone, Default)]
pub(crate) struct WindowAgg {
    pub(crate) today: AiUsage,
    pub(crate) week: AiUsage,
    pub(crate) last7: AiUsage,
    pub(crate) last30: AiUsage,
    pub(crate) all: AiUsage,
    pub(crate) last_ts: i64,
    pub(crate) incomplete_files: u64,
    pub(crate) daily: std::collections::BTreeMap<i64, AiUsage>,
    pub(crate) sessions: Vec<AiSession>,
}

// 缓存日累计和读取位置，不保存消息正文；时区变化后重建日桶。
#[derive(Default)]
pub(crate) struct ClaudeFileScan {
    pub(crate) size: u64,
    pub(crate) offset: u64,
    pub(crate) modified: Option<std::time::SystemTime>,
    pub(crate) created: Option<std::time::SystemTime>,
    pub(crate) prefix: Option<FilePrefix>,
    pub(crate) days: std::collections::BTreeMap<i64, [u64; 5]>,
    pub(crate) last_ts: i64,
    pub(crate) read_bytes: u64,
    pub(crate) activity: SessionFacts,
    pub(crate) mode: ScanMode,
    pub(crate) messages: std::collections::HashMap<String, (i64, [u64; 5])>,
}
#[derive(Default)]
pub(crate) struct ClaudeCache {
    pub(crate) timezone: String,
    pub(crate) mode: ScanMode,
    pub(crate) files: std::collections::HashMap<PathBuf, ClaudeFileScan>,
}
#[derive(PartialEq)]
pub(crate) struct FilePrefix {
    pub(crate) len: usize,
    pub(crate) hash: [u8; 32],
}
pub(crate) async fn file_prefix(path: &std::path::Path, limit: usize) -> Option<FilePrefix> {
    let file = tokio::fs::File::open(path).await.ok()?;
    let mut bytes = Vec::with_capacity(limit);
    file.take(limit as u64).read_to_end(&mut bytes).await.ok()?;
    let mut hash = [0; 32];
    hash.copy_from_slice(ring::digest::digest(&ring::digest::SHA256, &bytes).as_ref());
    Some(FilePrefix {
        len: bytes.len(),
        hash,
    })
}
pub(crate) fn local_day(timestamp: i64) -> i64 {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(timestamp, 0)
        .single()
        .and_then(|time| time.date_naive().and_hms_opt(0, 0, 0))
        .and_then(|day| chrono::Local.from_local_datetime(&day).earliest())
        .map(|day| day.timestamp())
        .unwrap_or(0)
}
pub(crate) async fn scan_claude_file(
    path: &std::path::Path,
    prior: Option<ClaudeFileScan>,
    mode: ScanMode,
) -> Option<ClaudeFileScan> {
    let meta = tokio::fs::metadata(path).await.ok()?;
    let modified = meta.modified().ok();
    let created = meta.created().ok();
    let mut scan = prior.filter(|scan| scan.mode == mode).unwrap_or_default();
    scan.read_bytes = 0;
    if scan.size == meta.len() && scan.modified == modified && scan.created == created {
        return Some(scan);
    }
    let appended = scan.size < meta.len()
        && scan.created == created
        && scan.offset <= meta.len()
        && file_prefix(
            path,
            scan.prefix.as_ref().map(|prefix| prefix.len).unwrap_or(0),
        )
        .await
        .as_ref()
            == scan.prefix.as_ref();
    if !appended {
        scan = ClaudeFileScan::default();
    }
    scan.mode = mode;
    if scan.prefix.is_none() {
        scan.prefix = Some(file_prefix(path, 4096).await?);
    }
    let mut file = tokio::fs::File::open(path).await.ok()?;
    file.seek(std::io::SeekFrom::Start(scan.offset))
        .await
        .ok()?;
    let mut reader = tokio::io::BufReader::with_capacity(256 * 1024, file);
    let mut line = String::new();
    loop {
        line.clear();
        let count = reader.read_line(&mut line).await.ok()?;
        scan.read_bytes += count as u64;
        if count == 0 || !line.ends_with('\n') {
            break;
        }
        scan.offset += count as u64;
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        scan.activity.claude(&value);
        if !mode.counts() {
            continue;
        }
        if let Some((i, o, cr, cw, ts)) = claude_usage(&value) {
            let mut values = [
                i,
                o,
                cr,
                cw,
                i.saturating_add(o).saturating_add(cr).saturating_add(cw),
            ];
            let mut day_ts = local_day(ts);
            // 流式消息可能重复携带同一 message.id 的累计 usage，仅计新增部分。
            {
                if let Some(id) = value
                    .pointer("/message/id")
                    .and_then(|id| id.as_str())
                    .filter(|id| !id.is_empty())
                {
                    let previous = scan
                        .messages
                        .entry(id.to_owned())
                        .or_insert((day_ts, [0; 5]));
                    day_ts = previous.0;
                    for n in 0..5 {
                        let current = values[n];
                        values[n] = current.saturating_sub(previous.1[n]);
                        previous.1[n] = previous.1[n].max(current);
                    }
                }
            }
            if ts > 0 {
                let day = scan.days.entry(day_ts).or_default();
                for n in 0..5 {
                    day[n] += values[n];
                }
            } else {
                let day = scan.days.entry(0).or_default();
                for n in 0..5 {
                    day[n] += values[n];
                }
            }
            scan.last_ts = scan.last_ts.max(ts);
        }
    }
    scan.size = meta.len();
    scan.modified = modified;
    scan.created = created;
    Some(scan)
}
pub(crate) async fn collect_claude_at(dirs: Vec<PathBuf>, cache: &mut ClaudeCache) -> WindowAgg {
    let (today, week, _) = window_bounds();
    let mut agg = WindowAgg::default();
    let (last7, last30) = (lookback_start(7), lookback_start(30));
    let mut seen = std::collections::HashSet::new();
    let mut stack = dirs;
    while let Some(dir) = stack.pop() {
        let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let Ok(kind) = entry.file_type().await else {
                agg.incomplete_files += 1;
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                stack.push(path);
                continue;
            }
            if !kind.is_file() || path.extension().and_then(|name| name.to_str()) != Some("jsonl") {
                continue;
            }
            seen.insert(path.clone());
            let prior = cache.files.remove(&path);
            match scan_claude_file(&path, prior, cache.mode).await {
                Some(scan) => {
                    cache.files.insert(path, scan);
                }
                None => agg.incomplete_files += 1,
            }
        }
    }
    cache.files.retain(|path, _| seen.contains(path));
    for (path, scan) in &cache.files {
        let mut active = [false; 5];
        for (day, values) in &scan.days {
            add_codex_usage(&mut agg.all, values);
            active[0] = true;
            if *day >= today {
                add_codex_usage(&mut agg.today, values);
                active[1] = true;
            }
            if *day >= week {
                add_codex_usage(&mut agg.week, values);
                active[2] = true;
            }
            if *day >= last7 {
                add_codex_usage(&mut agg.last7, values);
                active[3] = true;
            }
            if *day >= last30 {
                add_codex_usage(&mut agg.last30, values);
                active[4] = true;
            }
            let daily = agg.daily.entry(*day).or_default();
            add_codex_usage(daily, values);
            daily.sessions += 1;
        }
        agg.all.sessions += u64::from(active[0]);
        agg.today.sessions += u64::from(active[1]);
        agg.week.sessions += u64::from(active[2]);
        agg.last7.sessions += u64::from(active[3]);
        agg.last30.sessions += u64::from(active[4]);
        agg.last_ts = agg.last_ts.max(scan.last_ts);
        if let Some(session) = scan
            .activity
            .snapshot("claude", path, agg_session_total(&scan.days))
        {
            agg.sessions.push(session);
        }
    }
    agg
}
pub(crate) async fn collect_claude(mode: ScanMode) -> WindowAgg {
    static CACHE: once_cell::sync::Lazy<tokio::sync::Mutex<ClaudeCache>> =
        once_cell::sync::Lazy::new(Default::default);
    let timezone = crate::platform::system_time::timezone_key().unwrap_or_default();
    let mut cache = CACHE.lock().await;
    if cache.timezone != timezone || cache.mode != mode {
        cache.files.clear();
        cache.timezone = timezone;
        cache.mode = mode;
    }
    collect_claude_at(vec![home().join(".claude/projects")], &mut cache).await
}

// ---------- Claude Code ----------

pub(crate) fn claude_usage(v: &serde_json::Value) -> Option<(u64, u64, u64, u64, i64)> {
    if v.get("type")?.as_str()? != "assistant" {
        return None;
    }
    let u = v.pointer("/message/usage")?;
    if !u.is_object() {
        return None;
    }
    let n = |k: &str| u.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    let ts = v
        .get("timestamp")
        .and_then(|x| x.as_str())
        .map(rfc3339_to_unix)
        .unwrap_or(0);
    Some((
        n("input_tokens"),
        n("output_tokens"),
        n("cache_read_input_tokens"),
        n("cache_creation_input_tokens"),
        ts,
    ))
}

pub(crate) async fn read_claude_account() -> AiAccount {
    let mut acc = AiAccount::default();
    let Ok(txt) = tokio::fs::read_to_string(home().join(".claude.json")).await else {
        return acc;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) else {
        return acc;
    };
    let Some(oa) = v.get("oauthAccount").filter(|x| x.is_object()) else {
        return acc;
    };
    let email = oa
        .get("emailAddress")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    let billing = oa
        .get("billingType")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let org_tier = oa
        .get("organizationRateLimitTier")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let billing_type = if billing.is_empty() {
        None
    } else {
        Some(billing.clone())
    };
    let (plan, plan_label) = classify_claude_plan(&billing, &org_tier);
    let name = oa
        .get("displayName")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| email.clone());
    acc.email = email;
    acc.name = name;
    acc.workspace = oa
        .get("organizationName")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    acc.billing_type = billing_type;
    acc.plan = Some(plan);
    acc.plan_label = Some(plan_label);
    acc.switchable = false;
    acc
}

/// Claude 套餐启发式：organizationRateLimitTier / billingType 组合
pub(crate) fn classify_claude_plan(billing: &str, org_tier: &str) -> (String, String) {
    if org_tier.contains("claude_code") {
        ("claude_code".into(), "Claude Code 席位".into())
    } else if billing.contains("subscription") {
        ("claude_pro".into(), "Pro 订阅".into())
    } else if billing.contains("api") {
        ("api".into(), "API 计费".into())
    } else if org_tier.is_empty() {
        ("unknown".into(), "套餐未知".into())
    } else {
        (org_tier.to_string(), org_tier.to_string())
    }
}

// ---------- Codex ----------

pub(crate) async fn read_codex_auth() -> Option<serde_json::Value> {
    let txt = tokio::fs::read_to_string(home().join(".codex").join("auth.json"))
        .await
        .ok()?;
    serde_json::from_str(&txt).ok()
}

pub(crate) async fn read_codex_account() -> AiAccount {
    let mut acc = AiAccount::default();
    let Some(v) = read_codex_auth().await else {
        return acc;
    };
    let Some(tokens) = v.get("tokens") else {
        // 仅 API key 模式
        return acc;
    };
    let claims = tokens
        .get("id_token")
        .and_then(|t| t.as_str())
        .and_then(jwt_claims);
    let email = claims_str(claims.as_ref(), &["email"]);
    let name = claims_str(claims.as_ref(), &["name"]);
    let plan = claims_str(
        claims.as_ref(),
        &["https://api.openai.com/auth", "chatgpt_plan_type"],
    );
    acc.email = email;
    acc.name = name;
    acc.plan = plan.clone();
    acc.plan_label = Some(plan_label(plan.as_deref()));
    acc.workspace = claims_str(
        claims.as_ref(),
        &["https://api.openai.com/auth", "chatgpt_account_type"],
    );
    acc.switchable = false;
    acc
}

pub(crate) fn plan_label(plan: Option<&str>) -> String {
    match plan {
        Some("free") => "免费版".into(),
        Some("plus") => "Plus 套餐".into(),
        Some("pro") => "Pro 套餐".into(),
        Some("max") => "Max 套餐".into(),
        Some("team") => "Team 套餐".into(),
        Some("business") => "Business 套餐".into(),
        Some("enterprise") => "Enterprise 套餐".into(),
        Some(other) => other.to_string(),
        None => "无 AI key / 未登录".into(),
    }
}

/// Codex token_count 事件解析：
/// - total_token_usage 是会话内累计值 → 相邻两次的差值才是本行增量（fork 继承的 base 不计）
/// - payload.rate_limits 是官方额度窗口快照（5h / 周），本地直接可用，无需远程刷新
#[derive(Debug, Clone, Default)]
pub(crate) struct CodexTokCount {
    pub(crate) cum: [u64; 5], // input, output, cached_read, cache_write, total (缓存包含在 input 中)
    pub(crate) ts: i64,
    pub(crate) ordinal: Option<u64>,
    pub(crate) rate: Option<RateSnapshot>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RateSnapshot {
    pub(crate) primary: Option<serde_json::Value>,
    pub(crate) secondary: Option<serde_json::Value>,
    pub(crate) captured_at: i64,
}

pub(crate) fn find_obj<'a>(
    v: &'a serde_json::Value,
    paths: &[&str],
) -> Option<&'a serde_json::Value> {
    paths
        .iter()
        .find_map(|p| v.pointer(p).filter(|x| x.is_object()))
}

pub(crate) fn num_of(v: &serde_json::Value, k: &str) -> u64 {
    v.get(k).and_then(|x| x.as_u64()).unwrap_or(0)
}

pub(crate) fn usage_values(usage: &serde_json::Value) -> [u64; 5] {
    [
        num_of(usage, "input_tokens"),
        num_of(usage, "output_tokens"),
        num_of(usage, "cached_input_tokens"),
        num_of(usage, "cache_write_input_tokens"),
        usage
            .get("total_tokens")
            .and_then(|n| n.as_u64())
            .unwrap_or_else(|| {
                num_of(usage, "input_tokens").saturating_add(num_of(usage, "output_tokens"))
            }),
    ]
}

pub(crate) fn codex_usage(v: &serde_json::Value) -> Option<CodexTokCount> {
    let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
    if ty != "event_msg" && ty != "token_count" && ty != "response_item" {
        return None;
    }
    let usage = find_obj(
        &v,
        &[
            "/payload/info/total_token_usage",
            "/payload/total_token_usage",
            "/info/total_token_usage",
            "/total_token_usage",
        ],
    )?;
    let rate = codex_rate(&v);
    Some(CodexTokCount {
        cum: usage_values(usage),
        ts: v
            .get("timestamp")
            .and_then(|x| x.as_str())
            .map(rfc3339_to_unix)
            .unwrap_or(0),
        ordinal: v.get("ordinal").and_then(|v| v.as_u64()),
        rate,
    })
}

pub(crate) fn codex_rate(v: &serde_json::Value) -> Option<RateSnapshot> {
    find_obj(
        &v,
        &[
            "/payload/info/rate_limits",
            "/payload/rate_limits",
            "/info/rate_limits",
            "/rate_limits",
        ],
    )
    .map(|r| RateSnapshot {
        primary: r.get("primary").filter(|x| x.is_object()).cloned(),
        secondary: r.get("secondary").filter(|x| x.is_object()).cloned(),
        captured_at: v
            .get("timestamp")
            .and_then(|x| x.as_str())
            .map(rfc3339_to_unix)
            .unwrap_or(0),
    })
}

/// 仅缓存计数事件及读取位置，不保留提示词、消息正文或凭据。
/// 初次按事件流扫描，后续仅读取追加部分，跨午夜会话按每次事件时间归属窗口。
#[derive(Default)]
pub(crate) struct CodexFileScan {
    pub(crate) created: Option<std::time::SystemTime>,
    pub(crate) prefix: Option<FilePrefix>,
    pub(crate) summary: Option<(i64, i64, WindowAgg)>,
    pub(crate) size: u64,
    pub(crate) offset: u64,
    pub(crate) modified: Option<std::time::SystemTime>,
    pub(crate) previous: [u64; 5],
    pub(crate) is_fork: bool,
    pub(crate) thread_id: Option<String>,
    pub(crate) own_history: bool,
    pub(crate) copied_meta: bool,
    pub(crate) local_start: Option<u64>,
    pub(crate) legacy_seen: bool,
    pub(crate) unverified: bool,
    pub(crate) records: std::collections::HashMap<String, (i64, [u64; 5])>,
    pub(crate) metadata_seen: bool,
    pub(crate) events: Vec<(i64, [u64; 5])>,
    pub(crate) last_ts: i64,
    pub(crate) rate: Option<(i64, RateSnapshot)>,
    pub(crate) activity: SessionFacts,
    pub(crate) mode: ScanMode,
}

pub(crate) async fn codex_scan_file(
    path: &std::path::Path,
    prior: Option<CodexFileScan>,
    mode: ScanMode,
) -> Option<CodexFileScan> {
    let meta = tokio::fs::metadata(path).await.ok()?;
    let modified = meta.modified().ok();
    let created = meta.created().ok();
    let mut scan = prior.filter(|scan| scan.mode == mode).unwrap_or_default();
    if scan.size == meta.len() && scan.modified == modified && scan.created == created {
        return Some(scan);
    }
    let appended = scan.size < meta.len()
        && scan.created == created
        && scan.offset <= meta.len()
        && file_prefix(
            path,
            scan.prefix.as_ref().map(|prefix| prefix.len).unwrap_or(0),
        )
        .await
        .as_ref()
            == scan.prefix.as_ref();
    if !appended {
        scan = CodexFileScan::default();
    }
    scan.mode = mode;
    if scan.prefix.is_none() {
        scan.prefix = Some(file_prefix(path, 4096).await?);
    }
    scan.created = created;
    scan.summary = None;
    let mut file = tokio::fs::File::open(path).await.ok()?;
    file.seek(std::io::SeekFrom::Start(scan.offset))
        .await
        .ok()?;
    let mut reader = tokio::io::BufReader::with_capacity(256 * 1024, file);
    let mut line = String::new();
    loop {
        line.clear();
        let count = reader.read_line(&mut line).await.ok()?;
        if count == 0 {
            break;
        }
        // 留下尚未写完的一行，下一次从行首读取，避免吞掉追加中的事件。
        if !line.ends_with('\n') {
            break;
        }
        scan.offset += count as u64;
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if v["type"] == "session_meta" {
            scan.activity.codex(&v, false);
            let id = v
                .pointer("/payload/id")
                .and_then(|s| s.as_str())
                .map(str::to_owned);
            if !scan.metadata_seen {
                scan.metadata_seen = true;
                scan.thread_id = id;
                scan.is_fork = v
                    .pointer("/payload/forked_from_id")
                    .and_then(|s| s.as_str())
                    .is_some();
                scan.local_start = v
                    .pointer("/payload/subagent_history_start_ordinal")
                    .and_then(|v| v.as_u64());
                scan.own_history = !scan.is_fork && scan.local_start.is_none();
            } else if id.is_some() && id != scan.thread_id {
                scan.copied_meta = true;
            }
        }
        // Copied parent metadata cannot establish ownership; only the local settings boundary can.
        if scan.is_fork
            && scan.copied_meta
            && v.pointer("/payload/type").and_then(|v| v.as_str())
                == Some("thread_settings_applied")
            && v.pointer("/payload/thread_id").and_then(|v| v.as_str()) == scan.thread_id.as_deref()
            && scan.thread_id.is_some()
        {
            scan.own_history = true;
        }
        let inherited = match scan.local_start {
            Some(start) => v["ordinal"].as_u64().is_none_or(|n| n < start),
            None => !scan.own_history,
        };
        let foreign = v["type"] == "token_usage_record"
            && v.pointer("/payload/thread_id").and_then(|id| id.as_str())
                != scan.thread_id.as_deref();
        scan.activity.codex(&v, inherited || foreign);
        if !mode.counts() {
            if !inherited {
                if let Some(rate) = codex_rate(&v) {
                    scan.rate = Some((rate.captured_at, rate));
                }
            }
            continue;
        }
        if v["type"] == "token_usage_record" {
            let payload = &v["payload"];
            if scan.thread_id.is_some()
                && payload["thread_id"].as_str() == scan.thread_id.as_deref()
            {
                if let (Some(id), Some(usage)) = (
                    payload["response_id"].as_str().filter(|id| !id.is_empty()),
                    payload["usage"].as_object(),
                ) {
                    let usage = serde_json::Value::Object(usage.clone());
                    let ts = v["timestamp"].as_str().map(rfc3339_to_unix).unwrap_or(0);
                    if ts == 0 {
                        scan.unverified = true;
                    }
                    scan.records
                        .entry(id.to_string())
                        .or_insert((ts, usage_values(&usage)));
                } else {
                    scan.unverified = true;
                }
            } else if scan.thread_id.is_none() {
                scan.unverified = true;
            }
        }
        let Some(event) = codex_usage(&v) else {
            continue;
        };
        scan.legacy_seen = true;
        let inherited = match scan.local_start {
            Some(start) => match event.ordinal {
                Some(ordinal) => ordinal < start,
                None => true,
            },
            None => !scan.own_history,
        };
        let mut delta = [0; 5];
        for i in 0..5 {
            delta[i] = event.cum[i].saturating_sub(scan.previous[i]);
        }
        scan.previous = event.cum;
        if !inherited {
            if scan.local_start.is_some() {
                scan.own_history = true;
            }
            if delta.iter().any(|n| *n > 0) {
                scan.events.push((event.ts, delta));
            }
            scan.last_ts = scan.last_ts.max(event.ts);
            if let Some(rate) = event.rate {
                scan.rate = Some((event.ts, rate));
            }
        }
    }
    scan.size = meta.len();
    scan.modified = modified;
    Some(scan)
}

pub(crate) fn add_codex_usage(usage: &mut AiUsage, delta: &[u64; 5]) {
    usage.input += delta[0];
    usage.output += delta[1];
    usage.cached += delta[2];
    usage.cache_write += delta[3];
    usage.total += delta[4];
}

pub(crate) fn aggregate_codex(scan: &CodexFileScan, agg: &mut WindowAgg, today: i64, week: i64) {
    let mut active = [false; 5];
    let mut active_days = std::collections::HashSet::new();
    let (last7, last30) = (lookback_start(7), lookback_start(30));
    let mixed_incomplete = !scan.records.is_empty()
        && !scan.events.is_empty()
        && scan
            .records
            .values()
            .map(|(_, values)| values[4])
            .sum::<u64>()
            != scan.events.iter().map(|(_, values)| values[4]).sum::<u64>();
    agg.incomplete_files += u64::from(
        (!scan.own_history && scan.legacy_seen)
            || mixed_incomplete
            || scan.unverified
            || !scan.metadata_seen,
    );
    let mut add = |(timestamp, delta): &(i64, [u64; 5])| {
        add_codex_usage(&mut agg.all, delta);
        active[0] = true;
        if *timestamp >= today {
            add_codex_usage(&mut agg.today, delta);
            active[1] = true;
        }
        if *timestamp >= week {
            add_codex_usage(&mut agg.week, delta);
            active[2] = true;
        }
        if *timestamp >= last7 {
            add_codex_usage(&mut agg.last7, delta);
            active[3] = true;
        }
        if *timestamp >= last30 {
            add_codex_usage(&mut agg.last30, delta);
            active[4] = true;
        }
        agg.last_ts = agg.last_ts.max(*timestamp);
        if *timestamp > 0 {
            let day = local_day(*timestamp);
            add_codex_usage(agg.daily.entry(day).or_default(), delta);
            active_days.insert(day);
        }
    };
    if scan.records.is_empty() {
        for event in &scan.events {
            add(event);
        }
    } else {
        for event in scan.records.values() {
            add(event);
        }
    }
    agg.all.sessions += u64::from(active[0]);
    agg.today.sessions += u64::from(active[1]);
    agg.week.sessions += u64::from(active[2]);
    agg.last7.sessions += u64::from(active[3]);
    agg.last30.sessions += u64::from(active[4]);
    agg.last_ts = agg.last_ts.max(scan.last_ts);
    for day in active_days {
        agg.daily.entry(day).or_default().sessions += 1;
    }
}
pub(crate) fn merge_aggregate(target: &mut WindowAgg, source: &WindowAgg) {
    for (target, source) in [
        (&mut target.all, &source.all),
        (&mut target.today, &source.today),
        (&mut target.week, &source.week),
        (&mut target.last7, &source.last7),
        (&mut target.last30, &source.last30),
    ] {
        merge_usage(target, source);
    }
    target.last_ts = target.last_ts.max(source.last_ts);
    target.incomplete_files += source.incomplete_files;
    for (day, usage) in &source.daily {
        merge_usage(target.daily.entry(*day).or_default(), usage);
    }
}

pub(crate) fn merge_usage(target: &mut AiUsage, source: &AiUsage) {
    target.input += source.input;
    target.output += source.output;
    target.cached += source.cached;
    target.cache_write += source.cache_write;
    target.total += source.total;
    target.sessions += source.sessions;
}
pub(crate) fn lookback_start(days: i64) -> i64 {
    use chrono::TimeZone;
    let date = chrono::Local::now().date_naive() - chrono::Duration::days(days - 1);
    date.and_hms_opt(0, 0, 0)
        .and_then(|d| chrono::Local.from_local_datetime(&d).earliest())
        .map(|d| d.timestamp())
        .unwrap_or(0)
}
pub(crate) fn agg_session_total(days: &std::collections::BTreeMap<i64, [u64; 5]>) -> u64 {
    days.values().map(|values| values[4]).sum()
}
pub(crate) fn daily_usage(agg: &WindowAgg) -> Vec<AiDailyUsage> {
    let start = lookback_start(30);
    agg.daily
        .iter()
        .filter(|(day, _)| **day >= start)
        .filter_map(|(day, usage)| {
            chrono::DateTime::from_timestamp(*day, 0).map(|ts| AiDailyUsage {
                date: ts
                    .with_timezone(&chrono::Local)
                    .format("%Y-%m-%d")
                    .to_string(),
                usage: usage.clone(),
            })
        })
        .collect()
}
pub(crate) fn recent_sessions(agg: &WindowAgg) -> Vec<AiSession> {
    let mut sessions = agg.sessions.clone();
    let cutoff = chrono::Utc::now().timestamp() - 30 * 86400;
    sessions.retain(|session| session.last_activity_at >= cutoff);
    sessions.sort_by(|a, b| {
        b.last_activity_at
            .cmp(&a.last_activity_at)
            .then(a.id.cmp(&b.id))
    });
    sessions.dedup_by(|a, b| a.id == b.id);
    sessions.truncate(30);
    sessions
}

pub(crate) async fn collect_codex(mode: ScanMode) -> (WindowAgg, Option<RateSnapshot>) {
    static CACHE: once_cell::sync::Lazy<
        tokio::sync::Mutex<std::collections::HashMap<PathBuf, CodexFileScan>>,
    > = once_cell::sync::Lazy::new(Default::default);
    static TIMEZONE: once_cell::sync::Lazy<parking_lot::Mutex<String>> =
        once_cell::sync::Lazy::new(Default::default);
    let mut cache = CACHE.lock().await;
    {
        let current = crate::platform::system_time::timezone_key().unwrap_or_default();
        let mut previous = TIMEZONE.lock();
        if *previous != current {
            for scan in cache.values_mut() {
                scan.summary = None;
            }
            *previous = current;
        }
    }
    let mut seen = std::collections::HashSet::new();
    let mut agg = WindowAgg::default();
    let mut best_rate: Option<(i64, RateSnapshot)> = None;
    let (today, week, _) = window_bounds();
    let mut stack = vec![
        home().join(".codex").join("sessions"),
        home().join(".codex").join("archived_sessions"),
    ];
    while let Some(dir) = stack.pop() {
        let Ok(mut rd) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = rd.next_entry().await {
            let path = entry.path();
            let Ok(kind) = entry.file_type().await else {
                continue;
            };
            if kind.is_dir() {
                stack.push(path);
                continue;
            }
            if !kind.is_file() || path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            seen.insert(path.clone());
            let prior = cache.remove(&path);
            let Some(scan) = codex_scan_file(&path, prior, mode).await else {
                agg.incomplete_files += 1;
                continue;
            };
            cache.insert(path, scan);
        }
    }
    cache.retain(|path, _| seen.contains(path));
    // 同一 canonical thread 的复制文件只统计最长的一份，避免重复文件重复收费。
    let mut canonical: std::collections::HashMap<String, PathBuf> =
        std::collections::HashMap::new();
    for (path, scan) in cache.iter() {
        let key = scan
            .thread_id
            .clone()
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        canonical
            .entry(key)
            .and_modify(|existing| {
                if scan.size > cache[existing].size {
                    *existing = path.clone();
                }
            })
            .or_insert_with(|| path.clone());
    }
    for path in canonical.values() {
        let scan = cache.get_mut(path).unwrap();
        if scan
            .summary
            .as_ref()
            .is_none_or(|(t, w, _)| *t != today || *w != week)
        {
            let mut summary = WindowAgg::default();
            if mode.counts() {
                aggregate_codex(scan, &mut summary, today, week);
            }
            scan.summary = Some((today, week, summary));
        }
        merge_aggregate(&mut agg, &scan.summary.as_ref().unwrap().2);
        if let Some(session) =
            scan.activity
                .snapshot("chatgpt", path, scan.summary.as_ref().unwrap().2.all.total)
        {
            agg.sessions.push(session);
        }
        if let Some((ts, rate)) = &scan.rate {
            if best_rate.as_ref().is_none_or(|(t, _)| ts >= t) {
                best_rate = Some((*ts, rate.clone()));
            }
        }
    }
    (agg, best_rate.map(|(_, rate)| rate))
}
