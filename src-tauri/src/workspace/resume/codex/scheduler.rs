use super::{State, adapter, database, model::*, native, now, source, store::Store};
use crate::workspace::resume::{runner, workflow};
use anyhow::Result;
use std::{path::Path, sync::atomic::Ordering};
use tauri::Manager;
use tauri_plugin_notification::NotificationExt;

pub fn observe(db: &Store, w: &Watch, f: &source::Frame, time: i64, rearm: bool) -> Result<()> {
    // 与 claude 对齐：投影落后于 rollout 文件时 latest 可能是旧回合，不能据此建守候。
    if !f.fresh {
        return Ok(());
    }
    let Some(t) = f
        .latest
        .as_ref()
        .filter(|t| t.quota && t.completed.is_some_and(|n| n >= w.created_at - 2 || rearm))
    else {
        return Ok(());
    };
    runner::observe(
        db,
        w,
        time,
        rearm,
        runner::Observation {
            id: digest(format!("{}\n{}\n{}\n{}", w.home_key, f.id, t.id, t.ordinal)),
            thread_id: &f.id,
            turn_id: t.id.clone(),
            ordinal: t.ordinal,
            title: &f.title,
            project: &f.project,
            reason: "额度耗尽，等待核验重置时间",
            marker: format!("[云阔续跑:{}]", uuid::Uuid::new_v4()),
            rearm_reason: "已重新守候，等待实时额度核验",
            quota_confirmed: false,
            reset_quota: false,
        },
    )
}
fn update(app: &tauri::AppHandle, r: &Recovery) -> Result<()> {
    let state = app.state::<State>();
    let _guard = state.fence.lock();
    let db = database(app)?;
    if let Some(current) = db.record(&r.id)? {
        if current.claimed_at.is_some() && r.claimed_at.is_none() {
            return Ok(());
        }
        if current.cancel_requested && !r.cancel_requested && r.claimed_at.is_none() {
            return Ok(());
        }
        let mut result = r.clone();
        result.cancel_requested |= current.cancel_requested;
        db.update(&result)?;
    }
    Ok(())
}
fn pending(
    app: &tauri::AppHandle,
    r: &Recovery,
    state: &str,
    reason: &str,
    delay: i64,
) -> Result<()> {
    update(app, &r.advance(state, reason, delay))
}
fn stable(r: &Recovery, f: &source::Frame) -> bool {
    f.fresh
        && !f.queued
        && f.configuration == r.configuration
        && f.latest
            .as_ref()
            .is_some_and(|t| t.id == r.turn_id && t.ordinal == r.ordinal && t.quota)
}
fn consume(app: &tauri::AppHandle, r: &Recovery) -> Result<()> {
    let state = app.state::<State>();
    let _guard = state.fence.lock();
    workflow::consume_watch(&database(app)?, r)
}
fn notice(app: &tauri::AppHandle, reason: &str) {
    let _ = app
        .notification()
        .builder()
        .title("云阔 · Codex 自动续跑")
        .body(reason)
        .show();
}

async fn settle(app: &tauri::AppHandle, home: &Path, mut r: Recovery, enabled: bool) -> Result<()> {
    let receipt = source::receipt(home, &r.thread_id, r.ordinal, &r.marker)?;
    let cancel = r.cancel_requested || !enabled;
    match receipt {
        source::Receipt::Running => {
            if r.state != "running" {
                notice(app, "已确认指定会话开始续跑");
            }
            r.set(
                "running",
                if cancel {
                    "继续消息已开始运行，停止请在 Codex 中操作"
                } else {
                    "已确认原会话开始续跑"
                },
                now(),
                5,
            );
        }
        source::Receipt::Completed => {
            if r.state != "completed" {
                notice(app, "续跑会话已完成本轮");
            }
            r.set("completed", "本轮已完成", now(), 0);
        }
        source::Receipt::Stopped => r.set(
            "stopped",
            "续跑后本轮已停止；需要再次恢复时请重新选择会话",
            now(),
            0,
        ),
        source::Receipt::Queued(id) => {
            r.queue_id = Some(id.clone());
            if cancel {
                let d = match native::desktop(home) {
                    Ok(d) => d,
                    Err(_) => {
                        r.set(
                            "submission_unknown",
                            "可能仍在队列中，等待撤回核验；不会重发",
                            now(),
                            10,
                        );
                        return update(app, &r);
                    }
                };
                if adapter::delete(home, &d, &r.thread_id, &id).await {
                    r.set("cancelled", "已撤回本次继续消息", now(), 0);
                } else {
                    r.set(
                        "submission_unknown",
                        "撤回未获确认，可能仍会送达；不会重发",
                        now(),
                        10,
                    );
                }
            } else {
                let unchanged = source::frame(home, &r.thread_id).is_ok_and(|f| {
                    f.latest.as_ref().is_some_and(|t| t.id == r.turn_id)
                        && f.configuration == r.configuration
                }) && source::account(home).is_ok_and(|a| a.key == r.account);
                if !unchanged {
                    r.cancel_requested = true;
                    r.set(
                        "submission_unknown",
                        "会话或账号已变化，正在撤回已排队消息",
                        now(),
                        1,
                    );
                } else {
                    r.set("queued", "继续消息已排队，等待 Codex 接收", now(), 2);
                }
            }
        }
        source::Receipt::Foreign => r.set(
            "superseded",
            "用户或其他执行器已接手，不再自动继续",
            now(),
            0,
        ),
        source::Receipt::Unknown | source::Receipt::None => {
            if r.state == "running" {
                r.set(
                    "submission_unknown",
                    "会话记录暂不可核实，不会重新提交",
                    now(),
                    10,
                );
            } else if now() - r.claimed_at.unwrap_or(now()) > 45 {
                r.set(
                    "submission_unknown",
                    "送达情况尚未核实，可能已提交；不会重发",
                    now(),
                    10,
                );
            } else {
                r.next_check = now() + 2;
            }
        }
    }
    update(app, &r)
}

async fn attempt(
    app: &tauri::AppHandle,
    home: &Path,
    engine: &native::Desktop,
    mut r: Recovery,
) -> Result<()> {
    let f = source::frame(home, &r.thread_id)?;
    if !f.fresh {
        return pending(app, &r, "waiting_history", "等待会话记录同步", 5);
    }
    if f.latest
        .as_ref()
        .is_none_or(|t| t.id != r.turn_id || !t.quota)
    {
        consume(app, &r)?;
        return pending(
            app,
            &r,
            "superseded",
            "原中断已变化，用户或其他任务已接手",
            0,
        );
    }
    if r.configuration != f.configuration {
        return pending(
            app,
            &r,
            "paused_configuration",
            "模型或运行配置已变化，请取消后重新选择此会话",
            60,
        );
    }
    if f.queued {
        return pending(
            app,
            &r,
            "waiting_thread",
            "此会话已有用户消息排队，等待用户接手",
            5,
        );
    }
    let a = source::account(home)?;
    if a.key != r.account {
        return pending(
            app,
            &r,
            "paused_identity",
            "账号已变化，请取消后重新选择此会话",
            60,
        );
    }
    if let Err(e) = native::loaded(home, &r.thread_id, engine) {
        return pending(app, &r, "waiting_thread", &adapter::public_error(&e), 5);
    }
    let (checked, q) = match adapter::usage(home, engine, &a).await {
        Ok(v) => v,
        Err(_) => {
            return pending(
                app,
                &r,
                "waiting_quota",
                "实时额度查询失败，未发送继续消息",
                30,
            );
        }
    };
    r.checked_at = Some(checked);
    r.reset_at = q.reset_at;
    if q.available != Some(true) {
        let delay = q
            .reset_at
            .map(|t| (t + 2 - now()).clamp(2, 60))
            .unwrap_or(30);
        r.set(
            if q.available == Some(false) {
                "waiting_reset"
            } else {
                "waiting_quota"
            },
            if q.available == Some(false) {
                "额度尚未恢复，等待全部阻塞窗口解除"
            } else {
                "实时额度未知，等待重新核验"
            },
            now(),
            delay,
        );
        return update(app, &r);
    }
    r.reset_at = None;
    r.next_check = now();
    update(app, &r)?;
    // Same lock order as launch_tool. Account/config mutations cannot interleave with the final check and spawn.
    let ai = app.state::<crate::ai::management::State>();
    let _ai = ai.operation.lock().await;
    let environment = app.state::<crate::platform::state::EnvironmentOperation>();
    let _environment = environment.lock().await;
    let config = app.state::<crate::platform::state::ConfigUpdateMutex>();
    let _config = config.lock().await;
    let (_, verdict) = crate::ai::execution_policy::check_launch_locked(app, "chatgpt").await;
    if !verdict.ok {
        return pending(
            app,
            &r,
            "waiting_environment",
            "环境或网络保护检查未通过，请在环境检测中处理",
            30,
        );
    }
    let child = {
        let runtime = app.state::<State>();
        let _fence = runtime.fence.lock();
        if runtime.stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        let mut db = database(app)?;
        let Some(current) = db.record(&r.id)? else {
            return Ok(());
        };
        if current.cancel_requested || !db.enabled()? {
            return Ok(());
        }
        let fresh = source::frame(home, &r.thread_id)?;
        if !stable(&current, &fresh) || source::account(home)?.key != current.account {
            return Ok(());
        }
        if now() - checked > 20 {
            return Ok(());
        }
        let current_engine = native::desktop(home)?;
        if current_engine != *engine {
            return Ok(());
        }
        native::loaded(home, &r.thread_id, engine)?;
        let Some(mut claimed) = db.claim(&r.id, now())? else {
            return Ok(());
        };
        let message = format!("{CONTINUATION}\n\n{}", claimed.marker);
        match adapter::spawn(home, engine, &claimed.thread_id, &message) {
            Ok(child) => {
                r = claimed;
                Some(child)
            }
            Err(_) => {
                // A process definitely never started. This is the only retryable submission failure.
                workflow::spawn_failed(
                    &db,
                    &mut claimed,
                    "消息进程未启动，稍后重新核验",
                    "消息进程连续启动失败，已停止自动尝试",
                    now(),
                    |failures| 10 * failures as i64,
                )?;
                None
            }
        }
    };
    drop(_config);
    drop(_environment);
    drop(_ai);
    if let Some(child) = child {
        match adapter::finish(child, &r.thread_id).await {
            Some(id) => {
                r.queue_id = Some(id);
                r.set("queued", "继续消息已提交，等待送达确认", now(), 1);
            }
            None => r.set(
                "submission_unknown",
                "提交结果不明，正在核实；不会重复发送",
                now(),
                2,
            ),
        }
        update(app, &r)?;
    }
    Ok(())
}

pub async fn tick(
    app: &tauri::AppHandle,
    cache: &mut Option<(native::Desktop, String)>,
) -> Result<i64> {
    let (enabled, watches, records) = {
        let runtime = app.state::<State>();
        let _guard = runtime.fence.lock();
        let db = database(app)?;
        db.expire_watches(now())?;
        (db.enabled()?, db.watches()?, db.records()?)
    };
    let has_wait = enabled && watches.iter().any(|w| w.enabled && !w.consumed);
    if !has_wait && !records.iter().any(Recovery::in_flight) {
        return Ok(30);
    }
    let home = source::home()?;
    let key = source::home_key(&home);
    for r in workflow::due_in_flight(&records, now()).filter(|r| r.home_key != key) {
        pending(
            app,
            r,
            "submission_unknown",
            "数据目录已变化，请在原 Codex 中核实已提交消息；不会重发",
            60,
        )?;
    }
    for r in workflow::due_in_flight(&records, now()).filter(|r| r.home_key == key) {
        if settle(app, &home, r.clone(), enabled).await.is_err() {
            pending(
                app,
                r,
                "submission_unknown",
                "会话送达记录暂不可读，核实前不会重发",
                10,
            )?;
        }
    }
    if !has_wait {
        return Ok(5);
    }
    let identity = source::account(&home);
    for w in watches
        .iter()
        .filter(|w| w.enabled && !w.consumed && w.home_key == key)
    {
        if let Ok(f) = source::frame(&home, &w.thread_id) {
            let state = app.state::<State>();
            let _guard = state.fence.lock();
            let db = database(app)?;
            // Re-read consent after any concurrent cancel; observations cannot resurrect an old plan.
            if db.enabled()?
                && db
                    .watch(&w.home_key, &w.thread_id)?
                    .is_some_and(|w| w.enabled && !w.consumed)
            {
                observe(&db, w, &f, now(), false)?;
            }
        }
    }
    let records = {
        let state = app.state::<State>();
        let _guard = state.fence.lock();
        database(app)?.records()?
    };
    for r in workflow::due(&records, now()) {
        if workflow::expired(r, now(), 7 * 86400) {
            consume(app, r)?;
            pending(
                app,
                r,
                "expired",
                "此恢复计划已超过 7 天，请重新选择会话",
                0,
            )?;
            continue;
        }
        if r.home_key != key {
            pending(
                app,
                r,
                "paused_configuration",
                "Codex 数据目录已变化，请取消后重新选择会话",
                60,
            )?;
            continue;
        }
        if identity.as_ref().is_err() || identity.as_ref().is_ok_and(|a| a.key != r.account) {
            pending(
                app,
                r,
                "paused_identity",
                "无法核验原账号，请取消后重新选择会话",
                60,
            )?;
            continue;
        }
        let engine = match native::desktop(&home) {
            Ok(d) => d,
            Err(e) => {
                pending(app, r, "waiting_thread", &adapter::public_error(&e), 10)?;
                continue;
            }
        };
        if cache.as_ref().is_none_or(|(old, _)| old != &engine) {
            match adapter::compatible(&home, &engine).await {
                Ok(version) => {
                    *app.state::<State>().compatibility.lock() = Compatibility {
                        supported: true,
                        engine_version: Some(version.clone()),
                        message: "本机接口可用，发送前会核验额度与会话".into(),
                        checked_at: Some(now()),
                    };
                    *cache = Some((engine.clone(), version));
                }
                Err(e) => {
                    pending(app, r, "waiting_thread", &adapter::public_error(&e), 60)?;
                    continue;
                }
            }
        }
        if let Err(e) = attempt(app, &home, &engine, r.clone()).await {
            pending(app, r, "waiting_history", &adapter::public_error(&e), 10)?;
        }
        // At most one due recovery per pass; all others must get their own live quota check.
        break;
    }
    let db = database(app)?;
    Ok(workflow::next_delay(&db.records()?, now(), true))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(db: &Store) -> (Watch, source::Frame) {
        let w = Watch {
            thread_id: uuid::Uuid::new_v4().to_string(),
            home_key: "home".into(),
            title: "fixture".into(),
            account: "a".into(),
            configuration: "c".into(),
            enabled: true,
            created_at: 100,
            ..Default::default()
        };
        db.put_watch(&w).unwrap();
        let f = source::Frame {
            id: w.thread_id.clone(),
            title: w.title.clone(),
            project: "demo".into(),
            configuration: "c".into(),
            latest: Some(source::Turn {
                id: uuid::Uuid::new_v4().to_string(),
                ordinal: 3,
                status: "failed".into(),
                quota: true,
                completed: Some(101),
            }),
            fresh: true,
            queued: false,
        };
        (w, f)
    }
    #[test]
    fn selection_baseline_and_explicit_existing_interruption() {
        let dir = crate::test_support::FixtureDir::new();
        let db = Store::open(dir.path()).unwrap();
        let (w, mut f) = fixture(&db);
        f.latest.as_mut().unwrap().completed = Some(50);
        observe(&db, &w, &f, 100, false).unwrap();
        assert!(db.records().unwrap().is_empty());
        observe(&db, &w, &f, 100, true).unwrap();
        observe(&db, &w, &f, 100, true).unwrap();
        assert_eq!(db.records().unwrap().len(), 1);
    }
    #[test]
    fn final_eligibility_rejects_user_takeover_old_projection_and_new_config() {
        let dir = crate::test_support::FixtureDir::new();
        let db = Store::open(dir.path()).unwrap();
        let (w, mut f) = fixture(&db);
        observe(&db, &w, &f, 101, false).unwrap();
        let r = db.records().unwrap().remove(0);
        assert!(stable(&r, &f));
        f.fresh = false;
        assert!(!stable(&r, &f));
        f.fresh = true;
        f.queued = true;
        assert!(!stable(&r, &f));
        f.queued = false;
        f.configuration = "changed".into();
        assert!(!stable(&r, &f));
        f.configuration = "c".into();
        f.latest.as_mut().unwrap().id = uuid::Uuid::new_v4().to_string();
        assert!(!stable(&r, &f));
    }
    #[test]
    fn unknown_cancelled_and_completed_turns_never_register() {
        let dir = crate::test_support::FixtureDir::new();
        let db = Store::open(dir.path()).unwrap();
        let (w, mut f) = fixture(&db);
        for status in ["completed", "interrupted", "inProgress", "failed"] {
            f.latest.as_mut().unwrap().status = status.into();
            f.latest.as_mut().unwrap().quota = false;
            observe(&db, &w, &f, 101, true).unwrap();
        }
        assert!(db.records().unwrap().is_empty());
    }
}
