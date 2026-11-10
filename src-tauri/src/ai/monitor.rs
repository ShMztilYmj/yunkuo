//! 可关闭的本地完成提醒。初次扫描只建基线，不补发历史提醒。
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use tauri::{Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

#[derive(Default, Serialize, Deserialize)]
struct Preferences {
    enabled: bool,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    id: u64,
    message: String,
    created_at: i64,
    #[serde(skip)]
    tool: String,
}
struct Completion {
    tool: String,
    message: String,
}
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    today_tokens: u64,
    running: usize,
    scanned_at: Option<i64>,
    partial: bool,
}
#[derive(Default)]
struct CompletionTracker {
    seen: HashMap<String, i64>,
    baseline_at: Option<i64>,
}
impl CompletionTracker {
    fn observe(&mut self, items: &[crate::ai::aimon::AiUsageItem], now: i64) -> Vec<Completion> {
        let mut messages = Vec::new();
        let mut current = HashMap::new();
        for item in items {
            for session in &item.recent_sessions {
                let identity = format!("{}:{}", item.key, session.id);
                let completed = session.completed_at.unwrap_or(0);
                let previous = self
                    .seen
                    .get(&identity)
                    .copied()
                    .unwrap_or(self.baseline_at.unwrap_or(now))
                    .max(current.get(&identity).copied().unwrap_or(0));
                if self.baseline_at.is_some()
                    && completed > previous
                    && completed >= now - 120
                    && completed <= now + 60
                    && session.state == "completed"
                {
                    let tool = crate::ai::agent_tools::label(&item.key);
                    messages.push(Completion {
                        tool: item.key.clone(),
                        message: format!("{tool} · {} 已完成一轮", session.project),
                    });
                }
                current.insert(identity, completed.max(previous));
            }
        }
        // 只保留采集范围内的主会话，丢失的历史不能在重新出现时补发。
        self.seen = current;
        self.baseline_at = Some(now);
        messages
    }
}
#[derive(Default)]
pub struct Monitor {
    enabled: bool,
    generation: u64,
    tracker: CompletionTracker,
    notices: VecDeque<Notice>,
    next_id: u64,
    summary: Summary,
    delivery_error: Option<String>,
}
impl Monitor {
    fn set_scope(&mut self, scope: &crate::ai::agent_tools::MonitoringScope) {
        self.generation = self.generation.wrapping_add(1);
        self.tracker = CompletionTracker::default();
        self.notices.retain(|notice| scope.contains(&notice.tool));
        self.summary = Summary::default();
    }
}
pub type State = Mutex<Monitor>;
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    enabled: bool,
    notices: Vec<Notice>,
    summary: Summary,
    delivery_error: Option<String>,
}

#[tauri::command]
pub fn ai_monitor_status(state: tauri::State<'_, State>) -> Status {
    let state = state.lock();
    Status {
        enabled: state.enabled,
        notices: state.notices.iter().cloned().collect(),
        summary: state.summary.clone(),
        delivery_error: state.delivery_error.clone(),
    }
}
#[tauri::command]
pub fn ai_monitor_set_enabled(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    enabled: bool,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("请在主界面设置会话提醒".into());
    }
    let mut state = app.state::<State>().inner().lock();
    let path = crate::data_dir(&app)
        .map_err(|e| e.to_string())?
        .join("ai-monitor.json");
    crate::config::atomic_write(
        &path,
        &serde_json::to_vec(&Preferences { enabled }).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    state.enabled = enabled;
    state.generation += 1;
    state.tracker = CompletionTracker::default();
    state.delivery_error = None;
    if !enabled {
        state.summary = Summary::default();
    }
    drop(state);
    let _ = app.emit("ai-monitor-changed", ());
    Ok(())
}
#[tauri::command]
pub fn ai_monitor_dismiss(state: tauri::State<'_, State>, id: u64) {
    state.lock().notices.retain(|notice| notice.id != id);
}

pub fn scope_changed(app: &tauri::AppHandle) {
    let managed = app.state::<State>();
    let mut state = managed.lock();
    state.set_scope(&crate::ai::agent_tools::MonitoringScope::current(app));
    drop(state);
    let _ = app.emit("ai-monitor-changed", ());
}

pub fn start(app: &tauri::AppHandle) {
    if let Ok(path) = crate::data_dir(app).map(|dir| dir.join("ai-monitor.json")) {
        if let Ok(bytes) = std::fs::read(path) {
            match serde_json::from_slice::<Preferences>(&bytes) {
                Ok(prefs) => app.state::<State>().lock().enabled = prefs.enabled,
                Err(_) => {
                    app.state::<State>().lock().delivery_error =
                        Some("提醒设置无法读取，已保持关闭，请重新设置。".into())
                }
            }
        }
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let (enabled, generation) = {
                let state = app.state::<State>();
                let state = state.lock();
                (state.enabled, state.generation)
            };
            if enabled {
                if let Ok(snapshot) = crate::ai::aimon::ai_usage_status(app.clone()).await {
                    let now = chrono::Utc::now().timestamp();
                    let messages = {
                        let managed = app.state::<State>();
                        let mut state = managed.lock();
                        if state.enabled && state.generation == generation {
                            let messages = state.tracker.observe(&snapshot.items, now);
                            state.summary = Summary {
                                today_tokens: snapshot
                                    .items
                                    .iter()
                                    .filter(|item| item.usage_available)
                                    .map(|item| item.today.total)
                                    .sum(),
                                running: snapshot
                                    .items
                                    .iter()
                                    .flat_map(|item| &item.recent_sessions)
                                    .filter(|session| session.state == "running")
                                    .count(),
                                scanned_at: Some(now),
                                partial: snapshot
                                    .items
                                    .iter()
                                    .any(|item| item.usage_available && !item.complete),
                            };
                            for message in &messages {
                                state.next_id += 1;
                                let id = state.next_id;
                                state.notices.push_front(Notice {
                                    id,
                                    message: message.message.clone(),
                                    created_at: now,
                                    tool: message.tool.clone(),
                                });
                                state.notices.truncate(20);
                            }
                            messages
                        } else {
                            Vec::new()
                        }
                    };
                    if !messages.is_empty() {
                        let body = if messages.len() == 1 {
                            messages[0].message.clone()
                        } else {
                            format!(
                                "{} 个 AI 会话完成了新一轮，打开 AI 统计查看。",
                                messages.len()
                            )
                        };
                        // 系统通知失败时保留应用内记录，不影响 CLI 执行。
                        let still_enabled = {
                            let state = app.state::<State>();
                            let state = state.lock();
                            state.enabled && state.generation == generation
                        };
                        if still_enabled {
                            let failure = app
                                .notification()
                                .builder()
                                .title("云阔 · 会话完成")
                                .body(body)
                                .show()
                                .err()
                                .map(|_| "系统通知未能送达，可在此查看完成记录。".to_owned());
                            app.state::<State>().lock().delivery_error = failure;
                        }
                    }
                    let _ = app.emit("ai-monitor-changed", ());
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(completed: i64) -> crate::ai::aimon::AiUsageItem {
        let session: crate::ai::aimon::activity::AiSession =
            crate::ai::aimon::activity::AiSession {
                id: "fixture".into(),
                tool: "claude".into(),
                project: "demo".into(),
                model: None,
                state: "completed".into(),
                last_activity_at: completed,
                completed_at: Some(completed),
                total: 1,
                usage_available: Some(true),
            };
        crate::ai::aimon::AiUsageItem {
            key: "claude".into(),
            usage_available: true,
            recent_sessions: vec![session],
            ..Default::default()
        }
    }
    #[test]
    fn removed_tools_leave_notices_summary_and_completion_baseline() {
        let mut state = Monitor {
            generation: 1,
            summary: Summary {
                today_tokens: 999,
                running: 2,
                ..Default::default()
            },
            ..Default::default()
        };
        for tool in ["claude", "deepseek"] {
            state.notices.push_back(Notice {
                id: 1,
                message: "fixture".into(),
                created_at: 100,
                tool: tool.into(),
            });
        }
        state.tracker.observe(&[item(100)], 100);
        state.set_scope(&crate::ai::agent_tools::MonitoringScope::from_cards(&[]));
        assert_eq!(state.generation, 2);
        assert_eq!(state.summary.today_tokens, 0);
        assert_eq!(state.notices.len(), 1);
        assert_eq!(state.notices[0].tool, "claude");
        assert!(
            state.tracker.observe(&[item(120)], 125).is_empty(),
            "new selection establishes a quiet baseline"
        );
    }
    #[test]
    fn first_observation_is_quiet_and_completions_are_not_repeated() {
        let mut tracker = CompletionTracker::default();
        assert!(tracker.observe(&[item(100)], 100).is_empty());
        assert_eq!(tracker.observe(&[item(120)], 125).len(), 1);
        assert!(tracker.observe(&[item(120)], 130).is_empty());
        assert!(tracker.observe(&[item(200)], 500).is_empty());
        assert!(tracker.observe(&[], 510).is_empty());
        assert!(tracker.observe(&[item(200)], 520).is_empty());
        let mut tracker = CompletionTracker::default();
        assert!(tracker.observe(&[item(520)], 525).is_empty());
    }
    #[test]
    fn deepseek_completion_keeps_harness_name() {
        let mut tracker = CompletionTracker::default();
        let mut before = item(100);
        before.key = "deepseek".into();
        before.recent_sessions[0].tool = "deepseek".into();
        assert!(tracker.observe(&[before.clone()], 100).is_empty());
        before.recent_sessions[0].completed_at = Some(120);
        let notices = tracker.observe(&[before], 125);
        assert_eq!(notices[0].message, "DeepSeek Harness · demo 已完成一轮");
        assert_eq!(notices[0].tool, "deepseek");
    }
    #[test]
    fn all_nine_tools_receive_their_own_completion_notice_without_token_requirement() {
        let mut tracker = CompletionTracker::default();
        let mut items: Vec<_> = crate::ai::agent_tools::order()
            .into_iter()
            .map(|key| {
                let mut value = item(100);
                value.key = key.into();
                value.usage_available = false;
                value.recent_sessions[0].tool = "claude".into();
                value
            })
            .collect();
        assert!(tracker.observe(&items, 100).is_empty());
        for item in &mut items {
            item.recent_sessions[0].completed_at = Some(120);
        }
        let notices = tracker.observe(&items, 125);
        assert_eq!(notices.len(), 9);
        for key in crate::ai::agent_tools::order() {
            assert!(notices.iter().any(|notice| {
                notice
                    .message
                    .starts_with(crate::ai::agent_tools::label(key))
            }));
        }
        assert!(tracker.observe(&items, 130).is_empty());
    }
}
