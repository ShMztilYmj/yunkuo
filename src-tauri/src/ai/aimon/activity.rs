//! 本地日志的会话事实。时间来自事件，重新扫描不能延长运行状态。
//! 生命周期规则参考 Agent HUD（Apache-2.0），不保存对话正文或工具参数。
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSession {
    pub id: String,
    pub tool: String,
    pub project: String,
    pub model: Option<String>,
    pub state: String,
    pub last_activity_at: i64,
    pub completed_at: Option<i64>,
    pub total: u64,
    /// None 兼容旧日志；Some(false) 仅有状态，无可核验 Token 数据。
    pub usage_available: Option<bool>,
}

#[derive(Default)]
pub(crate) struct SessionFacts {
    id: Option<String>,
    project: String,
    model: Option<String>,
    state: &'static str,
    last_event: i64,
    completed_at: Option<i64>,
    subagent: bool,
    metadata_seen: bool,
}

fn text(value: &Value, path: &str) -> Option<String> {
    value
        .pointer(path)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(160).collect())
}
fn folder(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .chars()
        .take(80)
        .collect()
}

impl SessionFacts {
    fn observe(&mut self, value: &Value) -> Option<i64> {
        let ts = value["timestamp"]
            .as_str()
            .map(super::rfc3339_to_unix)
            .filter(|ts| *ts > 0)?;
        if ts < self.last_event {
            return None;
        }
        self.last_event = ts;
        if let Some(cwd) = value["cwd"].as_str() {
            self.project = folder(cwd);
        }
        Some(ts)
    }
    fn begin(&mut self) {
        self.state = "running";
        self.completed_at = None;
    }
    fn end(&mut self, state: &'static str, ts: i64) {
        self.state = state;
        self.completed_at = (state == "completed").then_some(ts);
    }
    pub(super) fn claude(&mut self, value: &Value) {
        if value["isSidechain"].as_bool() == Some(true) {
            self.subagent = true;
        }
        if self.id.is_none() {
            self.id = text(value, "/sessionId");
        }
        let Some(ts) = self.observe(value) else {
            return;
        };
        if self.subagent {
            return;
        }
        match value["type"].as_str().unwrap_or("") {
            "user" => {
                if value["isMeta"].as_bool() == Some(true) {
                    return;
                }
                if let Some(content) = value.pointer("/message/content").and_then(Value::as_str) {
                    if content.starts_with("[Request interrupted") {
                        self.end("ended", ts);
                    } else if !content.trim_start().starts_with(['/', '<']) {
                        self.begin();
                    }
                }
                // tool_result 属于已有轮次，不开始新轮次。
                else if value
                    .pointer("/message/content")
                    .and_then(Value::as_array)
                    .is_some_and(|blocks| {
                        blocks.iter().any(|block| {
                            block["type"] == "text"
                                && block["text"]
                                    .as_str()
                                    .is_some_and(|s| !s.trim_start().starts_with(['/', '<']))
                        })
                    })
                {
                    self.begin();
                }
            }
            "assistant" => {
                if value.pointer("/message/model").and_then(Value::as_str) == Some("<synthetic>") {
                    return;
                }
                if let Some(model) = text(value, "/message/model") {
                    self.model = Some(model);
                }
                match value
                    .pointer("/message/stop_reason")
                    .and_then(Value::as_str)
                {
                    Some("end_turn" | "stop_sequence") => self.end("completed", ts),
                    Some("tool_use") => self.begin(),
                    _ => {}
                }
            }
            "system"
                if matches!(
                    value["subtype"].as_str(),
                    Some("stop_hook_summary" | "turn_duration")
                ) =>
            {
                if self.state == "running" {
                    self.end("ended", ts);
                }
            }
            _ => {}
        }
    }
    pub(super) fn codex(&mut self, value: &Value, inherited: bool) {
        let payload = &value["payload"];
        if value["type"] == "session_meta" {
            if !self.metadata_seen {
                self.metadata_seen = true;
                self.id = text(value, "/payload/id");
                if let Some(cwd) = payload["cwd"].as_str() {
                    self.project = folder(cwd);
                }
                self.subagent = payload.pointer("/source/subagent").is_some()
                    || payload["parent_thread_id"].is_string();
            }
            return;
        }
        if inherited || self.subagent {
            return;
        }
        let Some(ts) = self.observe(value) else {
            return;
        };
        if value["type"] == "turn_context" {
            if let Some(model) = text(value, "/payload/model") {
                self.model = Some(model);
            }
        }
        if value["type"] != "event_msg" {
            return;
        }
        match payload["type"].as_str().unwrap_or("") {
            "task_started" => self.begin(),
            "task_complete" => self.end(
                if payload.get("error").is_some_and(|error| !error.is_null()) {
                    "ended"
                } else {
                    "completed"
                },
                ts,
            ),
            "turn_aborted" => self.end("ended", ts),
            _ => {}
        }
    }
    pub(super) fn deepseek(&mut self, value: &Value) {
        let kind = value["type"].as_str().unwrap_or_default();
        if kind == "session" {
            self.id = text(value, "/id");
            if let Some(cwd) = value["cwd"].as_str() {
                self.project = folder(cwd);
            }
            self.subagent = value
                .get("parentSession")
                .is_some_and(|parent| !parent.is_null())
                || value["delegationDepth"].as_u64().unwrap_or(0) > 0;
            return;
        }
        if !matches!(
            kind,
            "turn/start"
                | "turn/end"
                | "step/start"
                | "assistant/message"
                | "tool/call"
                | "tool/result"
                | "approval/asked"
                | "approval/decided"
        ) {
            return;
        }
        let time = value["time"]
            .as_i64()
            .map(|time| {
                if time > 100_000_000_000 {
                    time / 1000
                } else {
                    time
                }
            })
            .unwrap_or(0);
        if time <= 0 || time < self.last_event {
            return;
        }
        self.last_event = time;
        if let Some(model) = text(value, "/data/message/source/model") {
            self.model = Some(model);
        }
        match kind {
            "turn/start" | "step/start" => self.begin(),
            "approval/asked" => {
                self.state = "asking";
                self.completed_at = None;
            }
            "approval/decided" => self.begin(),
            "turn/end" => self.end(
                if value.pointer("/data/reason/kind").and_then(Value::as_str) == Some("completed") {
                    "completed"
                } else {
                    "ended"
                },
                time,
            ),
            _ => {}
        }
    }
    pub(super) fn snapshot(
        &self,
        tool: &str,
        path: &std::path::Path,
        total: u64,
    ) -> Option<AiSession> {
        self.snapshot_at(tool, path, total, chrono::Utc::now().timestamp())
    }
    fn snapshot_at(
        &self,
        tool: &str,
        path: &std::path::Path,
        total: u64,
        now: i64,
    ) -> Option<AiSession> {
        if self.subagent || self.last_event <= 0 {
            return None;
        }
        let fallback = || {
            let digest =
                ring::digest::digest(&ring::digest::SHA256, path.to_string_lossy().as_bytes());
            digest.as_ref()[..8]
                .iter()
                .map(|n| format!("{n:02x}"))
                .collect::<String>()
        };
        let id = self.id.clone().unwrap_or_else(fallback);
        let state = if self.state.is_empty()
            || (matches!(self.state, "running" | "asking")
                && now.saturating_sub(self.last_event) >= 1800)
        {
            "unknown"
        } else {
            self.state
        };
        Some(AiSession {
            id: format!("{tool}:{id}"),
            tool: tool.into(),
            project: if self.project.is_empty() {
                "本地会话".into()
            } else {
                self.project.clone()
            },
            model: self.model.clone(),
            state: state.into(),
            last_activity_at: self.last_event,
            completed_at: self.completed_at,
            total,
            usage_available: Some(total > 0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(kind: &str, seconds: u64) -> Value {
        serde_json::json!({"type":"event_msg", "timestamp":format!("2026-10-18T10:00:{seconds:02}Z"), "payload":{"type":kind}})
    }
    #[test]
    fn codex_requires_terminal_event_and_reading_does_not_refresh_evidence() {
        let mut facts = SessionFacts::default();
        facts.codex(&event("task_started", 0), false);
        facts.codex(&event("token_count", 10), false);
        let at = super::super::rfc3339_to_unix("2026-10-18T10:00:10Z");
        let path = std::path::Path::new("private/path.jsonl");
        let fresh = facts.snapshot_at("chatgpt", path, 100, at).unwrap();
        assert_eq!(fresh.state, "running");
        assert_eq!(fresh.completed_at, None);
        assert_eq!(
            facts
                .snapshot_at("chatgpt", path, 100, at + 1801)
                .unwrap()
                .state,
            "unknown"
        );
        assert_eq!(facts.last_event, at);
        facts.codex(&event("task_complete", 15), true);
        assert_eq!(facts.state, "running");
        facts.codex(&event("turn_aborted", 20), false);
        assert_eq!(facts.state, "ended");
        assert_eq!(facts.completed_at, None);
        facts.codex(&event("task_started", 30), false);
        facts.codex(&event("task_complete", 40), false);
        assert_eq!(facts.state, "completed");
        assert!(facts.completed_at.is_some());
        assert!(!serde_json::to_string(&fresh).unwrap().contains("private"));
    }
    #[test]
    fn claude_ignores_local_commands_and_subagents() {
        let mut facts = SessionFacts::default();
        let user = serde_json::json!({"type":"user", "timestamp":"2026-10-18T10:00:00Z", "message":{"content":"/model"}});
        facts.claude(&user);
        assert_ne!(facts.state, "running");
        let mut user = user;
        user["message"]["content"] = "hello".into();
        facts.claude(&user);
        assert_eq!(facts.state, "running");
        facts.claude(&serde_json::json!({"type":"assistant", "timestamp":"2026-10-18T10:01:00Z", "message":{"stop_reason":"tool_use"}}));
        assert_eq!(facts.state, "running");
        facts.claude(&serde_json::json!({"type":"assistant", "timestamp":"2026-10-18T10:02:00Z", "message":{"stop_reason":"end_turn"}}));
        assert_eq!(facts.state, "completed");
        facts.claude(&serde_json::json!({"type":"user", "isSidechain":true,"timestamp":"2026-10-18T10:03:00Z"}));
        assert!(
            facts
                .snapshot("claude", std::path::Path::new("fixture"), 0)
                .is_none()
        );
    }
}
