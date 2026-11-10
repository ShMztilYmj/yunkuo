use serde::{Deserialize, Serialize};

pub const CONTINUATION: &str = "上一轮因额度耗尽中断，现在额度已恢复。请结合本会话记录和当前项目状态，继续之前尚未完成的任务，避免重复已完成的操作；沿用原有权限，遇到需要我决定的问题时暂停。";

pub fn digest(text: impl AsRef<[u8]>) -> String {
    crate::platform::digest::sha256_lower(text.as_ref())
}
pub fn valid_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|v| v.to_string() == id)
}
pub fn label(text: &str, limit: usize) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect()
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Watch {
    pub thread_id: String,
    pub home_key: String,
    pub title: String,
    pub project: String,
    pub account: String,
    pub configuration: String,
    pub enabled: bool,
    pub consumed: bool,
    pub created_at: i64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recovery {
    pub id: String,
    pub thread_id: String,
    pub home_key: String,
    pub turn_id: String,
    pub ordinal: i64,
    pub account: String,
    pub configuration: String,
    pub title: String,
    pub project: String,
    pub state: String,
    pub reason: String,
    pub marker: String,
    pub reset_at: Option<i64>,
    pub next_check: i64,
    pub checked_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub claimed_at: Option<i64>,
    pub queue_id: Option<String>,
    pub cancel_requested: bool,
    pub spawn_failures: u32,
    #[serde(default)]
    pub quota_confirmed: bool,
}
impl Recovery {
    pub fn in_flight(&self) -> bool {
        self.claimed_at.is_some()
            && matches!(
                self.state.as_str(),
                "submitting" | "queued" | "submission_unknown" | "running"
            )
    }
    pub fn waiting(&self) -> bool {
        self.claimed_at.is_none()
            && matches!(
                self.state.as_str(),
                "waiting_reset"
                    | "waiting_quota"
                    | "waiting_thread"
                    | "waiting_environment"
                    | "waiting_history"
                    | "paused_identity"
                    | "paused_configuration"
            )
    }
    pub fn set(&mut self, state: &str, reason: &str, now: i64, delay: i64) {
        self.state = state.into();
        self.reason = reason.into();
        self.updated_at = now;
        self.next_check = now + delay;
    }
    /// 推进到下一个等待状态的副本。
    ///
    /// 两个驱动原本各写一份同样的"克隆 → set → 保存"；时钟与保存入口仍归调用方，
    /// 因为 claude 保存时会额外通知前端、codex 不会。
    pub fn advance(&self, state: &str, reason: &str, delay: i64) -> Self {
        let mut next = self.clone();
        next.set(state, reason, crate::workspace::resume::now(), delay);
        next
    }
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Compatibility {
    pub supported: bool,
    pub engine_version: Option<String>,
    pub message: String,
    pub checked_at: Option<i64>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub thread_id: String,
    pub title: String,
    pub project: String,
    pub quota_stopped: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchView {
    pub thread_id: String,
    pub title: String,
    pub project: String,
    pub enabled: bool,
    pub consumed: bool,
}
impl From<Watch> for WatchView {
    fn from(w: Watch) -> Self {
        Self {
            thread_id: w.thread_id,
            title: w.title,
            project: w.project,
            enabled: w.enabled,
            consumed: w.consumed,
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryView {
    pub id: String,
    pub thread_id: String,
    pub title: String,
    pub project: String,
    pub state: String,
    pub reason: String,
    pub reset_at: Option<i64>,
    pub checked_at: Option<i64>,
    pub updated_at: i64,
    pub cancel_requested: bool,
    pub may_be_queued: bool,
}
impl From<Recovery> for RecoveryView {
    fn from(r: Recovery) -> Self {
        let may_be_queued = r.in_flight();
        Self {
            id: r.id,
            thread_id: r.thread_id,
            title: r.title,
            project: r.project,
            state: r.state,
            reason: r.reason,
            reset_at: r.reset_at,
            checked_at: r.checked_at,
            updated_at: r.updated_at,
            cancel_requested: r.cancel_requested,
            may_be_queued,
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub recovery_id: String,
    pub at: i64,
    pub state: String,
    pub reason: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub enabled: bool,
    pub compatibility: Compatibility,
    pub watches: Vec<WatchView>,
    pub recoveries: Vec<RecoveryView>,
    pub events: Vec<Event>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Quota {
    pub available: Option<bool>,
    pub reset_at: Option<i64>,
}
// Conservative across all exposed buckets: an unidentified blocking window cannot authorize a send.
pub fn quota(value: &serde_json::Value, now: i64) -> Quota {
    let unknown = Quota {
        available: None,
        reset_at: None,
    };
    let buckets: Vec<_> = if let Some(map) = value["rateLimitsByLimitId"]
        .as_object()
        .filter(|m| !m.is_empty())
    {
        map.values().collect()
    } else if value["rateLimits"].is_object() {
        vec![&value["rateLimits"]]
    } else {
        return unknown;
    };
    let mut windows = 0;
    let mut blocked = Vec::new();
    let mut explicit = false;
    for bucket in buckets {
        if !bucket.is_object() {
            return unknown;
        }
        if bucket["spendControlReached"] == true {
            return Quota {
                available: Some(false),
                reset_at: None,
            };
        }
        explicit |= bucket["rateLimitReachedType"]
            .as_str()
            .is_some_and(|s| !s.is_empty());
        for key in ["primary", "secondary"] {
            let w = &bucket[key];
            if w.is_null() {
                continue;
            }
            let Some(used) = w["usedPercent"]
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.)
            else {
                return unknown;
            };
            if !w["windowDurationMins"].as_i64().is_some_and(|n| n > 0) {
                return unknown;
            }
            windows += 1;
            if used >= 100. {
                blocked.push(
                    w["resetsAt"]
                        .as_i64()
                        .filter(|n| *n > now && *n < now + 370 * 86400),
                );
            }
        }
    }
    if !blocked.is_empty() {
        let reset_at = if blocked.iter().all(Option::is_some) {
            blocked.into_iter().flatten().max()
        } else {
            None
        };
        Quota {
            available: Some(false),
            reset_at,
        }
    } else if explicit {
        Quota {
            available: Some(false),
            reset_at: None,
        }
    } else if windows > 0 {
        Quota {
            available: Some(true),
            reset_at: None,
        }
    } else {
        unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn only_fresh_complete_windows_authorize_sending() {
        let w = |used, reset| json!({"usedPercent":used,"windowDurationMins":300,"resetsAt":reset});
        assert_eq!(quota(&json!({}), 100).available, None);
        assert_eq!(
            quota(&json!({"rateLimits":{"primary":w(20,200)}}), 100).available,
            Some(true)
        );
        assert_eq!(
            quota(&json!({"rateLimits":{"primary":w(100,90)}}), 100).available,
            Some(false)
        );
        assert_eq!(
            quota(
                &json!({"rateLimits":{"primary":w(100,200),"secondary":w(100,500)}}),
                100
            )
            .reset_at,
            Some(500)
        );
        assert_eq!(quota(&json!({"rateLimitsByLimitId":{"codex":{"primary":w(20,200)},"other":{"primary":w(100,500)}}}),100).available,Some(false));
        assert_eq!(
            quota(&json!({"rateLimits":{"primary":{"usedPercent":0}}}), 100).available,
            None
        );
    }
}
