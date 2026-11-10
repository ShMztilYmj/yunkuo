//! 执行前检查契约，由资源消费者定义、应用层装配。
use crate::config::AppConfig;
use serde::Serialize;
use std::{future::Future, pin::Pin};
use tauri::{AppHandle, Manager};

#[derive(Serialize)]
pub struct LaunchResult {
    pub ok: bool,
    pub failures: Vec<String>,
}
pub(crate) type CheckFuture = Pin<Box<dyn Future<Output = (AppConfig, LaunchResult)> + Send>>;
pub(crate) struct State {
    check: fn(AppHandle, String) -> CheckFuture,
}
impl State {
    pub(crate) fn new(check: fn(AppHandle, String) -> CheckFuture) -> Self {
        Self { check }
    }
}
pub(crate) async fn check_launch_locked(app: &AppHandle, tool: &str) -> (AppConfig, LaunchResult) {
    let check = app.try_state::<State>().map(|state| state.check);
    match check {
        Some(check) => check(app.clone(), tool.to_owned()).await,
        None => (
            app.state::<crate::config::ConfigMutex>().lock().clone(),
            LaunchResult {
                ok: false,
                failures: vec!["执行前检查尚未装配，未启动工具".into()],
            },
        ),
    }
}
