//! Codex databases are opened read-only. Queries are deliberately confined to this module.
use super::model::{Candidate, digest, label, valid_id};
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::Value;
use std::{
    collections::HashSet,
    io::{BufRead, Read},
    path::{Path, PathBuf},
};

pub fn home() -> Result<PathBuf> {
    let dir = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join(".codex")))
        .context("找不到 Codex 数据目录")?;
    if !dir.is_absolute() {
        bail!("Codex 数据目录必须是绝对路径");
    }
    dir.canonicalize().context("Codex 数据目录不存在")
}
pub fn home_key(home: &Path) -> String {
    digest(home.to_string_lossy().to_lowercase())
}
fn bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() || !meta.is_file() || meta.len() > limit {
        bail!("文件类型或大小不受支持");
    }
    crate::platform::files::read_bounded(path, limit as usize, "文件过大")
}
pub fn database(home: &Path, kind: &str) -> Result<PathBuf> {
    let mut found = None;
    for file in std::fs::read_dir(home)? {
        let file = file?;
        let name = file.file_name();
        let name = name.to_string_lossy();
        let Some(n) = name
            .strip_prefix(&format!("{kind}_"))
            .and_then(|s| s.strip_suffix(".sqlite"))
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        if !file.file_type()?.is_file() {
            continue;
        }
        if found.as_ref().is_none_or(|(old, _)| n > *old) {
            found = Some((n, file.path()));
        }
    }
    let path = found.context("Codex 数据库尚不可用")?.1;
    if path.canonicalize()?.parent() != Some(home) {
        bail!("Codex 数据库路径不受支持");
    }
    Ok(path)
}
fn db(home: &Path, kind: &str) -> Result<Connection> {
    let c = Connection::open_with_flags(
        database(home, kind)?,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    c.busy_timeout(std::time::Duration::from_millis(800))?;
    c.execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;")?;
    let tables: &[(&str, &[&str])] = match kind {
        "state" => &[(
            "threads",
            &[
                "id",
                "name",
                "cwd",
                "source",
                "thread_source",
                "history_mode",
                "rollout_path",
                "archived",
                "model_provider",
                "model",
                "sandbox_policy",
                "approval_mode",
                "updated_at",
            ],
        )],
        "thread_history" => &[
            (
                "thread_turns",
                &[
                    "thread_id",
                    "turn_id",
                    "status",
                    "error_json",
                    "rollout_ordinal",
                    "started_at",
                    "completed_at",
                    "first_user_item_id",
                ],
            ),
            (
                "thread_items",
                &["thread_id", "turn_id", "item_id", "item_type", "item_json"],
            ),
            (
                "thread_history_projection_state",
                &["thread_id", "next_rollout_byte_offset"],
            ),
        ],
        "queue" => &[("queued_items", &["id", "thread_id", "payload_json"])],
        _ => bail!("未知数据库类型"),
    };
    for (table, required) in tables {
        let cols = c
            .prepare(&format!("PRAGMA table_info({table})"))?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<rusqlite::Result<HashSet<_>>>()?;
        if !required.iter().all(|n| cols.contains(*n)) {
            bail!("Codex 数据结构已变更，自动发送已暂停");
        }
    }
    Ok(c)
}
pub fn schemas(home: &Path) -> Result<()> {
    db(home, "state")?;
    db(home, "thread_history")?;
    db(home, "queue")?;
    Ok(())
}

pub struct Account {
    pub key: String,
    pub email: String,
}
// Stable account identity only; refresh/access tokens never leave this function or enter our database.
pub fn account(home: &Path) -> Result<Account> {
    let bytes = zeroize::Zeroizing::new(bounded(&home.join("auth.json"), 1024 * 1024)?);
    let value: Value = serde_json::from_slice(&bytes)?;
    if value["OPENAI_API_KEY"]
        .as_str()
        .is_some_and(|s| !s.is_empty())
        || value["auth_mode"].as_str().is_some_and(|s| s != "chatgpt")
    {
        bail!("首版自动续跑仅支持 ChatGPT 登录模式");
    }
    let token = value["tokens"]["id_token"]
        .as_str()
        .context("当前 ChatGPT 登录身份无法核验")?;
    let part = token.split('.').nth(1).context("登录身份格式不受支持")?;
    let claims: Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part.trim_end_matches('='))?)?;
    let id = value["tokens"]["account_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .context("登录缺少账户标识")?;
    let email = claims["email"]
        .as_str()
        .filter(|s| !s.is_empty())
        .context("登录缺少可核验身份")?
        .to_lowercase();
    Ok(Account {
        key: digest(format!("{id}\n{email}")),
        email,
    })
}
pub fn config_digest(home: &Path) -> Result<String> {
    let p = home.join("config.toml");
    Ok(digest(if p.exists() {
        bounded(&p, 1024 * 1024)?
    } else {
        Vec::new()
    }))
}

#[derive(Clone)]
pub struct Turn {
    pub id: String,
    pub ordinal: i64,
    pub status: String,
    pub quota: bool,
    pub completed: Option<i64>,
}
#[derive(Clone)]
pub struct Frame {
    pub id: String,
    pub title: String,
    pub project: String,
    pub configuration: String,
    pub latest: Option<Turn>,
    pub fresh: bool,
    pub queued: bool,
}
impl Frame {
    pub fn candidate(&self) -> Candidate {
        Candidate {
            thread_id: self.id.clone(),
            title: self.title.clone(),
            project: self.project.clone(),
            quota_stopped: self.latest.as_ref().is_some_and(|t| t.quota),
        }
    }
}
fn is_quota(error: Option<&str>) -> bool {
    let Some(v) = error
        .filter(|s| s.len() < 64 * 1024)
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
    else {
        return false;
    };
    v["codexErrorInfo"] == "usageLimitExceeded"
        || v["codexErrorInfo"]
            .as_object()
            .is_some_and(|m| m.len() == 1 && m.contains_key("usageLimitExceeded"))
}
pub fn frame(home: &Path, id: &str) -> Result<Frame> {
    if !valid_id(id) {
        bail!("请输入完整有效的会话 UUID");
    }
    let c = db(home, "state")?;
    let row=c.query_row("SELECT name,cwd,source,thread_source,history_mode,rollout_path,archived,model_provider,model,sandbox_policy,approval_mode FROM threads WHERE id=?",[id],|r|Ok((r.get::<_,Option<String>>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,i64>(6)?,r.get::<_,String>(7)?,r.get::<_,Option<String>>(8)?,r.get::<_,String>(9)?,r.get::<_,String>(10)?))) .optional()?.context("找不到该会话")?;
    if row.2 != "vscode"
        || !matches!(row.3.as_deref(), None | Some("user" | "chatgpt_handoff"))
        || row.4 != "paginated"
        || row.6 != 0
        || row.7 != "openai"
    {
        bail!("仅支持未归档的官方桌面 ChatGPT 会话");
    }
    let file = PathBuf::from(&row.5)
        .canonicalize()
        .context("会话记录暂不可用")?;
    let sessions = home.join("sessions").canonicalize()?;
    if !file.starts_with(&sessions) || file.extension().is_none_or(|s| s != "jsonl") {
        bail!("会话记录路径不受支持");
    }
    let mut reader = std::io::BufReader::new(std::fs::File::open(&file)?).take(1024 * 1024);
    let mut first = String::new();
    reader.read_line(&mut first)?;
    let meta: Value = serde_json::from_str(&first)?;
    let meta = &meta["payload"];
    if meta["id"] != id
        || !matches!(
            meta["originator"].as_str(),
            Some("Codex Desktop" | "codex_work_desktop" | "codex-tui")
        )
        || !matches!(meta["source"].as_str(), Some("vscode" | "cli"))
        || meta["thread_source"].as_str() != row.3.as_deref()
    {
        bail!("无法核验桌面会话来源");
    }
    // Split/compacted rollout identities need their own chain mapping; never infer an alias from a filename.
    if !file
        .file_name()
        .is_some_and(|s| s.to_string_lossy().ends_with(&format!("{id}.jsonl")))
    {
        bail!("此会话已更换记录标识，等待兼容适配");
    }
    let length = std::fs::metadata(&file)?.len();
    let h = db(home, "thread_history")?;
    let latest=h.query_row("SELECT turn_id,rollout_ordinal,status,error_json,completed_at FROM thread_turns WHERE thread_id=? ORDER BY rollout_ordinal DESC LIMIT 1",[id],|r|{
        let error=r.get::<_,Option<String>>(3)?;let status=r.get::<_,String>(2)?;let completed=r.get::<_,Option<i64>>(4)?;
        Ok(Turn{id:r.get(0)?,ordinal:r.get(1)?,quota:status=="failed"&&completed.is_some()&&is_quota(error.as_deref()),status,completed})
    }).optional()?;
    if latest.as_ref().is_some_and(|t| {
        !valid_id(&t.id)
            || t.ordinal < 0
            || !matches!(
                t.status.as_str(),
                "inProgress" | "completed" | "failed" | "interrupted"
            )
    }) {
        bail!("会话状态格式不受支持");
    }
    let projected:Option<i64>=h.query_row("SELECT next_rollout_byte_offset FROM thread_history_projection_state WHERE thread_id=?",[id],|r|r.get(0)).optional()?;
    let fresh = projected.and_then(|offset| u64::try_from(offset).ok()) == Some(length)
        && std::fs::metadata(&file)?.len() == length;
    let q = db(home, "queue")?;
    let queued = q.query_row(
        "SELECT EXISTS(SELECT 1 FROM queued_items WHERE thread_id=?)",
        [id],
        |r| r.get(0),
    )?;
    let title = label(
        row.0
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("未命名会话"),
        100,
    );
    let project = label(
        Path::new(&row.1)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .as_ref(),
        80,
    );
    let configuration = digest(serde_json::to_vec(&(
        row.1,
        row.7,
        row.8,
        row.9,
        row.10,
        config_digest(home)?,
    ))?);
    Ok(Frame {
        id: id.into(),
        title,
        project,
        configuration,
        latest,
        fresh,
        queued,
    })
}
pub fn candidates(home: &Path) -> Result<Vec<Candidate>> {
    let c = db(home, "state")?;
    let ids=c.prepare("SELECT id FROM threads WHERE source='vscode' AND archived=0 AND model_provider='openai' AND (thread_source IN ('user','chatgpt_handoff') OR thread_source IS NULL) ORDER BY updated_at DESC LIMIT 60")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids
        .into_iter()
        .filter_map(|id| frame(home, &id).ok().map(|f| f.candidate()))
        .collect())
}

#[derive(Debug, PartialEq)]
pub enum Receipt {
    None,
    Queued(String),
    Running,
    Completed,
    Stopped,
    Foreign,
    Unknown,
}
fn contains(v: &Value, marker: &str) -> bool {
    v.as_array().is_some_and(|a| {
        a.iter()
            .any(|v| v["type"] == "text" && v["text"].as_str().is_some_and(|s| s.contains(marker)))
    })
}
pub fn receipt(home: &Path, id: &str, ordinal: i64, marker: &str) -> Result<Receipt> {
    let h = db(home, "thread_history")?;
    let rows=h.prepare("SELECT t.status,i.item_json,t.first_user_item_id FROM thread_turns t LEFT JOIN thread_items i ON i.thread_id=t.thread_id AND i.item_id=t.first_user_item_id AND i.item_type='userMessage' WHERE t.thread_id=? AND t.rollout_ordinal>? ORDER BY t.rollout_ordinal LIMIT 65")?.query_map(params![id,ordinal],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,Option<String>>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut subsequent = Receipt::None;
    for (status, item, first) in &rows {
        if first.is_none() || item.is_none() {
            subsequent = Receipt::Unknown;
            continue;
        }
        let v: Value = serde_json::from_str(item.as_deref().unwrap_or_default())?;
        if v["type"] != "userMessage" || !contains(&v["content"], marker) {
            subsequent = Receipt::Foreign;
        } else {
            return Ok(match status.as_str() {
                "inProgress" => Receipt::Running,
                "completed" => Receipt::Completed,
                "failed" | "interrupted" => Receipt::Stopped,
                _ => Receipt::Unknown,
            });
        }
    }
    if rows.len() > 64 {
        subsequent = Receipt::Unknown;
    }
    let q = db(home, "queue")?;
    let rows=q.prepare("SELECT id,payload_json FROM queued_items WHERE thread_id=? AND instr(payload_json,?)>0 LIMIT 8")?.query_map(params![id,marker],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    for (qid, text) in rows {
        let v: Value = serde_json::from_str(&text)?;
        if valid_id(&qid)
            && v.as_object().is_some_and(|m| m.len() == 1)
            && contains(&v["UserInput"]["content"], marker)
        {
            return Ok(Receipt::Queued(qid));
        }
    }
    Ok(subsequent)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quota_requires_structured_error_not_message_text() {
        assert!(is_quota(Some(r#"{"codexErrorInfo":"usageLimitExceeded"}"#)));
        for s in [
            r#"{"message":"usageLimitExceeded"}"#,
            r#"{"codexErrorInfo":"rateLimitExceeded"}"#,
            r#"{"codexErrorInfo":"unauthorized"}"#,
            "invalid",
        ] {
            assert!(!is_quota(Some(s)));
        }
    }
    #[test]
    fn newest_schema_never_falls_back_to_stale_generation() {
        let dir = crate::test_support::FixtureDir::new();
        let h = dir.path().canonicalize().unwrap();
        Connection::open(h.join("queue_1.sqlite"))
            .unwrap()
            .execute_batch("CREATE TABLE queued_items(id TEXT,thread_id TEXT,payload_json TEXT)")
            .unwrap();
        assert!(db(&h, "queue").is_ok());
        Connection::open(h.join("queue_2.sqlite")).unwrap();
        assert!(db(&h, "queue").is_err());
    }
    struct Fixture {
        dir: crate::test_support::FixtureDir,
        id: String,
        turn: String,
        file: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let dir = crate::test_support::FixtureDir::new();
            let home = dir.path().canonicalize().unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            let turn = uuid::Uuid::new_v4().to_string();
            std::fs::create_dir(home.join("sessions")).unwrap();
            let file = home.join("sessions").join(format!("rollout-{id}.jsonl"));
            std::fs::write(&file,format!("{}\n",serde_json::json!({"type":"session_meta","payload":{"id":id,"source":"vscode","originator":"Codex Desktop","thread_source":"user"}}))).unwrap();
            let c = Connection::open(home.join("state_1.sqlite")).unwrap();
            c.execute_batch("CREATE TABLE threads(id TEXT,name TEXT,cwd TEXT,source TEXT,thread_source TEXT,history_mode TEXT,rollout_path TEXT,archived INTEGER,model_provider TEXT,model TEXT,sandbox_policy TEXT,approval_mode TEXT,updated_at INTEGER)").unwrap();
            c.execute("INSERT INTO threads VALUES(?, '示例会话','C:/fixture','vscode','user','paginated',?,0,'openai','fixture-model','workspace-write','on-request',100)",params![id,file.to_string_lossy()]).unwrap();
            let h = Connection::open(home.join("thread_history_1.sqlite")).unwrap();
            h.execute_batch("CREATE TABLE thread_turns(thread_id TEXT,turn_id TEXT,status TEXT,error_json TEXT,rollout_ordinal INTEGER,started_at INTEGER,completed_at INTEGER,first_user_item_id TEXT);CREATE TABLE thread_items(thread_id TEXT,turn_id TEXT,item_id TEXT,item_type TEXT,item_json TEXT);CREATE TABLE thread_history_projection_state(thread_id TEXT,next_rollout_byte_offset INTEGER)").unwrap();
            h.execute(
                "INSERT INTO thread_turns VALUES(?,?,'failed',?,1,100,101,NULL)",
                params![id, turn, r#"{"codexErrorInfo":"usageLimitExceeded"}"#],
            )
            .unwrap();
            h.execute(
                "INSERT INTO thread_history_projection_state VALUES(?,?)",
                params![id, std::fs::metadata(&file).unwrap().len() as i64],
            )
            .unwrap();
            Connection::open(home.join("queue_1.sqlite"))
                .unwrap()
                .execute_batch(
                    "CREATE TABLE queued_items(id TEXT,thread_id TEXT,payload_json TEXT)",
                )
                .unwrap();
            Self {
                dir,
                id,
                turn,
                file,
            }
        }
        fn home(&self) -> PathBuf {
            self.dir.path().canonicalize().unwrap()
        }
        fn add_turn(&self, text: &str, status: &str) {
            let h = Connection::open(self.home().join("thread_history_1.sqlite")).unwrap();
            let turn = uuid::Uuid::new_v4().to_string();
            h.execute(
                "INSERT INTO thread_turns VALUES(?,?,?,NULL,2,102,NULL,'u')",
                params![self.id, turn, status],
            )
            .unwrap();
            h.execute("INSERT INTO thread_items VALUES(?,?,'u','userMessage',?)",params![self.id,turn,serde_json::json!({"type":"userMessage","content":[{"type":"text","text":text}]}).to_string()]).unwrap();
        }
    }
    #[test]
    fn only_selected_desktop_identity_and_current_projection_are_eligible() {
        let f = Fixture::new();
        let frame = frame(&f.home(), &f.id).unwrap();
        assert!(frame.fresh && frame.latest.as_ref().unwrap().quota);
        assert_eq!(frame.latest.unwrap().id, f.turn);
        assert!(super::frame(&f.home(), &uuid::Uuid::new_v4().to_string()).is_err());
        {
            use std::io::Write;
            std::fs::OpenOptions::new()
                .append(true)
                .open(&f.file)
                .unwrap()
                .write_all(b"{}\n")
                .unwrap();
        }
        assert!(!super::frame(&f.home(), &f.id).unwrap().fresh);
        let c = Connection::open(f.home().join("state_1.sqlite")).unwrap();
        c.execute("UPDATE threads SET archived=1", []).unwrap();
        assert!(super::frame(&f.home(), &f.id).is_err());
        c.execute("UPDATE threads SET archived=0,thread_source='subagent'", [])
            .unwrap();
        assert!(super::frame(&f.home(), &f.id).is_err());
    }
    #[test]
    fn queue_acceptance_is_distinct_from_started_turn_and_exact_marker() {
        let f = Fixture::new();
        let marker = "[fixture-unique-marker]";
        let q = Connection::open(f.home().join("queue_1.sqlite")).unwrap();
        let qid = uuid::Uuid::new_v4().to_string();
        q.execute(
            "INSERT INTO queued_items VALUES(?,?,?)",
            params![
                qid,
                f.id,
                serde_json::json!({"UserInput":{"content":[{"type":"text","text":marker}]}})
                    .to_string()
            ],
        )
        .unwrap();
        assert_eq!(
            receipt(&f.home(), &f.id, 1, marker).unwrap(),
            Receipt::Queued(qid.clone())
        );
        assert_eq!(
            receipt(&f.home(), &f.id, 1, "other").unwrap(),
            Receipt::None
        );
        f.add_turn(marker, "inProgress");
        assert_eq!(
            receipt(&f.home(), &f.id, 1, marker).unwrap(),
            Receipt::Running
        );
        let h = Connection::open(f.home().join("thread_history_1.sqlite")).unwrap();
        h.execute(
            "UPDATE thread_turns SET status='completed' WHERE rollout_ordinal=2",
            [],
        )
        .unwrap();
        assert_eq!(
            receipt(&f.home(), &f.id, 1, marker).unwrap(),
            Receipt::Completed
        );
    }
    #[test]
    fn user_takeover_does_not_hide_a_continuation_still_queued() {
        let f = Fixture::new();
        let marker = "[fixture-marker]";
        f.add_turn("用户自己继续了", "inProgress");
        assert_eq!(
            receipt(&f.home(), &f.id, 1, marker).unwrap(),
            Receipt::Foreign
        );
        let q = Connection::open(f.home().join("queue_1.sqlite")).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        q.execute(
            "INSERT INTO queued_items VALUES(?,?,?)",
            params![
                id,
                f.id,
                serde_json::json!({"UserInput":{"content":[{"type":"text","text":marker}]}})
                    .to_string()
            ],
        )
        .unwrap();
        assert_eq!(
            receipt(&f.home(), &f.id, 1, marker).unwrap(),
            Receipt::Queued(id)
        );
    }
    #[tokio::test]
    async fn simulated_reset_dispatch_receipt_and_restart_never_touch_real_codex() {
        use super::super::{adapter, model, scheduler, store::Store};
        let f = Fixture::new();
        let home = f.home();
        let app_data = home.join("fake-yunkuo-data");
        let mut store = Store::open(&app_data).unwrap();
        let frame = frame(&home, &f.id).unwrap();
        let watch = model::Watch {
            thread_id: f.id.clone(),
            home_key: home_key(&home),
            title: "fixture".into(),
            project: "fixture".into(),
            account: "fake-account".into(),
            configuration: frame.configuration.clone(),
            enabled: true,
            consumed: false,
            created_at: 100,
        };
        store.enabled_set(true, 100).unwrap();
        store.put_watch(&watch).unwrap();
        scheduler::observe(&store, &watch, &frame, 101, true).unwrap();
        let mut record = store.records().unwrap().remove(0);
        let limited = model::quota(
            &serde_json::json!({"rateLimits":{"primary":{"usedPercent":100,"windowDurationMins":300,"resetsAt":200}}}),
            101,
        );
        record.reset_at = limited.reset_at;
        store.update(&record).unwrap();
        assert_eq!(limited.available, Some(false));
        assert!(store.claim(&record.id, 199).unwrap().is_none());
        let restored = model::quota(
            &serde_json::json!({"rateLimits":{"primary":{"usedPercent":0,"windowDurationMins":300,"resetsAt":500}}}),
            202,
        );
        assert_eq!(restored.available, Some(true));
        record.reset_at = None;
        store.update(&record).unwrap();
        let mut claimed = store.claim(&record.id, 202).unwrap().unwrap();
        // The fake process writes only this test's queue DB, then emits the same receipt shape as Codex.
        let script = r#"const {DatabaseSync}=require('node:sqlite');const [file,thread,id,marker]=process.argv.slice(1);const d=new DatabaseSync(file);d.prepare('INSERT INTO queued_items VALUES(?,?,?)').run(id,thread,JSON.stringify({UserInput:{content:[{type:'text',text:marker}]}}));d.close();process.stdout.write(`Queued message ${id} for thread ${thread}.\n`);"#;
        let queue_id = uuid::Uuid::new_v4().to_string();
        let child = tokio::process::Command::new("node")
            .args(["--eval", script])
            .arg(home.join("queue_1.sqlite"))
            .args([&f.id, &queue_id, &claimed.marker])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .creation_flags(0x08000000)
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        assert_eq!(adapter::finish(child, &f.id).await, Some(queue_id.clone()));
        claimed.queue_id = Some(queue_id.clone());
        claimed.set("queued", "fake queue accepted", 203, 1);
        store.update(&claimed).unwrap();
        assert_eq!(
            receipt(&home, &f.id, 1, &claimed.marker).unwrap(),
            Receipt::Queued(queue_id)
        );
        drop(store);
        let mut restarted = Store::open(&app_data).unwrap();
        restarted.recover_crash(204).unwrap();
        assert!(restarted.claim(&record.id, 204).unwrap().is_none());
        f.add_turn(&claimed.marker, "inProgress");
        assert_eq!(
            receipt(&home, &f.id, 1, &claimed.marker).unwrap(),
            Receipt::Running
        );
    }
}
