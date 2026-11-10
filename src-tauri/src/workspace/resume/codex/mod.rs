//! Opt-in, exact-thread recovery. No message is submitted until both global and per-thread consent exist.
mod adapter;
mod native;
mod scheduler;
mod source;
/// 数据模型、存储与时间戳由上层 `resume` 提供，子模块继续按 `super::model` 访问。
pub(crate) use super::{model, now, store};

use anyhow::{Result, bail};
use model::*;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, Manager};

#[derive(Default)]
pub struct State {
    // All cancellations/settings changes and the final spawn share this short fence; never held across await.
    fence: parking_lot::Mutex<()>,
    compatibility: parking_lot::Mutex<Compatibility>,
    probe: tokio::sync::Mutex<()>,
    wake: tokio::sync::Notify,
    stop: AtomicBool,
}
fn database(app: &tauri::AppHandle) -> Result<store::Store> {
    store::Store::open(&app.path().app_data_dir()?)
}
fn allowed(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("自动续跑仅允许主窗口管理".into())
    }
}
fn error(e: anyhow::Error) -> String {
    adapter::public_error(&e)
}
fn changed(app: &tauri::AppHandle) {
    let _ = app.emit("codex-resume-changed", ());
}
fn snapshot(app: &tauri::AppHandle) -> Result<Status> {
    let runtime = app.state::<State>();
    let _guard = runtime.fence.lock();
    let db = database(app)?;
    Ok(Status {
        enabled: db.enabled()?,
        compatibility: runtime.compatibility.lock().clone(),
        watches: db.watches()?.into_iter().map(Into::into).collect(),
        recoveries: db.records()?.into_iter().map(Into::into).collect(),
        events: db.events()?,
    })
}
#[tauri::command]
pub async fn codex_resume_status(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Status, String> {
    allowed(&window)?;
    snapshot(&app).map_err(error)
}
#[tauri::command]
pub async fn codex_resume_candidates(
    window: tauri::WebviewWindow,
) -> Result<Vec<Candidate>, String> {
    allowed(&window)?;
    tokio::task::spawn_blocking(|| source::candidates(&source::home()?))
        .await
        .map_err(|_| "读取会话列表未完成".to_string())?
        .map_err(error)
}
#[tauri::command]
pub async fn codex_resume_check(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Status, String> {
    allowed(&window)?;
    let runtime = app.state::<State>();
    let _probe = runtime
        .probe
        .try_lock()
        .map_err(|_| "兼容检查正在进行，请稍候".to_string())?;
    let report = match source::home() {
        Ok(home) => adapter::report(&home).await,
        Err(e) => Compatibility {
            message: error(e),
            checked_at: Some(now()),
            ..Default::default()
        },
    };
    *runtime.compatibility.lock() = report;
    // A recheck wakes the scheduler but does not bypass a reset or authorize a message.
    runtime.wake.notify_one();
    changed(&app);
    snapshot(&app).map_err(error)
}
#[tauri::command]
pub async fn codex_resume_enable(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    enabled: bool,
) -> Result<Status, String> {
    allowed(&window)?;
    {
        let runtime = app.state::<State>();
        let _guard = runtime.fence.lock();
        if enabled && !runtime.compatibility.lock().supported {
            return Err("请先完成兼容检查，再开启自动续跑".into());
        }
        database(&app)
            .and_then(|db| db.enabled_set(enabled, now()))
            .map_err(error)?;
        runtime.wake.notify_one();
    }
    changed(&app);
    snapshot(&app).map_err(error)
}
#[tauri::command]
pub async fn codex_resume_watch(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    thread_id: String,
    enabled: bool,
) -> Result<Status, String> {
    allowed(&window)?;
    let result = (|| -> Result<()> {
        let runtime = app.state::<State>();
        let _guard = runtime.fence.lock();
        let db = database(&app)?;
        if !enabled {
            // 关闭守候要按已存 home_key 逐个取消：CODEX_HOME 变化后仍须生效。
            db.cancel_thread(&thread_id, now())?;
            runtime.wake.notify_one();
            return Ok(());
        }
        let home = source::home()?;
        let key = source::home_key(&home);
        if !db.enabled()? || !runtime.compatibility.lock().supported {
            bail!("请先通过兼容检查并开启总开关");
        }
        let account = source::account(&home)?;
        let f = source::frame(&home, &thread_id)?;
        if db
            .records()?
            .iter()
            .any(|r| r.home_key == key && r.thread_id == thread_id && r.in_flight())
        {
            bail!("该会话已有提交或运行中的恢复，请先核实结果");
        }
        let watch = Watch {
            thread_id: f.id.clone(),
            home_key: key,
            title: f.title.clone(),
            project: f.project.clone(),
            account: account.key,
            configuration: f.configuration.clone(),
            enabled: true,
            consumed: false,
            created_at: now(),
        };
        db.put_watch(&watch)?;
        scheduler::observe(&db, &watch, &f, now(), true)?;
        runtime.wake.notify_one();
        Ok(())
    })();
    result.map_err(error)?;
    changed(&app);
    snapshot(&app).map_err(error)
}
#[tauri::command]
pub async fn codex_resume_cancel(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    thread_id: String,
) -> Result<Status, String> {
    allowed(&window)?;
    {
        let runtime = app.state::<State>();
        let _guard = runtime.fence.lock();
        let db = database(&app).map_err(error)?;
        // Match stored home keys too: cancellation must still work after CODEX_HOME changes or disappears.
        db.cancel_thread(&thread_id, now()).map_err(error)?;
        runtime.wake.notify_one();
    }
    changed(&app);
    snapshot(&app).map_err(error)
}
#[tauri::command]
pub async fn codex_resume_forget(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    thread_id: String,
) -> Result<Status, String> {
    allowed(&window)?;
    {
        let runtime = app.state::<State>();
        let _guard = runtime.fence.lock();
        let db = database(&app).map_err(error)?;
        db.forget_thread(&thread_id, now()).map_err(error)?;
    }
    changed(&app);
    snapshot(&app).map_err(error)
}
pub(crate) fn pause_for_account_change(app: &tauri::AppHandle) -> Result<(), String> {
    let runtime = app.state::<State>();
    let _guard = runtime.fence.lock();
    let db = database(app).map_err(error)?;
    db.enabled_set(false, now()).map_err(error)?;
    runtime.wake.notify_one();
    Ok(())
}
pub fn start(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let startup = database(&app).and_then(|db| db.recover_crash(now()));
        if let Err(e) = startup {
            *app.state::<State>().compatibility.lock() = Compatibility {
                message: error(e),
                ..Default::default()
            };
            return;
        }
        let mut engine = None;
        loop {
            if app.state::<State>().stop.load(Ordering::SeqCst) {
                break;
            }
            let delay = match scheduler::tick(&app, &mut engine).await {
                Ok(delay) => delay,
                Err(e) => {
                    *app.state::<State>().compatibility.lock() = Compatibility {
                        message: error(e),
                        checked_at: Some(now()),
                        ..Default::default()
                    };
                    30
                }
            };
            changed(&app);
            let runtime = app.state::<State>();
            tokio::select! {_=tokio::time::sleep(std::time::Duration::from_secs(delay.clamp(1,30) as u64))=>{},_=runtime.wake.notified()=>{}}
        }
    });
}
pub fn stop(app: &tauri::AppHandle) {
    let state = app.state::<State>();
    let _guard = state.fence.lock();
    state.stop.store(true, Ordering::SeqCst);
    state.wake.notify_one();
}

/// Read-only native diagnostic; contains no thread identifiers, paths, account labels or message content.
pub fn diagnose(output: std::path::PathBuf) {
    let result = tauri::async_runtime::block_on(async {
        let home = source::home()?;
        let compatibility = adapter::report(&home).await;
        let candidates = source::candidates(&home)?;
        let desktop = native::desktop(&home)?;
        let loaded = candidates
            .iter()
            .filter(|c| native::loaded(&home, &c.thread_id, &desktop).is_ok())
            .count();
        Ok::<_, anyhow::Error>(
            serde_json::json!({"compatibility":compatibility,"candidateCount":candidates.len(),"loadedCount":loaded,"messagesSent":0}),
        )
    });
    let value = match result {
        Ok(v) => v,
        Err(e) => serde_json::json!({"error":error(e),"messagesSent":0}),
    };
    let _ = std::fs::write(
        output,
        serde_json::to_vec_pretty(&value).unwrap_or_default(),
    );
}
