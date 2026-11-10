//! 额度窗口与状态文案：把扫描结果换算成界面需要的窗口和一句话说明。

use super::model::*;
use super::scan::*;
// ---------- 额度快照 → 通用额度窗口 ----------

/// rollout 里的 rate_limits（used_percent / window_minutes / resets_at）转通用窗口
pub(crate) fn rate_from_snapshot(s: &Option<RateSnapshot>, now: i64) -> AiRateLimit {
    let Some(s) = s else {
        return AiRateLimit::default();
    };
    let win = |v: &Option<serde_json::Value>| -> Option<AiRateWindow> {
        let v = v.as_ref()?;
        let used = v.get("used_percent").and_then(|x| x.as_f64())?;
        Some(AiRateWindow {
            utilization: used / 100.0,
            reset_at: v
                .get("resets_at")
                .and_then(|x| x.as_i64())
                .filter(|r| *r > 0),
            limit_window_seconds: v
                .get("window_minutes")
                .and_then(|x| x.as_u64())
                .unwrap_or(0)
                * 60,
            reset_after_seconds: v
                .get("resets_at")
                .and_then(|x| x.as_i64())
                .map(|r| (r - now).max(0) as u64)
                .unwrap_or(0),
        })
    };
    AiRateLimit {
        primary: win(&s.primary),
        secondary: win(&s.secondary),
        captured_at: (s.captured_at > 0).then_some(s.captured_at),
        account_verified: false,
        metrics: Vec::new(),
    }
}

/// Claude 官方 `/api/oauth/usage` 的原始窗口 → AI 概览的额度结构。
///
/// 响应里的 `utilization` 是百分数（100 = 已用满），而前端窗口统一使用 0–1，
/// 因此这里必须除以 100；缺失或非法窗口直接省略，不能补成 0%。
pub(crate) fn claude_rate_from_usage(value: &serde_json::Value, now: i64) -> AiRateLimit {
    let window = |name: &str, seconds: u64| -> Option<AiRateWindow> {
        let raw = value.get(name)?;
        if raw.is_null() {
            return None;
        }
        let utilization = raw
            .get("utilization")
            .and_then(|x| x.as_f64())
            .filter(|x| x.is_finite() && *x >= 0.)?;
        let reset_at = raw
            .get("resets_at")
            .and_then(|x| x.as_str())
            .map(rfc3339_to_unix)
            .filter(|at| *at > 0);
        Some(AiRateWindow {
            utilization: utilization / 100.,
            limit_window_seconds: seconds,
            reset_at,
            reset_after_seconds: reset_at.map(|at| (at - now).max(0) as u64).unwrap_or(0),
        })
    };
    let primary = window("five_hour", 5 * 3600);
    let secondary = window("seven_day", 7 * 86400);
    let has_window = primary.is_some() || secondary.is_some();
    AiRateLimit {
        metrics: Vec::new(),
        primary,
        secondary,
        captured_at: has_window.then_some(now),
        account_verified: true,
    }
}

// ---------- 状态文案 ----------

pub(crate) fn human_tokens(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.2}B", n as f64 / 1e9)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else if n >= 1_000 {
        format!("{}K", n / 1_000)
    } else {
        format!("{n}")
    }
}

pub(crate) fn activity_desc(last: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let mins = ((now - last).max(0) / 60) as u64;
    if mins < 1 {
        "刚刚有新会话".into()
    } else if mins < 60 {
        format!("最近 {mins} 分钟前活跃")
    } else if mins < 60 * 24 {
        format!("最近 {} 小时前活跃", mins / 60)
    } else {
        format!("最近 {} 天前活跃", mins / 60 / 24)
    }
}

pub(crate) fn items_note(last_ts: i64, all: &AiUsage) -> String {
    if last_ts <= 0 || all.total == 0 {
        "本机暂无该工具的会话记录（仅统计本地数据）".into()
    } else {
        format!(
            "共 {} 个会话 · 累计 {} tokens · {}",
            all.sessions,
            human_tokens(all.total),
            activity_desc(last_ts)
        )
    }
}
