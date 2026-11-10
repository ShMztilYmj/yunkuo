use super::model::*;
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

pub struct Store(Connection);
impl Store {
    pub fn open(root: &Path) -> Result<Self> {
        Self::open_for(root, "codex")
    }
    pub fn open_for(root: &Path, tool: &str) -> Result<Self> {
        let filename = match tool {
            "codex" => "codex-resume.sqlite",
            "claude" => "claude-resume.sqlite",
            _ => bail!("不支持的恢复记录类型"),
        };
        std::fs::create_dir_all(root)?;
        let c = Connection::open(root.join(filename))?;
        c.busy_timeout(std::time::Duration::from_secs(2))?;
        c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA trusted_schema=OFF;
          CREATE TABLE IF NOT EXISTS resume_settings(id INTEGER PRIMARY KEY CHECK(id=1),enabled INTEGER NOT NULL);
          INSERT OR IGNORE INTO resume_settings VALUES(1,0);
          CREATE TABLE IF NOT EXISTS resume_watches(home TEXT NOT NULL,thread TEXT NOT NULL,body TEXT NOT NULL,PRIMARY KEY(home,thread));
          CREATE TABLE IF NOT EXISTS resume_records(id TEXT PRIMARY KEY,home TEXT NOT NULL,thread TEXT NOT NULL,body TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS resume_events(id INTEGER PRIMARY KEY,recovery TEXT NOT NULL,at INTEGER NOT NULL,state TEXT NOT NULL,reason TEXT NOT NULL);")?;
        Ok(Self(c))
    }
    pub fn enabled(&self) -> Result<bool> {
        Ok(self
            .0
            .query_row("SELECT enabled FROM resume_settings WHERE id=1", [], |r| {
                r.get::<_, i32>(0)
            })?
            == 1)
    }
    pub fn watches(&self) -> Result<Vec<Watch>> {
        let mut stmt = self
            .0
            .prepare("SELECT body FROM resume_watches ORDER BY rowid DESC LIMIT 32")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn watch(&self, home: &str, id: &str) -> Result<Option<Watch>> {
        let text: Option<String> = self
            .0
            .query_row(
                "SELECT body FROM resume_watches WHERE home=? AND thread=?",
                params![home, id],
                |r| r.get(0),
            )
            .optional()?;
        text.map(|t| serde_json::from_str(&t).map_err(Into::into))
            .transpose()
    }
    pub fn put_watch(&self, w: &Watch) -> Result<()> {
        if self.watch(&w.home_key, &w.thread_id)?.is_none() && self.watches()?.len() >= 32 {
            bail!("最多同时保留 32 个守候会话，请先移除旧会话");
        }
        self.0.execute("INSERT INTO resume_watches VALUES(?,?,?) ON CONFLICT(home,thread) DO UPDATE SET body=excluded.body",params![w.home_key,w.thread_id,serde_json::to_string(w)?])?;
        Ok(())
    }
    pub fn records(&self) -> Result<Vec<Recovery>> {
        let mut stmt = self
            .0
            .prepare("SELECT body FROM resume_records ORDER BY rowid DESC")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn record(&self, id: &str) -> Result<Option<Recovery>> {
        let text: Option<String> = self
            .0
            .query_row("SELECT body FROM resume_records WHERE id=?", [id], |r| {
                r.get(0)
            })
            .optional()?;
        text.map(|s| serde_json::from_str(&s).map_err(Into::into))
            .transpose()
    }
    pub fn insert(&self, r: &Recovery) -> Result<bool> {
        let n = self.0.execute(
            "INSERT OR IGNORE INTO resume_records VALUES(?,?,?,?)",
            params![r.id, r.home_key, r.thread_id, serde_json::to_string(r)?],
        )?;
        if n > 0 {
            self.event(r)?;
        }
        Ok(n > 0)
    }
    fn event(&self, r: &Recovery) -> Result<()> {
        self.0.execute(
            "INSERT INTO resume_events(recovery,at,state,reason) VALUES(?,?,?,?)",
            params![r.id, r.updated_at, r.state, r.reason],
        )?;
        self.0.execute("DELETE FROM resume_events WHERE id NOT IN (SELECT id FROM resume_events ORDER BY id DESC LIMIT 512)",[])?;
        Ok(())
    }
    pub fn update(&self, r: &Recovery) -> Result<()> {
        let old = self.record(&r.id)?.context("恢复记录不存在")?;
        self.0.execute(
            "UPDATE resume_records SET body=? WHERE id=?",
            params![serde_json::to_string(r)?, r.id],
        )?;
        if old.state != r.state || old.reason != r.reason {
            self.event(r)?;
        }
        Ok(())
    }
    // Called while the runtime's short dispatch fence is held. SQLite also prevents a second owner claiming it.
    pub fn claim(&mut self, id: &str, now: i64) -> Result<Option<Recovery>> {
        let tx = self
            .0
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let enabled: bool =
            tx.query_row("SELECT enabled FROM resume_settings WHERE id=1", [], |r| {
                r.get(0)
            })?;
        let text: Option<String> = tx
            .query_row("SELECT body FROM resume_records WHERE id=?", [id], |r| {
                r.get(0)
            })
            .optional()?;
        let Some(mut r) = text
            .map(|s| serde_json::from_str::<Recovery>(&s))
            .transpose()?
        else {
            return Ok(None);
        };
        let w: Option<String> = tx
            .query_row(
                "SELECT body FROM resume_watches WHERE home=? AND thread=?",
                params![r.home_key, r.thread_id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(mut w) = w.map(|s| serde_json::from_str::<Watch>(&s)).transpose()? else {
            return Ok(None);
        };
        if !enabled
            || !w.enabled
            || w.consumed
            || r.cancel_requested
            || !r.waiting()
            || r.next_check > now
            || r.reset_at.is_some_and(|t| t > now)
        {
            return Ok(None);
        }
        if w.account != r.account || w.configuration != r.configuration {
            return Ok(None);
        }
        let other = tx
            .prepare("SELECT body FROM resume_records WHERE home=? AND id<>?")?
            .query_map(params![r.home_key, r.id], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for text in other {
            let other: Recovery = serde_json::from_str(&text)?;
            if other.in_flight() && other.state != "running" {
                return Ok(None);
            }
        }
        r.claimed_at = Some(now);
        r.set("submitting", "正在提交继续消息", now, 2);
        w.consumed = true;
        tx.execute(
            "UPDATE resume_records SET body=? WHERE id=?",
            params![serde_json::to_string(&r)?, r.id],
        )?;
        tx.execute(
            "UPDATE resume_watches SET body=? WHERE home=? AND thread=?",
            params![serde_json::to_string(&w)?, w.home_key, w.thread_id],
        )?;
        tx.execute(
            "INSERT INTO resume_events(recovery,at,state,reason) VALUES(?,?,?,?)",
            params![r.id, now, r.state, r.reason],
        )?;
        tx.commit()?;
        Ok(Some(r))
    }
    pub fn enabled_set(&self, on: bool, now: i64) -> Result<()> {
        self.0
            .execute("UPDATE resume_settings SET enabled=? WHERE id=1", [on])?;
        if !on {
            for mut w in self.watches()? {
                w.enabled = false;
                self.put_watch(&w)?;
            }
            for mut r in self.records()? {
                if r.waiting() || r.in_flight() {
                    r.cancel_requested = true;
                    if r.claimed_at.is_none() {
                        r.set("cancelled", "自动续跑已关闭", now, 0);
                    }
                    self.update(&r)?;
                }
            }
        }
        Ok(())
    }
    pub fn cancel(&self, home: &str, thread: &str, now: i64) -> Result<()> {
        if let Some(mut w) = self.watch(home, thread)? {
            w.enabled = false;
            self.put_watch(&w)?;
        }
        for mut r in self.records()? {
            if r.home_key == home && r.thread_id == thread && (r.waiting() || r.in_flight()) {
                r.cancel_requested = true;
                if r.claimed_at.is_none() {
                    r.set("cancelled", "已取消本次恢复", now, 0);
                } else {
                    r.reason = "已停止后续恢复，正在核实已提交消息".into();
                    r.updated_at = now;
                }
                self.update(&r)?;
            }
        }
        Ok(())
    }
    pub fn forget(&self, home: &str, thread: &str) -> Result<()> {
        if self
            .records()?
            .iter()
            .any(|r| r.home_key == home && r.thread_id == thread && r.in_flight())
        {
            bail!("消息仍在核实或运行中，请先等待结果；历史去重记录会保留");
        }
        self.0.execute(
            "DELETE FROM resume_watches WHERE home=? AND thread=?",
            params![home, thread],
        )?;
        Ok(())
    }
    /// 按会话 ID 取消全部 home_key 下的守候。
    ///
    /// 命令行入口曾经用"当前 home"去取消，CODEX_HOME/CLAUDE_HOME 改过或目录消失后
    /// 会静默命中 0 行：界面显示已关闭，调度器却继续守着该会话。
    pub fn cancel_thread(&self, thread: &str, now: i64) -> Result<()> {
        for w in self.watches()? {
            if w.thread_id == thread {
                self.cancel(&w.home_key, thread, now)?;
            }
        }
        Ok(())
    }
    /// 与 `cancel_thread` 同理，额外删除守候记录。
    pub fn forget_thread(&self, thread: &str, now: i64) -> Result<()> {
        for w in self.watches()? {
            if w.thread_id == thread {
                self.cancel(&w.home_key, thread, now)?;
                self.forget(&w.home_key, thread)?;
            }
        }
        Ok(())
    }
    pub fn events(&self) -> Result<Vec<Event>> {
        Ok(self
            .0
            .prepare(
                "SELECT recovery,at,state,reason FROM resume_events ORDER BY id DESC LIMIT 48",
            )?
            .query_map([], |r| {
                Ok(Event {
                    recovery_id: r.get(0)?,
                    at: r.get(1)?,
                    state: r.get(2)?,
                    reason: r.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn recover_crash(&self, now: i64) -> Result<()> {
        for mut r in self.records()? {
            if r.state == "submitting" {
                r.set(
                    "submission_unknown",
                    "上次提交结果不明，核实前不会重发",
                    now,
                    0,
                );
                self.update(&r)?;
            }
        }
        Ok(())
    }
    pub fn expire_watches(&self, now: i64) -> Result<()> {
        for mut w in self.watches()? {
            if w.enabled && !w.consumed && now - w.created_at > 7 * 86400 {
                w.enabled = false;
                w.consumed = true;
                self.put_watch(&w)?;
                for mut r in self.records()? {
                    if r.home_key == w.home_key && r.thread_id == w.thread_id && r.waiting() {
                        r.set("expired", "守候已超过 7 天，请重新选择会话", now, 0);
                        self.update(&r)?;
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn claude_and_codex_settings_watches_and_claims_are_isolated() {
        let root = crate::test_support::FixtureDir::new();
        let codex = Store::open(root.path()).unwrap();
        let mut claude = Store::open_for(root.path(), "claude").unwrap();
        let r = fixture(&claude);
        claude.insert(&r).unwrap();
        claude.enabled_set(true, 100).unwrap();
        assert!(!codex.enabled().unwrap());
        assert!(codex.watches().unwrap().is_empty());
        assert!(codex.records().unwrap().is_empty());
        assert!(claude.claim(&r.id, 100).unwrap().is_some());
        codex.enabled_set(false, 101).unwrap();
        assert!(claude.enabled().unwrap());
        assert!(!claude.record(&r.id).unwrap().unwrap().cancel_requested);
        let mut restarted = Store::open_for(root.path(), "claude").unwrap();
        restarted.recover_crash(102).unwrap();
        assert!(restarted.claim(&r.id, 102).unwrap().is_none());
        assert!(Store::open_for(root.path(), "../unsafe").is_err());
    }
    #[test]
    fn cancelling_a_thread_covers_every_stored_home() {
        let root = crate::test_support::FixtureDir::new();
        let store = Store::open(root.path()).unwrap();
        let thread = "fixture-thread";
        for home in ["old-home", "new-home"] {
            store
                .put_watch(&Watch {
                    thread_id: thread.into(),
                    home_key: home.into(),
                    enabled: true,
                    account: "a".into(),
                    configuration: "c".into(),
                    ..Default::default()
                })
                .unwrap();
        }
        // home_key 变化后取消仍要覆盖旧记录：这正是命令行入口曾经漏掉的情况。
        store.cancel_thread(thread, 100).unwrap();
        let watches = store.watches().unwrap();
        assert_eq!(watches.len(), 2);
        assert!(watches.iter().all(|w| !w.enabled));
        store.forget_thread(thread, 101).unwrap();
        assert!(store.watches().unwrap().is_empty());
    }
    /// 取消只作用于目标会话，且重启/崩溃恢复/过期清理都不能让它复活。
    #[test]
    fn cancelled_watch_never_revives_after_restart() {
        let root = crate::test_support::FixtureDir::new();
        let mut store = Store::open(root.path()).unwrap();
        store.enabled_set(true, 100).unwrap();
        let cancelled = fixture(&store);
        assert!(store.insert(&cancelled).unwrap());
        let other = fixture(&store);
        assert!(store.insert(&other).unwrap());
        store.cancel_thread(&cancelled.thread_id, 101).unwrap();
        // 相邻会话仍可认领：取消不能顺带波及别的守候。
        assert!(store.claim(&other.id, 101).unwrap().is_some());
        drop(store);
        let mut restarted = Store::open(root.path()).unwrap();
        restarted.recover_crash(102).unwrap();
        restarted.expire_watches(102).unwrap();
        assert!(restarted.claim(&cancelled.id, 103).unwrap().is_none());
        let watch = restarted
            .watch(&cancelled.home_key, &cancelled.thread_id)
            .unwrap()
            .unwrap();
        assert!(!watch.enabled);
    }
    pub fn fixture(s: &Store) -> Recovery {
        let w = Watch {
            thread_id: uuid::Uuid::new_v4().to_string(),
            home_key: "fixture-home".into(),
            enabled: true,
            account: "a".into(),
            configuration: "c".into(),
            ..Default::default()
        };
        s.put_watch(&w).unwrap();
        Recovery {
            id: uuid::Uuid::new_v4().to_string(),
            thread_id: w.thread_id,
            home_key: w.home_key,
            turn_id: uuid::Uuid::new_v4().to_string(),
            ordinal: 2,
            account: w.account,
            configuration: w.configuration,
            title: "示例".into(),
            project: "demo".into(),
            state: "waiting_quota".into(),
            reason: "等待".into(),
            marker: "fixture".into(),
            reset_at: None,
            next_check: 100,
            checked_at: None,
            created_at: 100,
            updated_at: 100,
            claimed_at: None,
            queue_id: None,
            cancel_requested: false,
            spawn_failures: 0,
            quota_confirmed: false,
        }
    }
    #[test]
    fn one_claim_survives_duplicate_scans_and_restart() {
        let root = crate::test_support::FixtureDir::new();
        let mut a = Store::open(root.path()).unwrap();
        let r = fixture(&a);
        assert!(a.insert(&r).unwrap());
        assert!(!a.insert(&r).unwrap());
        assert!(a.claim(&r.id, 100).unwrap().is_none());
        a.enabled_set(true, 100).unwrap();
        assert!(a.claim(&r.id, 100).unwrap().is_some());
        let mut b = Store::open(root.path()).unwrap();
        assert!(b.claim(&r.id, 101).unwrap().is_none());
        b.recover_crash(102).unwrap();
        assert_eq!(
            b.record(&r.id).unwrap().unwrap().state,
            "submission_unknown"
        );
        assert!(b.claim(&r.id, 103).unwrap().is_none());
    }
    #[test]
    fn cancel_and_reset_time_block_claims() {
        let root = crate::test_support::FixtureDir::new();
        let mut s = Store::open(root.path()).unwrap();
        let mut r = fixture(&s);
        r.reset_at = Some(200);
        s.insert(&r).unwrap();
        s.enabled_set(true, 100).unwrap();
        assert!(s.claim(&r.id, 199).unwrap().is_none());
        s.cancel(&r.home_key, &r.thread_id, 200).unwrap();
        assert!(s.claim(&r.id, 201).unwrap().is_none());
    }
    #[test]
    fn account_mismatch_and_other_in_flight_recovery_block_submission() {
        let dir = crate::test_support::FixtureDir::new();
        let mut s = Store::open(dir.path()).unwrap();
        s.enabled_set(true, 100).unwrap();
        let a = fixture(&s);
        s.insert(&a).unwrap();
        let mut w = s.watch(&a.home_key, &a.thread_id).unwrap().unwrap();
        w.account = "different".into();
        s.put_watch(&w).unwrap();
        assert!(s.claim(&a.id, 100).unwrap().is_none());
        w.account = a.account.clone();
        s.put_watch(&w).unwrap();
        assert!(s.claim(&a.id, 100).unwrap().is_some());
        let b = fixture(&s);
        s.insert(&b).unwrap();
        assert!(s.claim(&b.id, 101).unwrap().is_none());
    }
    #[test]
    fn switching_off_preserves_possible_delivery_and_prevents_new_claims() {
        let dir = crate::test_support::FixtureDir::new();
        let mut s = Store::open(dir.path()).unwrap();
        s.enabled_set(true, 100).unwrap();
        let a = fixture(&s);
        s.insert(&a).unwrap();
        s.claim(&a.id, 100).unwrap().unwrap();
        s.enabled_set(false, 101).unwrap();
        let r = s.record(&a.id).unwrap().unwrap();
        assert!(r.cancel_requested && r.in_flight());
        assert!(s.forget(&a.home_key, &a.thread_id).is_err());
        assert!(s.claim(&a.id, 102).unwrap().is_none());
    }
    #[test]
    fn concurrent_owners_can_only_claim_once() {
        let root = crate::test_support::FixtureDir::new();
        let s = Store::open(root.path()).unwrap();
        let r = fixture(&s);
        s.insert(&r).unwrap();
        s.enabled_set(true, 100).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let path = root.path().to_owned();
            let id = r.id.clone();
            let barrier = barrier.clone();
            handles.push(std::thread::spawn(move || {
                let mut s = Store::open(&path).unwrap();
                barrier.wait();
                s.claim(&id, 100).unwrap().is_some()
            }));
        }
        let count = handles
            .into_iter()
            .map(|h| usize::from(h.join().unwrap()))
            .sum::<usize>();
        assert_eq!(count, 1);
    }
    #[test]
    fn idle_watch_expires_before_it_can_resume_unrelated_future_work() {
        let root = crate::test_support::FixtureDir::new();
        let s = Store::open(root.path()).unwrap();
        let r = fixture(&s);
        s.insert(&r).unwrap();
        s.expire_watches(8 * 86400).unwrap();
        assert!(!s.watch(&r.home_key, &r.thread_id).unwrap().unwrap().enabled);
        assert_eq!(s.record(&r.id).unwrap().unwrap().state, "expired");
    }
}
