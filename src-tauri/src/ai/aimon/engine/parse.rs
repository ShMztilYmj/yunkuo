//! 引擎输出的解析、聚合与投影：只处理已捕获的 JSON，不持有进程或缓存状态。

use super::super::activity::AiSession;
use super::super::*;
use super::ResultSet;
use super::process::antigravity_cache;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
};

pub(super) fn key(client: &str) -> Option<&'static str> {
    crate::ai::agent_roster::key_for_client(client)
}
pub(super) fn text(v: &Value, name: &str) -> String {
    v[name]
        .as_str()
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(160)
        .collect()
}
pub(super) fn numbers(v: &Value, client: &str) -> Option<AiUsage> {
    let input = v["input"].as_u64()?;
    let output = v["output"].as_u64()?;
    let cached = v["cacheRead"].as_u64()?;
    let cache_write = v["cacheWrite"].as_u64()?;
    let reasoning = if matches!(
        client,
        "codex"
            | "antigravity"
            | "antigravity-cli"
            | "antigravity-extension"
            | "dsh"
            | "grok"
            | "zcode"
            | "opencode"
    ) {
        v["reasoning"].as_u64().unwrap_or(0)
    } else {
        0
    };
    let output = output.saturating_add(reasoning);
    Some(AiUsage {
        input,
        output,
        cached,
        cache_write,
        total: input
            .saturating_add(output)
            .saturating_add(cached)
            .saturating_add(cache_write),
        sessions: 0,
    })
}
pub(super) fn add(to: &mut AiUsage, from: &AiUsage) {
    to.input = to.input.saturating_add(from.input);
    to.output = to.output.saturating_add(from.output);
    to.cached = to.cached.saturating_add(from.cached);
    to.cache_write = to.cache_write.saturating_add(from.cache_write);
    to.total = to.total.saturating_add(from.total);
}
pub(super) fn report(value: &Value) -> Result<HashMap<String, (AiUsage, Vec<AiSession>)>, String> {
    let entries = value["entries"].as_array().ok_or("用量报表结构不兼容")?;
    let metadata: HashMap<_, _> = value["sessions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| Some(((v["client"].as_str()?, v["sessionId"].as_str()?), v)))
        .collect();
    let now = chrono::Utc::now().timestamp();
    let mut tools: HashMap<String, (AiUsage, BTreeMap<String, AiSession>)> = HashMap::new();
    for row in entries {
        let client = row["client"].as_str().unwrap_or_default();
        let Some(tool) = key(client) else { continue };
        let counts = numbers(row, client).ok_or("用量报表包含无效计数")?;
        let (total, sessions) = tools.entry(tool.into()).or_default();
        add(total, &counts);
        let raw = text(row, "sessionId");
        if raw.is_empty() {
            continue;
        }
        let meta = metadata
            .get(&(client, raw.as_str()))
            .copied()
            .unwrap_or(&Value::Null);
        // Parent metadata means the tokens belong in totals but the child is not a second main session.
        if meta["parentSessionId"]
            .as_str()
            .is_some_and(|v| !v.is_empty())
        {
            continue;
        }
        let ts = meta["lastActiveMs"].as_i64().unwrap_or(0) / 1000;
        let model = text(row, "model");
        let session = sessions.entry(raw.clone()).or_insert_with(|| AiSession {
            id: format!("{tool}:{raw}"),
            tool: tool.into(),
            project: text(row, "workspaceLabel"),
            model: None,
            state: "unknown".into(),
            last_activity_at: if ts > 0 && ts <= now + 60 { ts } else { 0 },
            completed_at: None,
            total: 0,
            usage_available: Some(true),
        });
        if !model.is_empty() {
            session.model = Some(model);
        }
        session.total = session.total.saturating_add(counts.total);
    }
    Ok(tools
        .into_iter()
        .map(|(key, (mut usage, sessions))| {
            usage.sessions = sessions.len() as u64;
            (key, (usage, sessions.into_values().collect()))
        })
        .collect())
}
pub(super) fn graph(value: &Value) -> Result<HashMap<String, Vec<AiDailyUsage>>, String> {
    let days = value["contributions"]
        .as_array()
        .ok_or("每日用量结构不兼容")?;
    let mut result: HashMap<String, BTreeMap<String, AiUsage>> = HashMap::new();
    for day in days {
        let date = text(day, "date");
        if chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d").is_err() {
            return Err("每日用量日期无效".into());
        }
        for row in day["clients"].as_array().ok_or("每日用量来源缺失")? {
            let client = row["client"].as_str().unwrap_or_default();
            let Some(tool) = key(client) else { continue };
            let counts = numbers(&row["tokens"], client).ok_or("每日用量计数无效")?;
            add(
                result
                    .entry(tool.into())
                    .or_default()
                    .entry(date.clone())
                    .or_default(),
                &counts,
            );
        }
    }
    Ok(result
        .into_iter()
        .map(|(key, days)| {
            (
                key,
                days.into_iter()
                    .map(|(date, usage)| AiDailyUsage { date, usage })
                    .collect(),
            )
        })
        .collect())
}
pub(super) fn quotas(value: &Value, now: i64) -> HashMap<String, Vec<AiQuotaMetric>> {
    let mut result: HashMap<String, Vec<AiQuotaMetric>> = HashMap::new();
    for row in value.as_array().into_iter().flatten() {
        let provider = text(row, "provider");
        let tool = match provider.as_str() {
            "Claude" => "claude",
            "Codex" => "chatgpt",
            "Cursor" => "cursor",
            "Antigravity" => "antigravity",
            "Kimi" => "kimi",
            "Grok" => "grok",
            "OpenCode Go" => "opencode",
            _ => continue,
        };
        // Multi-account results retain their account label and are never silently assigned to the active local login.
        let label = row
            .pointer("/account/label")
            .and_then(Value::as_str)
            .or_else(|| row["email"].as_str())
            .unwrap_or_default();
        let label: String = label
            .chars()
            .filter(|c| !c.is_control())
            .take(160)
            .collect();
        for metric in row["metrics"].as_array().into_iter().flatten().take(32) {
            let Some(used) = metric["used_percent"]
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.)
            else {
                continue;
            };
            let remaining = metric["remaining_percent"]
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.);
            result.entry(tool.into()).or_default().push(AiQuotaMetric {
                label: text(metric, "label"),
                used_percent: used,
                remaining_percent: remaining,
                remaining_label: metric["remaining_label"]
                    .as_str()
                    .map(|_| text(metric, "remaining_label")),
                reset_at: metric["resets_at"]
                    .as_str()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|t| t.timestamp()),
                account_label: label.clone(),
                plan: text(row, "plan"),
                captured_at: now,
            });
        }
    }
    result
}
pub(super) fn bucket_timezone(base: &Path) -> Option<chrono_tz::Tz> {
    let path = antigravity_cache(base).parent()?.join("settings.json");
    if std::fs::metadata(&path).ok()?.len() > 4 * 1024 * 1024 {
        return None;
    }
    let settings: Value = serde_json::from_reader(std::fs::File::open(path).ok()?).ok()?;
    settings
        .pointer("/scanner/bucketTimezone")?
        .as_str()?
        .trim()
        .parse()
        .ok()
}
pub(super) fn bucket_day(
    time: chrono::DateTime<chrono::Utc>,
    zone: Option<chrono_tz::Tz>,
) -> chrono::NaiveDate {
    zone.map(|zone| time.with_timezone(&zone).date_naive())
        .unwrap_or_else(|| time.with_timezone(&chrono::Local).date_naive())
}
pub(super) fn period_usage(
    days: &[AiDailyUsage],
    sessions: &[AiSession],
    start: chrono::NaiveDate,
    end: chrono::NaiveDate,
    zone: Option<chrono_tz::Tz>,
) -> AiUsage {
    let (start_key, end_key) = (start.to_string(), end.to_string());
    let mut usage = AiUsage::default();
    for day in days
        .iter()
        .filter(|d| d.date >= start_key && d.date <= end_key)
    {
        add(&mut usage, &day.usage);
    }
    usage.sessions = sessions
        .iter()
        .filter(|session| {
            chrono::DateTime::from_timestamp(session.last_activity_at, 0)
                .filter(|_| session.last_activity_at > 0)
                .is_some_and(|time| {
                    let day = bucket_day(time, zone);
                    day >= start && day <= end
                })
        })
        .count() as u64;
    usage
}
pub(super) fn apply(items: &mut [AiUsageItem], result: &ResultSet) {
    for item in items {
        if let Some(engine) = result.items.iter().find(|e| e.key == item.key) {
            let mut replacement = engine.clone();
            // Lifecycle read problems do not invalidate the engine's independent Token ledger.
            replacement.account = item.account.clone();
            replacement.rate = item.rate.clone();
            // DSH 模型源额度来自独立探测，引擎的账本替换不能把它丢掉。
            replacement.providers = std::mem::take(&mut item.providers);
            // A token timestamp cannot manufacture a running/completed lifecycle event.
            for native in &item.recent_sessions {
                if let Some(session) = replacement
                    .recent_sessions
                    .iter_mut()
                    .find(|s| s.id == native.id)
                {
                    let total = session.total;
                    let model = session.model.clone();
                    *session = native.clone();
                    session.total = total;
                    session.usage_available = Some(true);
                    if session.model.is_none() {
                        session.model = model;
                    }
                } else {
                    replacement.recent_sessions.push(native.clone());
                }
            }
            replacement
                .recent_sessions
                .sort_by_key(|s| std::cmp::Reverse(s.last_activity_at));
            replacement.recent_sessions.truncate(100);
            replacement.session_available = !replacement.recent_sessions.is_empty();
            *item = replacement;
        }
        if !result.items.iter().any(|e| e.key == item.key)
            && matches!(item.key.as_str(), "cursor" | "antigravity")
        {
            item.note = if item.key == "cursor" {
                "尚未读取到 Cursor 账号用量；请在 Cursor 官方应用登录后同步"
            } else {
                "尚未发现 Antigravity 可核验的本地用量；使用后同步刷新"
            }
            .into();
            item.complete = false;
        }
        if let Some(quota) = result.quota.get(&item.key) {
            item.rate.metrics = quota.clone();
        }
    }
}
