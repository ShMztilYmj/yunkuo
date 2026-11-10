//! Selected local CLI transcripts only. Never writes to Claude's transcripts or credentials.
use crate::workspace::resume::model::{Candidate, Quota, digest, label, valid_id};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

pub fn home() -> Result<PathBuf> {
    if std::env::var_os("CLAUDE_CONFIG_DIR").is_some_and(|s| !s.is_empty()) {
        bail!("首版仅支持默认 Claude 配置目录；自定义目录暂不自动续跑");
    }
    PathBuf::from(std::env::var_os("USERPROFILE").context("无法定位 Claude 数据目录")?)
        .join(".claude")
        .canonicalize()
        .context("请先安装 Claude Code 并登录")
}
pub fn key(home: &Path) -> String {
    digest(home.to_string_lossy().to_lowercase())
}
fn bytes(path: &Path, limit: u64) -> Result<Vec<u8>> {
    crate::platform::secure_storage::safe(path)?;
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() || meta.len() > limit {
        bail!("本地配置超出读取范围");
    }
    crate::platform::files::read_bounded(path, limit as usize, "本地文件过大")
}
fn settings(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(serde_json::json!({}));
    }
    Ok(serde_json::from_slice(&bytes(path, 1024 * 1024)?)?)
}
const OTHER_AUTH: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_ANTHROPIC_AWS",
    "CLAUDE_CODE_USE_ANTHROPIC_GOOGLE_CLOUD",
    "CLAUDE_CODE_USE_MANTLE",
];
pub fn subscription_only(home: &Path, cwd: Option<&Path>) -> Result<()> {
    let mut configs = vec![settings(&home.join("settings.json"))?];
    if let Some(cwd) = cwd {
        configs.push(settings(&cwd.join(".claude/settings.json"))?);
        configs.push(settings(&cwd.join(".claude/settings.local.json"))?);
    }
    for k in OTHER_AUTH {
        if std::env::var_os(k).is_some_and(|v| !v.is_empty())
            || configs
                .iter()
                .any(|v| v["env"][k].as_str().is_some_and(|v| !v.is_empty()))
        {
            bail!("此配置使用 API Key 或第三方提供商，暂不适用订阅额度恢复");
        }
    }
    if configs
        .iter()
        .any(|v| v["apiKeyHelper"].is_string() || v["env"]["CLAUDE_CODE_OAUTH_TOKEN"].is_string())
    {
        bail!("设置覆盖了认证来源，暂不自动恢复");
    }
    Ok(())
}
pub struct Login {
    pub key: String,
    pub token: Zeroizing<String>,
}
pub fn login(home: &Path) -> Result<Login> {
    subscription_only(home, None)?;
    let token =
        if let Some(v) = std::env::var_os("CLAUDE_CODE_OAUTH_TOKEN").filter(|v| !v.is_empty()) {
            v.into_string()
                .map_err(|_| anyhow::anyhow!("登录令牌格式不受支持"))?
        } else {
            let raw = Zeroizing::new(
                bytes(&home.join(".credentials.json"), 1024 * 1024)
                    .context("请先在 Claude Code 中完成订阅登录")?,
            );
            let v: Value = serde_json::from_slice(&raw)?;
            let oauth = &v["claudeAiOauth"];
            if oauth["expiresAt"]
                .as_i64()
                .is_some_and(|n| n <= chrono::Utc::now().timestamp_millis())
            {
                bail!("Claude 登录已过期，请先在 Claude Code 中刷新登录");
            }
            oauth["accessToken"]
                .as_str()
                .filter(|s| !s.is_empty())
                .context("订阅登录信息不可用")?
                .to_owned()
        };
    Ok(Login {
        key: digest(token.as_bytes()),
        token: Zeroizing::new(token),
    })
}
#[derive(Clone)]
pub struct Frame {
    pub id: String,
    pub title: String,
    pub project: String,
    pub cwd: PathBuf,
    pub configuration: String,
    pub model: String,
    pub anchor: String,
    pub quota: bool,
    pub quota_proven: bool,
    pub event_at: i64,
    pub fresh: bool,
    pub last_user: String,
    pub marker_seen: bool,
    pub completed: bool,
}
impl Frame {
    pub fn recovered_round_completed(&self, marker: &str) -> bool {
        self.fresh && self.marker_seen && self.last_user.contains(marker) && self.completed
    }
}
fn files(home: &Path) -> Result<Vec<PathBuf>> {
    let root = home
        .join("projects")
        .canonicalize()
        .context("尚未找到 Claude Code 本地会话")?;
    let mut result = Vec::new();
    for dir in std::fs::read_dir(&root)?.take(2048) {
        let dir = dir?;
        if !dir.file_type()?.is_dir() {
            continue;
        }
        for f in std::fs::read_dir(dir.path())?.take(4096) {
            let f = f?;
            if f.file_type()?.is_file()
                && f.path().extension().is_some_and(|e| e == "jsonl")
                && f.path()
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(valid_id)
            {
                result.push(f.path());
            }
        }
    }
    Ok(result)
}
fn text(v: &Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        return Some(s.to_owned());
    }
    v.as_array().map(|a| {
        a.iter()
            .filter(|v| v["type"] == "text")
            .filter_map(|v| v["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    })
}
fn epoch(v: &Value) -> i64 {
    v.as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.timestamp())
        .unwrap_or(0)
}
pub fn frame(home: &Path, id: &str, marker: Option<&str>) -> Result<Frame> {
    if !valid_id(id) {
        bail!("请输入完整有效的 Claude 会话 UUID");
    }
    let matches: Vec<_> = files(home)?
        .into_iter()
        .filter(|p| p.file_stem().and_then(|s| s.to_str()) == Some(id))
        .collect();
    if matches.len() != 1 {
        bail!("无法唯一定位该 Claude 本地会话");
    }
    let f = read_frame(home, &matches[0], id, marker)?;
    subscription_only(home, Some(&f.cwd))?;
    Ok(f)
}
fn read_frame(home: &Path, path: &Path, id: &str, marker: Option<&str>) -> Result<Frame> {
    crate::platform::secure_storage::safe(path)?;
    let path = path.canonicalize()?;
    if !path.starts_with(home.join("projects").canonicalize()?) {
        bail!("会话文件不在 Claude 项目目录内");
    }
    let before = std::fs::metadata(&path)?;
    let size = before.len();
    let mut file = std::fs::File::open(&path)?;
    let mut head = Vec::new();
    (&mut file).take(256 * 1024).read_to_end(&mut head)?;
    let offset = size.saturating_sub(4 * 1024 * 1024);
    file.seek(SeekFrom::Start(offset))?;
    let mut tail = Vec::new();
    file.take(4 * 1024 * 1024 + 1).read_to_end(&mut tail)?;
    if offset > 0 {
        let start = tail
            .iter()
            .position(|b| *b == b'\n')
            .context("会话记录行过大")?
            + 1;
        tail.drain(..start);
    }
    let after = std::fs::metadata(&path)?;
    let mut fresh = tail.last() == Some(&b'\n')
        && after.len() == size
        && after.modified()? == before.modified()?;
    let mut cwd = None;
    let mut origin = None;
    let mut model = String::new();
    let mut f = Frame {
        id: id.into(),
        title: format!("本地会话 {}", &id[..8]),
        project: String::new(),
        cwd: PathBuf::new(),
        configuration: String::new(),
        model: String::new(),
        anchor: String::new(),
        quota: false,
        quota_proven: false,
        event_at: 0,
        fresh,
        last_user: String::new(),
        marker_seen: false,
        completed: false,
    };
    for line in head.split(|b| *b == b'\n') {
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        if v["sessionId"] != id || v["isSidechain"] == true {
            continue;
        }
        if cwd.is_none() {
            cwd = v["cwd"].as_str().map(PathBuf::from);
        }
        if origin.is_none() {
            origin = v["entrypoint"].as_str().map(str::to_owned);
        }
    }
    if !matches!(origin.as_deref(), Some("cli" | "sdk-cli" | "claude-code")) {
        bail!("首版仅接管本地 CLI 会话，桌面 Code、Chat 和 Cowork 暂不支持");
    }
    for line in tail.split(|b| *b == b'\n') {
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            fresh = false;
            continue;
        };
        if v["sessionId"] != id || v["isSidechain"] == true {
            continue;
        }
        if v["type"] == "custom-title" {
            if let Some(t) = v["customTitle"].as_str() {
                f.title = label(t, 100);
            }
        }
        if v["type"] == "user" && v["isMeta"] != true {
            if let Some(s) = text(&v["message"]["content"]) {
                if !s.is_empty() {
                    f.last_user = s;
                    f.quota = false;
                    f.completed = false;
                    f.anchor = v["uuid"].as_str().unwrap_or_default().into();
                    f.event_at = epoch(&v["timestamp"]);
                }
            }
        }
        if v["type"] == "assistant" {
            if let Some(m) = v["message"]["model"]
                .as_str()
                .filter(|m| *m != "<synthetic>")
            {
                model = m.into();
            }
            f.anchor = v["uuid"].as_str().unwrap_or_default().into();
            f.event_at = epoch(&v["timestamp"]);
            f.quota = v["isApiErrorMessage"] == true && v["error"] == "rate_limit";
            f.quota_proven = f.quota && v["apiError"] == "usage_limit_reached";
            f.completed = !f.quota
                && v["isApiErrorMessage"] != true
                && matches!(
                    v["message"]["stop_reason"].as_str(),
                    Some("end_turn" | "stop_sequence")
                );
        }
        if marker.is_some_and(|m| f.last_user.contains(m)) {
            f.marker_seen = true;
        }
    }
    let cwd = cwd
        .context("会话没有可核验的工作目录")?
        .canonicalize()
        .context("原项目目录已不存在")?;
    if !cwd.is_dir() {
        bail!("原工作目录不可用");
    }
    let mut cfg = Vec::new();
    for p in [
        home.join("settings.json"),
        cwd.join(".claude/settings.json"),
        cwd.join(".claude/settings.local.json"),
        cwd.join(".mcp.json"),
    ] {
        if p.exists() {
            cfg.extend(bytes(&p, 1024 * 1024)?);
        }
        cfg.push(0);
    }
    f.project = label(
        cwd.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .as_ref(),
        80,
    );
    f.configuration = digest(serde_json::to_vec(&(
        cwd.to_string_lossy(),
        path.to_string_lossy(),
        &model,
        digest(cfg),
    ))?);
    f.cwd = cwd;
    f.model = model;
    f.fresh = fresh;
    if f.anchor.is_empty() || f.event_at <= 0 {
        bail!("会话最新轮次暂不可核验");
    }
    Ok(f)
}
pub fn candidates(home: &Path) -> Result<Vec<Candidate>> {
    let mut paths = files(home)?;
    paths.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    Ok(paths
        .into_iter()
        .rev()
        .take(80)
        .filter_map(|p| {
            let id = p.file_stem()?.to_str()?;
            let f = read_frame(home, &p, id, None).ok()?;
            Some(Candidate {
                thread_id: f.id,
                title: f.title,
                project: f.project,
                quota_stopped: f.quota,
            })
        })
        .collect())
}
pub fn quota(v: &Value, time: i64) -> Quota {
    let unknown = Quota {
        available: None,
        reset_at: None,
    };
    let mut windows = Vec::new();
    if v.as_object().is_none_or(|m| {
        m.keys().any(|k| {
            (k.starts_with("seven_day") || k.starts_with("five_hour"))
                && !matches!(
                    k.as_str(),
                    "five_hour"
                        | "seven_day"
                        | "seven_day_opus"
                        | "seven_day_sonnet"
                        | "seven_day_cowork"
                        | "seven_day_oauth_apps"
                )
        })
    }) {
        return unknown;
    }
    for key in [
        "five_hour",
        "seven_day",
        "seven_day_opus",
        "seven_day_sonnet",
        "seven_day_cowork",
        "seven_day_oauth_apps",
    ] {
        let w = &v[key];
        if w.is_null() {
            continue;
        }
        let Some(used) = w["utilization"]
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 0.)
        else {
            return unknown;
        };
        windows.push((used, epoch(&w["resets_at"])));
    }
    if let Some(limits) = v.get("limits").filter(|v| !v.is_null()) {
        let Some(items) = limits.as_array() else {
            return unknown;
        };
        for w in items {
            let Some(used) = w["percent"].as_f64().filter(|n| n.is_finite() && *n >= 0.) else {
                return unknown;
            };
            windows.push((used, epoch(&w["resets_at"])));
        }
    }
    if windows.is_empty() {
        return unknown;
    }
    let blocked: Vec<_> = windows.into_iter().filter(|(u, _)| *u >= 100.).collect();
    if blocked.is_empty() {
        Quota {
            available: Some(true),
            reset_at: None,
        }
    } else {
        Quota {
            available: Some(false),
            reset_at: if blocked.iter().all(|(_, t)| *t > time) {
                blocked.iter().map(|(_, t)| *t).max()
            } else {
                None
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn live_windows_need_all_blocking_limits_to_reset() {
        let w = |p, t: &str| json!({"utilization":p,"resets_at":t});
        let now = 100;
        assert_eq!(quota(&json!({}), now).available, None);
        assert_eq!(quota(&json!({"five_hour":w(100,"1970-01-01T00:03:20Z"),"seven_day":w(100,"1970-01-01T00:08:20Z")}),now).reset_at,Some(500));
        assert_eq!(
            quota(&json!({"five_hour":w(0,"1970-01-01T00:08:20Z")}), now).available,
            Some(true)
        );
        assert_eq!(
            quota(&json!({"five_hour":w(100,"1970-01-01T00:00:10Z")}), now).available,
            Some(false)
        );
        assert_eq!(quota(&json!({"five_hour":w(0,"1970-01-01T00:08:20Z"),"limits":[{"percent":100,"resets_at":"1970-01-01T00:08:20Z"}]}),now).available,Some(false));
        assert_eq!(
            quota(&json!({"five_hour":{"utilization":"bad"}}), now).available,
            None
        );
        assert_eq!(quota(&json!({"five_hour":w(0,"1970-01-01T00:08:20Z"),"seven_day_future":{"utilization":100}}),now).available,None);
    }
    fn fixture(origin: &str) -> (crate::test_support::FixtureDir, PathBuf, PathBuf, String) {
        let d = crate::test_support::FixtureDir::new();
        let h = d.path().canonicalize().unwrap();
        let dir = h.join("projects/demo");
        std::fs::create_dir_all(&dir).unwrap();
        let cwd = h.join("workspace");
        std::fs::create_dir(&cwd).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let p = dir.join(format!("{id}.jsonl"));
        let user = json!({"type":"user","sessionId":id,"uuid":uuid::Uuid::new_v4(),"entrypoint":origin,"cwd":cwd,"timestamp":"2026-10-21T12:00:00Z","message":{"content":"fixture task"}});
        let fail = json!({"type":"assistant","sessionId":id,"uuid":uuid::Uuid::new_v4(),"entrypoint":origin,"cwd":cwd,"timestamp":"2026-10-21T12:01:00Z","error":"rate_limit","isApiErrorMessage":true,"apiError":"usage_limit_reached","message":{"model":"fixture-model"}});
        std::fs::write(&p, format!("{user}\n{fail}\n")).unwrap();
        (d, h, p, id)
    }
    #[test]
    fn exact_cli_transcript_identifies_failure_and_user_takeover() {
        use std::io::Write;
        let (_d, h, p, id) = fixture("cli");
        let f = read_frame(&h, &p, &id, None).unwrap();
        assert!(f.quota && f.quota_proven && f.fresh);
        assert_eq!(f.model, "fixture-model");
        let next = json!({"type":"user","uuid":uuid::Uuid::new_v4(),"sessionId":id,"timestamp":"2026-10-21T12:02:00Z","message":{"content":"用户停止"}});
        writeln!(
            std::fs::OpenOptions::new().append(true).open(&p).unwrap(),
            "{next}"
        )
        .unwrap();
        let current = read_frame(&h, &p, &id, None).unwrap();
        assert!(!current.quota);
        assert_ne!(f.anchor, current.anchor);
    }
    #[test]
    fn desktop_and_unfinished_line_are_never_ready() {
        use std::io::Write;
        let (_d, h, p, id) = fixture("claude-desktop");
        assert!(read_frame(&h, &p, &id, None).is_err());
        let (_d, h, p, id) = fixture("sdk-cli");
        std::fs::OpenOptions::new()
            .append(true)
            .open(&p)
            .unwrap()
            .write_all(b"{\"type\":")
            .unwrap();
        assert!(!read_frame(&h, &p, &id, None).unwrap().fresh);
    }
    #[test]
    fn malformed_new_events_cannot_leave_old_failure_actionable() {
        use std::io::Write;
        let (_d, h, p, id) = fixture("cli");
        writeln!(
            std::fs::OpenOptions::new().append(true).open(&p).unwrap(),
            "bad-json"
        )
        .unwrap();
        assert!(!read_frame(&h, &p, &id, None).unwrap().fresh);
    }
    #[test]
    fn restart_completion_must_belong_to_the_recovery_user_turn() {
        use std::io::Write;
        let (_d, h, p, id) = fixture("cli");
        let marker = "[fixture-resume]";
        let mut append = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        let user = |content: &str| json!({"type":"user","uuid":uuid::Uuid::new_v4(),"sessionId":id,"timestamp":"2026-10-21T12:02:00Z","message":{"content":content}});
        let complete = json!({"type":"assistant","uuid":uuid::Uuid::new_v4(),"sessionId":id,"timestamp":"2026-10-21T12:03:00Z","message":{"model":"fixture-model","stop_reason":"end_turn"}});
        writeln!(append, "{}\n{complete}", user(marker)).unwrap();
        assert!(
            read_frame(&h, &p, &id, Some(marker))
                .unwrap()
                .recovered_round_completed(marker)
        );
        writeln!(append, "{}\n{complete}", user("a later unrelated task")).unwrap();
        let latest = read_frame(&h, &p, &id, Some(marker)).unwrap();
        assert!(latest.marker_seen && latest.completed);
        assert!(!latest.recovered_round_completed(marker));
    }
    /// 与 codex 的 simulated_reset_dispatch_receipt_and_restart_never_touch_real_codex 对等：
    /// 用真实转录跑「发现限流 → 建守候 → 额度未重置不能认领 → 重置后认领一次 → 续跑回合完成 →
    /// 重启后不再复活」，全程不启动任何 CLI。
    #[test]
    fn simulated_reset_resume_and_restart_never_touch_a_real_claude() {
        use super::super::scheduler;
        use crate::workspace::resume::{model::Watch, now, store::Store};
        use std::io::Write;
        let (_d, h, p, id) = fixture("cli");
        let app_data = h.join("fake-yunkuo-data");
        let mut store = Store::open_for(&app_data, "claude").unwrap();
        let f = frame(&h, &id, None).unwrap();
        assert!(f.quota && f.fresh);
        let watch = Watch {
            thread_id: f.id.clone(),
            home_key: key(&h),
            title: f.title.clone(),
            project: f.project.clone(),
            account: "fixture-account".into(),
            configuration: f.configuration.clone(),
            enabled: true,
            consumed: false,
            created_at: f.event_at - 1,
        };
        store.put_watch(&watch).unwrap();
        store.enabled_set(true, now()).unwrap();
        scheduler::observe(&store, &watch, &f, true).unwrap();
        let record = store.records().unwrap().remove(0);
        assert_eq!(record.state, "waiting_quota");
        assert_eq!(record.turn_id, f.anchor);
        // observe 用真实时钟写入 next_check，因此认领时间也必须用 now()。
        let t = record.next_check;
        // 额度未重置前不能认领。
        let mut limited = record.clone();
        limited.reset_at = Some(t + 60);
        store.update(&limited).unwrap();
        assert!(store.claim(&record.id, t).unwrap().is_none());
        // 重置后只认领一次，随后同一失败再扫一遍也不会新增记录。
        let mut ready = record.clone();
        ready.reset_at = None;
        store.update(&ready).unwrap();
        let claimed = store.claim(&record.id, t + 61).unwrap().unwrap();
        assert_eq!(claimed.state, "submitting");
        scheduler::observe(&store, &watch, &f, true).unwrap();
        assert_eq!(store.records().unwrap().len(), 1);
        // 续跑的完成回合必须来自带 marker 的那一轮真实转录。
        let marker = claimed.marker.clone();
        let user = json!({"type":"user","uuid":uuid::Uuid::new_v4(),"sessionId":id,"timestamp":"2026-10-21T12:02:00Z","message":{"content":marker}});
        let complete = json!({"type":"assistant","uuid":uuid::Uuid::new_v4(),"sessionId":id,"timestamp":"2026-10-21T12:03:00Z","message":{"model":"fixture-model","stop_reason":"end_turn"}});
        writeln!(
            std::fs::OpenOptions::new().append(true).open(&p).unwrap(),
            "{user}\n{complete}"
        )
        .unwrap();
        assert!(
            frame(&h, &id, Some(&marker))
                .unwrap()
                .recovered_round_completed(&marker)
        );
        // 重启后守候已消费，崩溃恢复也不能让它复活。
        drop(store);
        let mut restarted = Store::open_for(&app_data, "claude").unwrap();
        restarted.recover_crash(t + 120).unwrap();
        assert!(restarted.claim(&record.id, t + 120).unwrap().is_none());
    }
}
