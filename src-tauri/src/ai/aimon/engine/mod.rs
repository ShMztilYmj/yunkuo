//! Token Monitor pinned tokscale bridge. Only normalized counts/metadata cross this boundary.
//! A report replaces one tool's ledger; it is never added to the native ledger.
use super::*;
use chrono::Datelike;
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::{Duration, Instant},
};

use crate::ai::agent_tools::MonitoringScope;

mod parse;
mod process;
use parse::*;
use process::*;
pub(crate) use process::{init, verify_bundle};
#[derive(Clone, Default)]
struct ResultSet {
    items: Vec<AiUsageItem>,
    quota: HashMap<String, Vec<AiQuotaMetric>>,
    day: String,
    zone: Option<chrono_tz::Tz>,
    at: i64,
}
#[derive(Default)]
struct Cache {
    result: Option<ResultSet>,
    scope: Option<MonitoringScope>,
    generation: u64,
    cancel: Option<tokio::sync::watch::Sender<bool>>,
    running: bool,
    attempted: Option<Instant>,
    remote_at: Option<Instant>,
    remote_requested: bool,
    failed: bool,
    issue: Option<String>,
}
static CACHE: once_cell::sync::Lazy<parking_lot::Mutex<Cache>> =
    once_cell::sync::Lazy::new(Default::default);

/// 自动扫描间隔。一次完整采集（report + graph，命中配额时再加 usage）在本机实测约 87 秒、
/// 消耗约 5 个 CPU 分钟；60 秒的节流会让 tokscale.exe 实际上一直在跑。轮询本身只读缓存，
/// 显式刷新（ai_usage_engine_refresh）仍会清空计时节流立即扫描。
const SCAN_INTERVAL: Duration = Duration::from_secs(300);
const QUOTA_PROVIDERS: [(&str, &str); 15] = [
    ("claude", "claude"),
    ("codex", "chatgpt"),
    ("cursor", "cursor"),
    ("antigravity", "antigravity"),
    ("grok", "grok"),
    ("kimi", "kimi"),
    ("opencode-go", "opencode"),
    ("zai", ""),
    ("amp", ""),
    ("copilot", ""),
    ("grok-bot", ""),
    ("minimax", ""),
    ("minimax-token-plan", ""),
    ("warp", ""),
    ("sakana", ""),
];
fn disabled_quota_providers(scope: &MonitoringScope) -> Vec<&'static str> {
    QUOTA_PROVIDERS
        .iter()
        .filter(|(_, tool)| !scope.contains(tool))
        .map(|(provider, _)| *provider)
        .collect()
}
struct QuotaProfile(PathBuf);
impl Drop for QuotaProfile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0.join("settings.json"));
        let _ = std::fs::remove_dir(&self.0);
    }
}
fn quota_profile(scope: &MonitoringScope) -> Result<QuotaProfile, String> {
    // Isolate the provider allowlist without changing the user's tokscale settings or copying credentials.
    // The pinned CLI checks disabledProviders before probing credentials or fetching any provider.
    let root = PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("无法定位采集设置目录")?)
        .join("YunKuo/agent-monitor/quota")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&root).map_err(|_| "无法创建采集设置目录")?;
    let profile = QuotaProfile(root);
    crate::config::atomic_write(
        &profile.0.join("settings.json"),
        &serde_json::to_vec(&serde_json::json!({
            "usage": { "disabledProviders": disabled_quota_providers(scope) }
        }))
        .map_err(|_| "无法生成采集设置")?,
    )
    .map_err(|_| "无法保存采集设置")?;
    Ok(profile)
}
async fn collect(bin: PathBuf, remote: bool, job: &Job) -> Result<ResultSet, String> {
    let base = home();
    let zone = bucket_timezone(&base);
    let today = bucket_day(chrono::Utc::now(), zone);
    // Explicit sync has its own budget. A failed remote sync must not discard readable local history.
    let mut sync_failed = HashSet::new();
    if remote
        && job.scope.contains("cursor")
        && (std::env::var_os("APPDATA").is_some_and(|p| {
            PathBuf::from(p)
                .join("Cursor/User/globalStorage/state.vscdb")
                .is_file()
        }) || base
            .join(".config/tokscale/cursor-credentials.json")
            .is_file())
    {
        if !run(
            &bin,
            &args(&["cursor", "sync", "--json"]),
            &base,
            150,
            job,
            None,
        )
        .await
        .is_ok_and(|v| v["synced"].as_bool() == Some(true))
        {
            sync_failed.insert("cursor");
        }
    }
    if remote
        && job.scope.contains("antigravity")
        && ["antigravity", "antigravity-ide", "antigravity-backup"]
            .iter()
            .any(|name| base.join(".gemini").join(name).is_dir())
    {
        // This subcommand returns human-readable output; only successful completion is relevant.
        if run(&bin, &args(&["antigravity", "sync"]), &base, 30, job, None)
            .await
            .is_err()
        {
            sync_failed.insert("antigravity");
        }
    }
    // A single all-time session report supplies the ledger and the last billed timestamp per session.
    // The daily graph supplies all dated Token buckets. Since all windows end today, last billed time
    // is sufficient to count distinct active main sessions for every cutoff without four more scans.
    let selected_clients = clients(&job.scope);
    let mut command = args(&[
        "--json",
        "--client",
        &selected_clients,
        "--group-by",
        "client,workspace,session,model",
        "--no-spinner",
        "--until",
    ]);
    command.push(today.to_string());
    let value = run(&bin, &command, &base, 120, job, None).await?;
    let mut incomplete = HashSet::new();
    for warning in value["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if warning.contains("Cursor usage requires") {
            incomplete.insert("cursor");
        } else {
            incomplete.extend(crate::ai::agent_tools::order());
        }
    }
    if value["undatedTokens"]
        .as_object()
        .is_some_and(|obj| obj.values().any(|v| v.as_u64().unwrap_or(0) > 0))
    {
        incomplete.extend(crate::ai::agent_tools::order());
    }
    let ledger = report(&value)?;
    let mut command = args(&[
        "graph",
        "--client",
        &selected_clients,
        "--no-spinner",
        "--until",
    ]);
    command.push(today.to_string());
    let daily = graph(&run(&bin, &command, &base, 120, job, None).await?)?;
    let monday = today - chrono::Duration::days(today.weekday().num_days_from_monday() as i64);
    let now = chrono::Utc::now().timestamp();
    let mut items = Vec::new();
    for tool in crate::ai::agent_tools::order()
        .into_iter()
        .filter(|tool| job.scope.contains(tool))
    {
        let Some((all, sessions)) = ledger.get(tool) else {
            continue;
        };
        let days = daily.get(tool).cloned().unwrap_or_default();
        let period = |start| period_usage(&days, sessions, start, today, zone);
        let last = sessions
            .iter()
            .map(|s| s.last_activity_at)
            .max()
            .filter(|ts| *ts > 0);
        let cloud = tool == "cursor";
        items.push(AiUsageItem {
            key: tool.into(),
            complete: !sync_failed.contains(tool) && !incomplete.contains(tool),
            usage_available: true,
            session_available: !sessions.is_empty(),
            input_includes_cached: Some(false),
            data_source: if cloud {
                "Cursor 账号云端用量 · 可包含其他设备".into()
            } else {
                "统一用量引擎 · 本地会话记录".into()
            },
            all: all.clone(),
            today: period(today),
            week: period(monday),
            last7: period(today - chrono::Duration::days(6)),
            last30: period(today - chrono::Duration::days(29)),
            daily: days,
            recent_sessions: sessions.clone(),
            scanned_at: now,
            last_activity_ts: last,
            note: if sync_failed.contains(tool) {
                if tool == "cursor" {
                    "云端同步未成功，显示最近缓存；请检查官方应用登录及网络"
                } else {
                    "本机服务同步未成功，显示可读取的本地记录；可在 Antigravity 启动后重试"
                }
                .into()
            } else if incomplete.contains(tool) {
                "部分上游记录缺少日期或存在解析告警，仅显示可核验部分".into()
            } else {
                format!(
                    "按来源工具归属，缓存与推理 Token 已统一计数；{}",
                    zone.map(|zone| format!("统计时区：{}", zone.name()))
                        .unwrap_or_else(|| "统计时区跟随系统".into())
                )
            },
            ..Default::default()
        });
    }
    // Publish counts before account endpoints finish, so a slow quota provider cannot hold the dashboard hostage.
    let previous_quota = {
        let mut cache = CACHE.lock();
        if !job.current(&cache) {
            return Err("Agent 名单已更新".into());
        }
        let quota = cache
            .result
            .as_ref()
            .map(|r| r.quota.clone())
            .unwrap_or_default();
        cache.result = Some(ResultSet {
            items: items.clone(),
            quota: quota.clone(),
            day: today.to_string(),
            zone,
            at: now,
        });
        quota
    };
    let quota = if remote {
        let current = if let Ok(profile) = quota_profile(&job.scope) {
            run(
                &bin,
                &args(&["usage", "--json"]),
                &base,
                90,
                job,
                Some(&profile.0),
            )
            .await
            .map(|v| quotas(&v, chrono::Utc::now().timestamp()))
            .unwrap_or_default()
        } else {
            HashMap::new()
        };
        // The CLI may return [] with per-provider fetch errors on stderr. Preserve old timestamps;
        // an absent response cannot turn yesterday's reading into a new successful sample.
        let mut merged = previous_quota;
        merged.extend(
            current
                .into_iter()
                .filter(|(key, _)| job.scope.contains(key)),
        );
        merged
    } else {
        previous_quota
    };
    Ok(ResultSet {
        items,
        quota,
        day: today.to_string(),
        zone,
        at: now,
    })
}
pub(super) struct Snapshot {
    result: Option<ResultSet>,
    usable: bool,
    issue: Option<String>,
}
impl Snapshot {
    fn new(result: Option<ResultSet>, failed: bool, issue: Option<String>) -> Self {
        let usable = !failed
            && result.as_ref().is_some_and(|r| {
                r.day == bucket_day(chrono::Utc::now(), r.zone).to_string()
                    && chrono::Utc::now().timestamp() - r.at <= 600
            });
        Self {
            result,
            usable,
            issue,
        }
    }
    pub(super) fn native_plan(&self, scope: MonitoringScope) -> NativePlan {
        if !self.usable {
            return NativePlan(Default::default(), scope);
        }
        NativePlan(
            self.result
                .as_ref()
                .unwrap()
                .items
                .iter()
                .filter(|item| item.usage_available)
                .map(|item| item.key.clone())
                .collect(),
            scope,
        )
    }
    pub(super) fn merge(self, items: &mut [AiUsageItem]) {
        if let Some(result) = &self.result {
            if self.usable {
                apply(items, result);
            } else {
                // Native fallback is already current; a failed or stale engine must not overwrite it.
                for item in items.iter_mut() {
                    if let Some(quota) = result.quota.get(&item.key) {
                        item.rate.metrics = quota.clone();
                    }
                    if item.usage_available {
                        item.note = format!("{}；统一引擎暂不可用，使用本地回退", item.note);
                    } else if matches!(item.key.as_str(), "cursor" | "antigravity") {
                        item.note = self.issue.clone().unwrap_or_else(|| {
                            "统一用量快照已过期，等待同步；工作状态仍独立显示".into()
                        });
                    }
                }
            }
        } else {
            for item in items
                .iter_mut()
                .filter(|item| matches!(item.key.as_str(), "cursor" | "antigravity"))
            {
                item.note = self.issue.clone().unwrap_or_else(|| {
                    "统一引擎正在读取；Cursor 需官方应用已登录，Antigravity 需有本地记录".into()
                });
            }
        }
    }
}
pub(super) fn snapshot(scope: MonitoringScope) -> Snapshot {
    schedule(scope.clone());
    let cache = CACHE.lock();
    if cache.scope.as_ref() != Some(&scope) {
        return Snapshot::new(None, false, None);
    }
    Snapshot::new(cache.result.clone(), cache.failed, cache.issue.clone())
}
pub(crate) fn invalidate_scope(scope: MonitoringScope) {
    CACHE.lock().set_scope(scope);
}
impl Cache {
    fn set_scope(&mut self, scope: MonitoringScope) {
        if self.scope.as_ref() == Some(&scope) {
            return;
        }
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(true);
        }
        self.generation = self.generation.wrapping_add(1);
        self.scope = Some(scope.clone());
        self.running = false;
        self.attempted = None;
        self.remote_at = None;
        self.failed = false;
        self.issue = None;
        // Keep unaffected tools' counts, but never allow a removed tool back via an old cache.
        if let Some(result) = &mut self.result {
            result.items.retain(|item| scope.contains(&item.key));
            result.quota.retain(|key, _| scope.contains(key));
        }
    }
}

fn schedule(scope: MonitoringScope) {
    let mut cache = CACHE.lock();
    if cache.scope.as_ref() != Some(&scope)
        || cache.running
        || cache
            .attempted
            .is_some_and(|at| at.elapsed() < SCAN_INTERVAL)
    {
        return;
    }
    let remote = cache.remote_requested
        || cache
            .remote_at
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(300));
    cache.remote_requested = false;
    if remote {
        cache.remote_at = Some(Instant::now());
    }
    let (cancel, receiver) = tokio::sync::watch::channel(false);
    let job = Job {
        scope,
        generation: cache.generation,
        cancel: receiver,
    };
    cache.cancel = Some(cancel);
    cache.running = true;
    cache.attempted = Some(Instant::now());
    drop(cache);
    tokio::spawn(async move {
        let result = match tokio::task::spawn_blocking(verified_binary).await {
            Ok(Ok(bin)) => collect(bin, remote, &job).await,
            Ok(Err(message)) => Err(message),
            _ => Err("用量引擎不可用".into()),
        };
        let mut cache = CACHE.lock();
        if !job.current(&cache) {
            return;
        }
        cache.running = false;
        cache.cancel = None;
        cache.failed = result.is_err();
        cache.issue = result.as_ref().err().cloned();
        if let Ok(result) = result {
            cache.result = Some(result);
        }
    });
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    available: bool,
    running: bool,
    last_success_at: Option<i64>,
    issue: Option<String>,
}
#[tauri::command]
pub fn ai_usage_engine_status() -> EngineStatus {
    let cache = CACHE.lock();
    EngineStatus {
        available: BINARY.get().is_some(),
        running: cache.running,
        last_success_at: cache.result.as_ref().map(|r| r.at),
        issue: cache.issue.clone(),
    }
}
#[tauri::command]
pub async fn ai_usage_engine_refresh(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<EngineStatus, String> {
    if window.label() != "main" {
        return Err("请在主窗口同步用量".into());
    }
    {
        let mut cache = CACHE.lock();
        cache.attempted = None;
        cache.remote_requested = true;
    }
    schedule(MonitoringScope::current(&app));
    Ok(ai_usage_engine_status())
}

#[cfg(test)]
mod tests {
    #[cfg(test)]
    use super::activity::AiSession;
    use super::*;
    use serde_json::json;
    #[test]
    fn automatic_scans_stay_well_behind_a_full_collection() {
        // 本机实测：report 约 40 秒、graph 约 47 秒，且解析缓存不可用时每次都是冷扫描。
        // 间隔若短于一次采集，schedule() 一释放就会立刻再起一轮，CPU 会长时间居高不下。
        assert!(SCAN_INTERVAL >= Duration::from_secs(180));
    }
    #[test]
    fn added_scope_filters_clients_native_scans_quota_probes_and_stale_jobs() {
        let default = MonitoringScope::from_cards(&[]);
        assert_eq!(clients(&default), "claude,codex");
        let added = MonitoringScope::from_cards(&["deepseek".into(), "antigravity".into()]);
        assert_eq!(
            clients(&added),
            "claude,codex,dsh,antigravity,antigravity-cli,antigravity-extension"
        );
        let plan = Snapshot::new(None, false, None).native_plan(default.clone());
        assert!(plan.enabled("claude"));
        assert!(!plan.enabled("deepseek"));
        assert!(!plan.enabled("cursor"));
        let disabled = disabled_quota_providers(&default);
        assert!(!disabled.contains(&"claude"));
        assert!(!disabled.contains(&"codex"));
        for provider in [
            "cursor",
            "antigravity",
            "grok",
            "kimi",
            "opencode-go",
            "zai",
            "grok-bot",
        ] {
            assert!(disabled.contains(&provider));
        }
        assert!(!disabled_quota_providers(&added).contains(&"antigravity"));
        let (sender, receiver) = tokio::sync::watch::channel(false);
        let old = Job {
            scope: added.clone(),
            generation: 1,
            cancel: receiver,
        };
        let cache = Cache {
            scope: Some(default),
            generation: 2,
            ..Default::default()
        };
        assert!(!old.current(&cache));
        let readded = Cache {
            scope: Some(added),
            generation: 3,
            ..Default::default()
        };
        assert!(
            !old.current(&readded),
            "remove and re-add must still reject the old job"
        );
        let mut cache = Cache {
            scope: Some(old.scope.clone()),
            generation: 1,
            cancel: Some(sender),
            running: true,
            result: Some(ResultSet {
                items: vec![
                    AiUsageItem {
                        key: "deepseek".into(),
                        ..Default::default()
                    },
                    AiUsageItem {
                        key: "claude".into(),
                        ..Default::default()
                    },
                ],
                quota: HashMap::from([("deepseek".into(), vec![])]),
                ..Default::default()
            }),
            ..Default::default()
        };
        cache.set_scope(MonitoringScope::from_cards(&[]));
        assert!(*old.cancel.borrow());
        assert!(!cache.running);
        assert!(!old.current(&cache));
        assert_eq!(cache.result.as_ref().unwrap().items.len(), 1);
        assert!(cache.result.as_ref().unwrap().quota.is_empty());
        cache.set_scope(old.scope.clone());
        assert!(!old.current(&cache));
    }
    #[test]
    fn snapshot_selects_one_usage_owner_and_falls_back_on_failure_or_expiry() {
        let now = chrono::Utc::now().timestamp();
        let result = ResultSet {
            items: vec![AiUsageItem {
                key: "claude".into(),
                usage_available: true,
                complete: true,
                all: AiUsage {
                    total: 999,
                    ..Default::default()
                },
                ..Default::default()
            }],
            day: chrono::Local::now().date_naive().to_string(),
            at: now,
            ..Default::default()
        };
        let good = Snapshot::new(Some(result.clone()), false, None);
        assert_eq!(
            good.native_plan(MonitoringScope::from_cards(&[]))
                .mode("claude"),
            ScanMode::Activity
        );
        assert_eq!(
            good.native_plan(MonitoringScope::from_cards(&[]))
                .mode("chatgpt"),
            ScanMode::Usage
        );
        for (failed, at) in [(true, now), (false, now - 601)] {
            let snapshot = Snapshot::new(
                Some(ResultSet {
                    at,
                    ..result.clone()
                }),
                failed,
                None,
            );
            assert_eq!(
                snapshot
                    .native_plan(MonitoringScope::from_cards(&[]))
                    .mode("claude"),
                ScanMode::Usage
            );
            let mut fallback = vec![AiUsageItem {
                key: "claude".into(),
                usage_available: true,
                all: AiUsage {
                    total: 123,
                    ..Default::default()
                },
                ..Default::default()
            }];
            snapshot.merge(&mut fallback);
            assert_eq!(fallback[0].all.total, 123);
        }
    }
    #[test]
    fn one_daily_ledger_supplies_all_windows_without_recounting_sessions_or_cache() {
        use chrono::TimeZone;
        let today = chrono::Local::now().date_naive();
        let days: Vec<_> = [0, 1, 6, 7, 29, 30]
            .into_iter()
            .map(|offset| AiDailyUsage {
                date: (today - chrono::Duration::days(offset)).to_string(),
                usage: AiUsage {
                    input: 100,
                    output: 20,
                    cached: 50,
                    cache_write: 10,
                    total: 180,
                    ..Default::default()
                },
            })
            .collect();
        let sessions: Vec<_> = [0, 7, 30]
            .into_iter()
            .map(|offset| AiSession {
                id: format!("claude:{offset}"),
                tool: "claude".into(),
                project: "fixture".into(),
                model: None,
                state: "unknown".into(),
                last_activity_at: chrono::Local
                    .from_local_datetime(
                        &(today - chrono::Duration::days(offset))
                            .and_hms_opt(12, 0, 0)
                            .unwrap(),
                    )
                    .earliest()
                    .unwrap()
                    .timestamp(),
                completed_at: None,
                total: 180,
                usage_available: Some(true),
            })
            .collect();
        for (offset, total, count) in [(0, 180, 1), (6, 540, 1), (29, 900, 2)] {
            let usage = period_usage(
                &days,
                &sessions,
                today - chrono::Duration::days(offset),
                today,
                None,
            );
            assert_eq!(usage.total, total);
            assert_eq!(usage.sessions, count);
            assert_eq!(
                usage.input + usage.output + usage.cached + usage.cache_write,
                total
            );
        }
    }
    #[test]
    fn all_nine_keep_tool_ownership_and_unified_cache_semantics() {
        let clients = [
            "claude",
            "codex",
            "cursor",
            "antigravity",
            "opencode",
            "kimi",
            "grok",
            "zcode",
            "dsh",
        ];
        let entries:Vec<_>=clients.iter().map(|client|json!({"client":client,"sessionId":"same","model":"claude-fixture","input":100,"output":20,"cacheRead":50,"cacheWrite":10,"reasoning":0})).collect();
        let report = report(&json!({"entries":entries})).unwrap();
        assert_eq!(report.len(), 9);
        for client in clients {
            let tool = key(client).unwrap();
            let (usage, sessions) = &report[tool];
            assert_eq!(usage.total, 180);
            assert_eq!(sessions[0].id, format!("{tool}:same"));
            assert_eq!(sessions[0].state, "unknown");
        }
    }
    #[test]
    fn aliases_fold_without_recounting_parent_metadata_or_reasoning() {
        let row = |client, sid| json!({"client":client,"sessionId":sid,"input":100,"output":20,"cacheRead":50,"cacheWrite":10,"reasoning":5});
        let data=report(&json!({"entries":[row("antigravity","main"),row("antigravity-cli","child")],"sessions":[{"client":"antigravity-cli","sessionId":"child","parentSessionId":"main"}]})).unwrap();
        assert_eq!(data["antigravity"].0.total, 370);
        assert_eq!(data["antigravity"].0.sessions, 1);
        assert!(report(&json!({"entries":[{"client":"cursor","input":null}]})).is_err());
    }
    #[test]
    fn replaces_counts_never_sums_native_and_never_invents_status() {
        let session = AiSession {
            id: "cursor:s".into(),
            tool: "cursor".into(),
            project: "demo".into(),
            model: None,
            state: "asking".into(),
            last_activity_at: 100,
            completed_at: None,
            total: 10,
            usage_available: Some(false),
        };
        let mut native = vec![AiUsageItem {
            key: "cursor".into(),
            all: AiUsage {
                total: 10,
                ..Default::default()
            },
            recent_sessions: vec![session.clone()],
            ..Default::default()
        }];
        let mut snapshot = session;
        snapshot.state = "unknown".into();
        snapshot.total = 180;
        let result = ResultSet {
            items: vec![AiUsageItem {
                key: "cursor".into(),
                complete: true,
                usage_available: true,
                all: AiUsage {
                    total: 180,
                    ..Default::default()
                },
                recent_sessions: vec![snapshot],
                ..Default::default()
            }],
            day: chrono::Local::now().date_naive().to_string(),
            at: chrono::Utc::now().timestamp(),
            ..Default::default()
        };
        apply(&mut native, &result);
        apply(&mut native, &result);
        assert_eq!(native[0].all.total, 180);
        assert_eq!(native[0].recent_sessions[0].state, "asking");
        assert_eq!(native[0].recent_sessions[0].total, 180);
        assert!(native[0].complete);
    }
    #[test]
    fn quota_accounts_and_model_groups_remain_separate() {
        let data = quotas(
            &json!([{"provider":"Antigravity","account":{"label":"model-group"},"metrics":[{"label":"Gemini","used_percent":25,"remaining_percent":75}]},{"provider":"Grok Bot","metrics":[{"label":"Weekly","used_percent":40}]}]),
            100,
        );
        assert_eq!(data["antigravity"][0].remaining_percent, Some(75.));
        assert_eq!(data["antigravity"][0].account_label, "model-group");
        assert!(!data.contains_key("grok"));
    }
    #[test]
    fn session_cutoffs_follow_the_same_pinned_timezone_as_daily_buckets() {
        let time = chrono::DateTime::parse_from_rfc3339("2026-10-21T02:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let china: chrono_tz::Tz = "Asia/Shanghai".parse().unwrap();
        let ny: chrono_tz::Tz = "America/New_York".parse().unwrap();
        assert_eq!(bucket_day(time, Some(china)).to_string(), "2026-10-21");
        assert_eq!(bucket_day(time, Some(ny)).to_string(), "2026-10-20");
        let session = AiSession {
            id: "same".into(),
            tool: "claude".into(),
            project: "fixture".into(),
            model: None,
            state: "unknown".into(),
            last_activity_at: time.timestamp(),
            completed_at: None,
            total: 180,
            usage_available: Some(true),
        };
        let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 21).unwrap();
        assert_eq!(
            period_usage(&[], &[session.clone()], date, date, Some(china)).sessions,
            1
        );
        assert_eq!(
            period_usage(&[], &[session], date, date, Some(ny)).sessions,
            0
        );
    }
    #[test]
    fn only_reclaims_the_reaped_childs_own_sync_lock() {
        let fixture = crate::test_support::FixtureDir::new();
        let cache = fixture.path();
        let now = chrono::Utc::now().timestamp();
        std::fs::write(cache.join("sync.os.lock"), "").unwrap();
        std::fs::write(cache.join("sync.lock"), format!("42 {now}")).unwrap();
        cleanup_owned_sync_lock(cache, 43, now);
        assert!(cache.join("sync.lock").exists());
        cleanup_owned_sync_lock(cache, 42, now + 10);
        assert!(cache.join("sync.lock").exists());
        cleanup_owned_sync_lock(cache, 42, now);
        assert!(!cache.join("sync.lock").exists());
    }
    #[tokio::test]
    #[ignore = "只读真实记录，输出计数；不输出正文或认证数据"]
    async fn real_engine_count_only() {
        init(&PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        let result = collect(
            verified_binary().unwrap(),
            true,
            &Job {
                scope: MonitoringScope::from_cards(
                    &crate::ai::agent_tools::order()
                        .into_iter()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>(),
                ),
                generation: 0,
                cancel: tokio::sync::watch::channel(false).1,
            },
        )
        .await
        .unwrap();
        for item in result.items {
            println!(
                "{} total={} sessions={}",
                item.key, item.all.total, item.all.sessions
            );
        }
    }
}
