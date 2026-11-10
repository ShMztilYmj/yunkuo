mod adapter;
mod protocol;
mod scheduler;
mod source;
use super::{model::*, store::Store};
use anyhow::{Result, bail};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
};
use tauri::{Emitter, Manager};

#[derive(Default)]
pub struct State {
    fence: parking_lot::Mutex<()>,
    compatibility: parking_lot::Mutex<Compatibility>,
    probe: tokio::sync::Mutex<()>,
    running: parking_lot::Mutex<HashMap<String, Arc<AtomicBool>>>,
    wake: tokio::sync::Notify,
    stop: AtomicBool,
    cooldown: AtomicI64,
}
/// 数据模型、存储与时间戳由上层 `resume` 提供，子模块继续按 `super::now` 访问。
pub(crate) use super::now;
/// AI 概览只借用一次官方用量响应；登录校验、响应上限与账号变化检查仍由 adapter 统一处理。
pub(crate) async fn usage_value() -> Result<serde_json::Value> {
    let home = source::home()?;
    adapter::usage_value(&home).await.map(|(_, value)| value)
}
fn db(app: &tauri::AppHandle) -> Result<Store> {
    Store::open_for(&app.path().app_data_dir()?, "claude")
}
fn allowed(w: &tauri::WebviewWindow) -> Result<(), String> {
    if w.label() == "main" {
        Ok(())
    } else {
        Err("Claude 续跑仅允许主窗口管理".into())
    }
}
fn error(e: anyhow::Error) -> String {
    adapter::public_error(&e)
}
fn changed(app: &tauri::AppHandle) {
    let _ = app.emit("claude-resume-changed", ());
}
fn snapshot(app: &tauri::AppHandle) -> Result<Status> {
    let state = app.state::<State>();
    let _g = state.fence.lock();
    let s = db(app)?;
    Ok(Status {
        enabled: s.enabled()?,
        compatibility: state.compatibility.lock().clone(),
        watches: s.watches()?.into_iter().map(Into::into).collect(),
        recoveries: s.records()?.into_iter().map(Into::into).collect(),
        events: s.events()?,
    })
}
#[tauri::command]
pub async fn claude_resume_status(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Status, String> {
    allowed(&window)?;
    snapshot(&app).map_err(error)
}
#[tauri::command]
pub async fn claude_resume_candidates(
    window: tauri::WebviewWindow,
) -> Result<Vec<Candidate>, String> {
    allowed(&window)?;
    tokio::task::spawn_blocking(|| source::candidates(&source::home()?))
        .await
        .map_err(|_| "会话读取未完成".to_owned())?
        .map_err(error)
}
#[tauri::command]
pub async fn claude_resume_check(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Status, String> {
    allowed(&window)?;
    let state = app.state::<State>();
    let _g = state
        .probe
        .try_lock()
        .map_err(|_| "兼容检查正在进行".to_owned())?;
    if now() < state.cooldown.load(Ordering::SeqCst) {
        return Err("额度查询正在冷却，请稍后重试".into());
    }
    let result = match source::home() {
        Ok(h) => adapter::probe(&h).await,
        Err(e) => Err(e),
    };
    let value = match result {
        Ok(version) => Compatibility {
            supported: true,
            engine_version: Some(version),
            message: "CLI 恢复接口与订阅额度可用；发送前仍会核验会话占用".into(),
            checked_at: Some(now()),
        },
        Err(e) => {
            if let Some(c) = e.downcast_ref::<adapter::Cooldown>() {
                state.cooldown.store(now() + c.0 as i64, Ordering::SeqCst);
            }
            Compatibility {
                message: error(e),
                checked_at: Some(now()),
                ..Default::default()
            }
        }
    };
    *state.compatibility.lock() = value;
    state.wake.notify_one();
    changed(&app);
    snapshot(&app).map_err(error)
}
#[tauri::command]
pub async fn claude_resume_enable(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    enabled: bool,
) -> Result<Status, String> {
    allowed(&window)?;
    {
        let state = app.state::<State>();
        let _g = state.fence.lock();
        if enabled && !state.compatibility.lock().supported {
            return Err("请先完成兼容检查".into());
        }
        db(&app)
            .and_then(|s| s.enabled_set(enabled, now()))
            .map_err(error)?;
        if !enabled {
            for flag in state.running.lock().values() {
                flag.store(true, Ordering::SeqCst);
            }
        }
        state.wake.notify_one();
    }
    changed(&app);
    snapshot(&app).map_err(error)
}
#[tauri::command]
pub async fn claude_resume_watch(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    thread_id: String,
    enabled: bool,
) -> Result<Status, String> {
    allowed(&window)?;
    if !enabled {
        return claude_resume_cancel(app, window, thread_id).await;
    }
    (|| -> Result<()> {
        let home = source::home()?;
        let state = app.state::<State>();
        let _g = state.fence.lock();
        let s = db(&app)?;
        if !s.enabled()? || !state.compatibility.lock().supported {
            bail!("请先通过检查并开启 Claude 自动续跑");
        }
        if s.records()?
            .iter()
            .any(|r| r.thread_id == thread_id && r.in_flight())
        {
            bail!("该会话已有正在执行或待核实的恢复");
        }
        let f = source::frame(&home, &thread_id, None)?;
        let data = app.path().app_data_dir()?.canonicalize()?;
        if f.cwd.starts_with(data) {
            bail!("软件自己的资料目录不支持此恢复入口");
        }
        let login = source::login(&home)?;
        let w = Watch {
            thread_id: f.id.clone(),
            home_key: source::key(&home),
            title: f.title.clone(),
            project: f.project.clone(),
            account: login.key,
            configuration: f.configuration.clone(),
            enabled: true,
            consumed: false,
            created_at: now(),
        };
        s.put_watch(&w)?;
        scheduler::observe(&s, &w, &f, true)?;
        state.wake.notify_one();
        Ok(())
    })()
    .map_err(error)?;
    changed(&app);
    snapshot(&app).map_err(error)
}
#[tauri::command]
pub async fn claude_resume_cancel(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    thread_id: String,
) -> Result<Status, String> {
    allowed(&window)?;
    {
        let state = app.state::<State>();
        let _g = state.fence.lock();
        let s = db(&app).map_err(error)?;
        s.cancel_thread(&thread_id, now()).map_err(error)?;
        for r in s.records().map_err(error)? {
            if r.thread_id == thread_id {
                if let Some(flag) = state.running.lock().get(&r.id) {
                    flag.store(true, Ordering::SeqCst);
                }
            }
        }
        state.wake.notify_one();
    }
    changed(&app);
    snapshot(&app).map_err(error)
}
#[tauri::command]
pub async fn claude_resume_forget(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    thread_id: String,
) -> Result<Status, String> {
    allowed(&window)?;
    {
        let state = app.state::<State>();
        let _g = state.fence.lock();
        let s = db(&app).map_err(error)?;
        s.forget_thread(&thread_id, now()).map_err(error)?;
    }
    changed(&app);
    snapshot(&app).map_err(error)
}
pub(crate) fn pause_for_identity_change(app: &tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<State>();
    let _g = state.fence.lock();
    db(app)
        .and_then(|s| s.enabled_set(false, now()))
        .map_err(error)?;
    for flag in state.running.lock().values() {
        flag.store(true, Ordering::SeqCst);
    }
    state.wake.notify_one();
    Ok(())
}
pub fn start(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = db(&app).and_then(|s| s.recover_crash(now())) {
            *app.state::<State>().compatibility.lock() = Compatibility {
                message: error(e),
                ..Default::default()
            };
            return;
        }
        loop {
            if app.state::<State>().stop.load(Ordering::SeqCst) {
                break;
            }
            let delay = match scheduler::tick(&app).await {
                Ok(t) => t,
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
            let state = app.state::<State>();
            tokio::select! {_=tokio::time::sleep(std::time::Duration::from_secs(delay.clamp(2,30)as u64))=>{},_=state.wake.notified()=>{}}
        }
    });
}
pub fn stop(app: &tauri::AppHandle) {
    let state = app.state::<State>();
    state.stop.store(true, Ordering::SeqCst);
    for flag in state.running.lock().values() {
        flag.store(true, Ordering::SeqCst);
    }
    state.wake.notify_one();
}
pub fn diagnose(output: std::path::PathBuf) {
    let value = tauri::async_runtime::block_on(async {
        match source::home() {
            Ok(h) => {
                serde_json::json!({"compatibility":adapter::report(&h).await,"candidateCount":source::candidates(&h).map(|v|v.len()).unwrap_or(0),"otherClaudeRunning":adapter::other_claude_running(),"messagesSent":0})
            }
            Err(e) => serde_json::json!({"error":error(e),"messagesSent":0}),
        }
    });
    let _ = std::fs::write(
        output,
        serde_json::to_vec_pretty(&value).unwrap_or_default(),
    );
}
