use super::{State, adapter, changed, db, now, protocol, source};
use crate::workspace::resume::{model::*, runner, store::Store, workflow};
use anyhow::Result;
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tauri::Manager;
use tauri_plugin_notification::NotificationExt;

pub fn observe(s: &Store, w: &Watch, f: &source::Frame, explicit: bool) -> Result<()> {
    if f.id != w.thread_id || !f.quota || !f.fresh || (!explicit && f.event_at < w.created_at - 2) {
        return Ok(());
    }
    runner::observe(
        s,
        w,
        now(),
        explicit,
        runner::Observation {
            id: digest(format!("{}:{}:{}", w.home_key, f.id, f.anchor)),
            thread_id: &f.id,
            turn_id: f.anchor.clone(),
            ordinal: 0,
            title: &w.title,
            project: &w.project,
            reason: "检测到限流，正在区分订阅额度与普通限流",
            marker: format!("[云阔-Claude-续跑:{}]", uuid::Uuid::new_v4()),
            rearm_reason: "已重新守候，等待订阅额度核验",
            quota_confirmed: f.quota_proven,
            reset_quota: true,
        },
    )
}
fn may_update(current: &Recovery, incoming: &Recovery) -> bool {
    current.marker == incoming.marker
        && current.account == incoming.account
        && current.configuration == incoming.configuration
        && !(current.claimed_at.is_some() && incoming.claimed_at.is_none())
        && !(current.cancel_requested
            && !incoming.cancel_requested
            && incoming.claimed_at.is_none())
}
fn save(app: &tauri::AppHandle, r: &Recovery) -> Result<()> {
    let state = app.state::<State>();
    let _g = state.fence.lock();
    let s = db(app)?;
    if let Some(current) = s.record(&r.id)? {
        if !may_update(&current, r) {
            return Ok(());
        }
        let mut r = r.clone();
        r.cancel_requested |= current.cancel_requested;
        s.update(&r)?;
    }
    changed(app);
    Ok(())
}
fn wait(app: &tauri::AppHandle, r: &Recovery, state: &str, reason: &str, delay: i64) -> Result<()> {
    save(app, &r.advance(state, reason, delay))
}
fn end(app: &tauri::AppHandle, r: &Recovery, state: &str, reason: &str) -> Result<()> {
    {
        let runtime = app.state::<State>();
        let _g = runtime.fence.lock();
        let s = db(app)?;
        if !s
            .record(&r.id)?
            .is_some_and(|current| may_update(&current, r))
        {
            return Ok(());
        }
        workflow::consume_watch(&s, r)?;
    }
    wait(app, r, state, reason, 0)
}
fn unchanged(r: &Recovery, f: &source::Frame) -> bool {
    f.fresh && f.quota && f.anchor == r.turn_id && f.configuration == r.configuration
}
async fn run(
    app: tauri::AppHandle,
    mut child: tokio::process::Child,
    job: crate::platform::process_job::ProcessJob,
    mut r: Recovery,
    cancel: Arc<AtomicBool>,
) {
    let thread = r.thread_id.clone();
    let marker = r.marker.clone();
    let result = if let (Some(mut input), Some(output)) = (child.stdin.take(), child.stdout.take())
    {
        protocol::exchange(
            &mut input,
            output,
            &thread,
            &marker,
            || cancel.load(Ordering::SeqCst) || app.state::<State>().stop.load(Ordering::SeqCst),
            |progress| {
                match progress {
                    protocol::Progress::Accepted => {
                        r.set("queued", "原会话已接收继续消息，等待运行", now(), 5)
                    }
                    protocol::Progress::Running => {
                        r.set("running", "已确认 Claude 原会话开始续跑", now(), 5);
                        let _ = app
                            .notification()
                            .builder()
                            .title("云阔 · Claude 自动续跑")
                            .body("已确认所选 Claude 会话开始续跑")
                            .show();
                    }
                }
                save(&app, &r)
            },
            Duration::from_secs(3600),
        )
        .await
    } else {
        Err(anyhow::anyhow!("恢复输入输出不可用"))
    };
    let stopped = adapter::stop_process(&mut child, job).await;
    let (state, reason) = if stopped.is_err() {
        ("submission_unknown", "进程停止结果待核实；不会重发")
    } else {
        result.unwrap_or(("submission_unknown", "恢复连接中断，结果待核实；不会重发"))
    };
    r.set(state, reason, now(), 0);
    let _ = save(&app, &r);
    app.state::<State>().running.lock().remove(&r.id);
    app.state::<State>().wake.notify_one();
    changed(&app);
}

async fn attempt(app: &tauri::AppHandle, home: &Path, mut r: Recovery) -> Result<()> {
    let f = source::frame(home, &r.thread_id, None)?;
    if !f.fresh {
        return wait(app, &r, "waiting_history", "等待完整会话记录", 5);
    }
    if f.anchor != r.turn_id || !f.quota {
        return end(
            app,
            &r,
            "superseded",
            "原中断已变化，用户已接手或会话已完成",
        );
    }
    if f.configuration != r.configuration {
        return wait(
            app,
            &r,
            "paused_configuration",
            "项目或模型配置已变化，请取消后重新选择",
            60,
        );
    }
    if f.model.is_empty() {
        return wait(
            app,
            &r,
            "paused_configuration",
            "无法核验原模型，请先在 Claude Code 中确认后继续",
            60,
        );
    }
    if source::login(home)?.key != r.account {
        return wait(
            app,
            &r,
            "paused_identity",
            "登录已变化，请取消后重新选择会话",
            60,
        );
    }
    let state = app.state::<State>();
    if now() < state.cooldown.load(Ordering::SeqCst) {
        return wait(app, &r, "waiting_quota", "额度查询处于服务端冷却期", 30);
    }
    let (_, q) = match adapter::usage(home).await {
        Ok(v) => v,
        Err(e) => {
            let delay = e
                .downcast_ref::<adapter::Cooldown>()
                .map(|c| c.0 as i64)
                .unwrap_or(60);
            state.cooldown.store(now() + delay, Ordering::SeqCst);
            return wait(app, &r, "waiting_quota", &adapter::public_error(&e), delay);
        }
    };
    r.checked_at = Some(now());
    if q.available == Some(false) {
        r.quota_confirmed = true;
        r.reset_at = q.reset_at;
        let delay = q
            .reset_at
            .map(|t| (t + 2 - now()).clamp(2, 60))
            .unwrap_or(60);
        r.set(
            "waiting_reset",
            "订阅额度仍不足，等待全部阻塞窗口恢复",
            now(),
            delay,
        );
        return save(app, &r);
    }
    if q.available.is_none() {
        return wait(app, &r, "waiting_quota", "实时额度数据不完整，等待核验", 60);
    }
    if !r.quota_confirmed {
        return end(
            app,
            &r,
            "stopped",
            "未能证明是订阅额度耗尽，普通限流不自动续跑",
        );
    }
    r.reset_at = None;
    r.next_check = now();
    save(app, &r)?;
    if adapter::other_claude_running() {
        return wait(
            app,
            &r,
            "waiting_thread",
            "检测到其他 Claude 实例，请先关闭以释放原会话",
            10,
        );
    }
    let (exe, _) = adapter::check(home).await?;
    let signature = adapter::fingerprint(&exe)?;
    let ai = app.state::<crate::ai::management::State>();
    let _ai = ai.operation.lock().await;
    let env = app.state::<crate::platform::state::EnvironmentOperation>();
    let _env = env.lock().await;
    let cfg = app.state::<crate::platform::state::ConfigUpdateMutex>();
    let _cfg = cfg.lock().await;
    if !crate::ai::execution_policy::check_launch_locked(app, "claude")
        .await
        .1
        .ok
    {
        return wait(
            app,
            &r,
            "waiting_environment",
            "环境保护检查未通过，请在环境检测中处理",
            30,
        );
    }
    let launch = {
        let state = app.state::<State>();
        let _g = state.fence.lock();
        let mut s = db(app)?;
        let Some(current) = s.record(&r.id)? else {
            return Ok(());
        };
        if state.stop.load(Ordering::SeqCst)
            || current.cancel_requested
            || current.marker != r.marker
            || !current.quota_confirmed
            || !s.enabled()?
            || !state.running.lock().is_empty()
        {
            return Ok(());
        }
        let f = source::frame(home, &r.thread_id, None)?;
        if !unchanged(&current, &f)
            || source::login(home)?.key != r.account
            || adapter::fingerprint(&exe)? != signature
            || now() - r.checked_at.unwrap_or(0) > 25
            || adapter::other_claude_running()
        {
            return Ok(());
        }
        let Some(mut claimed) = s.claim(&r.id, now())? else {
            return Ok(());
        };
        let mut c = adapter::resume_command(&exe, &f.cwd, &r.thread_id, &f.model);
        match c.spawn() {
            Ok(mut child) => match crate::platform::process_job::ProcessJob::attach(&child) {
                Ok(job) => {
                    let cancel = Arc::new(AtomicBool::new(false));
                    state
                        .running
                        .lock()
                        .insert(claimed.id.clone(), cancel.clone());
                    Some((child, job, claimed, cancel))
                }
                Err(_) => {
                    let _ = child.start_kill();
                    claimed.set(
                        "submission_unknown",
                        "无法保护恢复进程生命周期，已停止；不会重发",
                        now(),
                        30,
                    );
                    s.update(&claimed)?;
                    None
                }
            },
            Err(_) => {
                workflow::spawn_failed(
                    &s,
                    &mut claimed,
                    "CLI 进程未启动，稍后重新核验",
                    "CLI 连续启动失败，已停止自动恢复",
                    now(),
                    |_| 30,
                )?;
                None
            }
        }
    };
    if let Some((child, job, claimed, cancel)) = launch {
        tauri::async_runtime::spawn(run(app.clone(), child, job, claimed, cancel));
    }
    Ok(())
}
pub async fn tick(app: &tauri::AppHandle) -> Result<i64> {
    let (enabled, watches, records) = {
        let state = app.state::<State>();
        let _g = state.fence.lock();
        let s = db(app)?;
        s.expire_watches(now())?;
        (s.enabled()?, s.watches()?, s.records()?)
    };
    if !enabled && !records.iter().any(Recovery::in_flight) {
        return Ok(30);
    }
    let home = source::home()?;
    let key = source::key(&home);
    for r in records.iter().filter(|r| r.in_flight()) {
        if app.state::<State>().running.lock().contains_key(&r.id) {
            continue;
        }
        // After restart, a claim is never submitted again. Only transcript evidence can settle it.
        if let Ok(f) = source::frame(&home, &r.thread_id, Some(&r.marker)) {
            if f.recovered_round_completed(&r.marker) {
                end(app, r, "completed", "会话记录已确认本轮完成")?;
            } else {
                wait(
                    app,
                    r,
                    "submission_unknown",
                    "上次恢复结果待核实，不会自动重发；请查看原会话",
                    60,
                )?;
            }
        }
    }
    if !enabled {
        return Ok(10);
    }
    for w in watches
        .iter()
        .filter(|w| w.enabled && !w.consumed && w.home_key == key)
    {
        if let Ok(f) = source::frame(&home, &w.thread_id, None) {
            let state = app.state::<State>();
            let _g = state.fence.lock();
            let s = db(app)?;
            if s.enabled()?
                && s.watch(&w.home_key, &w.thread_id)?
                    .is_some_and(|w| w.enabled && !w.consumed)
            {
                observe(&s, w, &f, false)?;
            }
        }
    }
    if !app.state::<State>().running.lock().is_empty() {
        return Ok(5);
    }
    for r in workflow::due(&records, now()) {
        if r.home_key != key {
            wait(
                app,
                r,
                "paused_configuration",
                "Claude 数据目录已变化，请重新选择会话",
                60,
            )?;
            continue;
        }
        if let Err(e) = attempt(app, &home, r.clone()).await {
            wait(app, r, "waiting_history", &adapter::public_error(&e), 30)?;
        }
        break;
    }
    Ok(workflow::next_delay(&db(app)?.records()?, now(), false))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn selected_frame() -> (Watch, source::Frame) {
        let w = Watch {
            thread_id: uuid::Uuid::new_v4().to_string(),
            home_key: "fixture-home".into(),
            account: "fixture-account".into(),
            configuration: "fixture-config".into(),
            enabled: true,
            created_at: now(),
            ..Default::default()
        };
        let f = source::Frame {
            id: w.thread_id.clone(),
            title: "fixture".into(),
            project: "fixture".into(),
            cwd: "fixture".into(),
            configuration: w.configuration.clone(),
            model: "fixture-model".into(),
            anchor: uuid::Uuid::new_v4().to_string(),
            quota: true,
            quota_proven: false,
            event_at: now(),
            fresh: true,
            last_user: "fixture task".into(),
            marker_seen: false,
            completed: false,
        };
        (w, f)
    }
    #[test]
    fn selected_failure_is_persistent_once_and_generic_rate_limit_is_not_proof() {
        let root = crate::test_support::FixtureDir::new();
        let mut s = Store::open_for(root.path(), "claude").unwrap();
        let (w, mut f) = selected_frame();
        s.put_watch(&w).unwrap();
        f.event_at = w.created_at - 100;
        observe(&s, &w, &f, false).unwrap();
        assert!(s.records().unwrap().is_empty());
        observe(&s, &w, &f, true).unwrap();
        observe(&s, &w, &f, true).unwrap();
        let records = s.records().unwrap();
        assert_eq!(records.len(), 1);
        assert!(!records[0].quota_confirmed);
        assert!(unchanged(&records[0], &f));
        f.anchor = uuid::Uuid::new_v4().to_string();
        assert!(!unchanged(&records[0], &f));
        s.enabled_set(true, now()).unwrap();
        assert!(s.claim(&records[0].id, now()).unwrap().is_some());
        let mut restarted = Store::open_for(root.path(), "claude").unwrap();
        restarted.recover_crash(now()).unwrap();
        f.anchor = records[0].turn_id.clone();
        observe(&restarted, &w, &f, true).unwrap();
        assert_eq!(restarted.records().unwrap().len(), 1);
        assert!(restarted.claim(&records[0].id, now()).unwrap().is_none());
    }
    #[test]
    fn partial_history_completed_turn_and_changed_configuration_cannot_resume() {
        let root = crate::test_support::FixtureDir::new();
        let s = Store::open_for(root.path(), "claude").unwrap();
        let (w, mut f) = selected_frame();
        s.put_watch(&w).unwrap();
        f.fresh = false;
        observe(&s, &w, &f, true).unwrap();
        f.fresh = true;
        f.quota = false;
        f.completed = true;
        observe(&s, &w, &f, true).unwrap();
        assert!(s.records().unwrap().is_empty());
        f.quota = true;
        f.completed = false;
        f.quota_proven = true;
        observe(&s, &w, &f, true).unwrap();
        let r = s.records().unwrap().remove(0);
        assert!(r.quota_confirmed && unchanged(&r, &f));
        f.configuration = "changed-project-or-model".into();
        assert!(!unchanged(&r, &f));
    }
    #[test]
    fn cancelled_and_reselected_failure_rejects_previous_async_results() {
        let root = crate::test_support::FixtureDir::new();
        let s = Store::open_for(root.path(), "claude").unwrap();
        let (w, f) = selected_frame();
        s.put_watch(&w).unwrap();
        observe(&s, &w, &f, true).unwrap();
        let mut previous = s.records().unwrap().remove(0);
        previous.quota_confirmed = true;
        previous.reset_at = Some(now() + 100);
        s.update(&previous).unwrap();
        s.cancel(&w.home_key, &w.thread_id, now()).unwrap();
        assert!(!may_update(
            &s.record(&previous.id).unwrap().unwrap(),
            &previous
        ));
        s.put_watch(&w).unwrap();
        observe(&s, &w, &f, true).unwrap();
        let current = s.record(&previous.id).unwrap().unwrap();
        assert_ne!(previous.marker, current.marker);
        assert!(!current.quota_confirmed);
        assert!(current.reset_at.is_none());
        assert!(!may_update(&current, &previous));
        assert!(may_update(&current, &current));
    }
}
