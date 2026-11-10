//! Tauri 适配与网络变更前的跨模块失效处理。
use crate::{
    config::{AppConfig, ConfigMutex},
    platform::state::{ConfigUpdateMutex, EnvironmentOperation, data_dir},
    proxy::{
        api,
        core::{self, CoreState},
        explain, overlay,
        profile::ProfileManager,
    },
};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Error {
    module: &'static str,
    code: &'static str,
    message: String,
    retryable: bool,
    operation_id: String,
}
fn error(value: anyhow::Error) -> Error {
    let operation_id = uuid::Uuid::new_v4().to_string();
    log::warn!("proxy operation failed; code=PROXY_OPERATION_FAILED operation_id={operation_id}");
    Error {
        module: "proxy",
        code: "PROXY_OPERATION_FAILED",
        message: format!("{value:#}"),
        retryable: false,
        operation_id,
    }
}
/// 输入不合法（域名、覆写 YAML、引用关系）不需要重试，也无需生成故障码。
fn invalid(message: String) -> Error {
    Error {
        module: "proxy",
        code: "PROXY_INPUT_INVALID",
        message,
        retryable: false,
        operation_id: uuid::Uuid::new_v4().to_string(),
    }
}
fn config(app: &AppHandle) -> AppConfig {
    app.state::<ConfigMutex>().lock().clone()
}

pub(crate) fn invalidate_network(app: &AppHandle) {
    app.state::<CoreState>()
        .network_revision
        .fetch_add(1, Ordering::SeqCst);
    crate::app::environment::invalidate(app);
}

#[tauri::command]
pub(crate) async fn proxy_nodes(app: AppHandle) -> Result<BTreeMap<String, api::Node>, Error> {
    api::nodes(&config(&app)).await.map_err(error)
}
#[tauri::command]
pub(crate) async fn proxy_rules(app: AppHandle) -> Result<Vec<api::Rule>, Error> {
    api::rules(&config(&app)).await.map_err(error)
}
#[tauri::command]
pub(crate) async fn proxy_connections(app: AppHandle) -> Result<api::Connections, Error> {
    api::connections(&config(&app)).await.map_err(error)
}

#[tauri::command]
pub(crate) async fn proxy_select_node(
    app: AppHandle,
    group: String,
    node: String,
) -> Result<(), Error> {
    // 只改运行中的内核，不碰持久化配置：串行化由 Engine 负责，
    // 不再占用配置事务锁，否则一次 TUN 应用会同时挡住切节点。
    let cfg = config(&app);
    crate::proxy::core::verify_proxy_ready(&app, &cfg)
        .await
        .map_err(error)?;
    invalidate_network(&app);
    // 选择只作用于运行中的内核：与 FlClash 一致，内核不记忆组选择，
    // 重启后各代理组回到订阅里的默认项。
    api::select(&cfg, &group, &node).await.map_err(error)
}

#[tauri::command]
pub(crate) async fn proxy_close_connections(
    app: AppHandle,
    id: Option<String>,
) -> Result<(), Error> {
    let cfg = config(&app);
    crate::proxy::core::verify_proxy_ready(&app, &cfg)
        .await
        .map_err(error)?;
    api::close(&cfg, id.as_deref()).await.map_err(error)
}

#[tauri::command]
pub(crate) async fn proxy_delay(
    app: AppHandle,
    node: String,
    group: bool,
    timeout: u64,
    url: String,
) -> Result<serde_json::Value, Error> {
    api::delay(&config(&app), &node, group, timeout, &url)
        .await
        .map_err(error)
}

/// 规则命中解释：只读两次控制器快照，不改路由、不改选择。
#[tauri::command]
pub(crate) async fn proxy_explain(
    app: AppHandle,
    host: String,
) -> Result<explain::Explanation, Error> {
    let cfg = config(&app);
    core::verify_proxy_ready(&app, &cfg).await.map_err(error)?;
    let host = explain::normalize_host(&host)
        .ok_or_else(|| invalid("请输入域名或 IP，例如 claude.ai 或 203.0.113.7".into()))?;
    let (rules, nodes) = tokio::join!(api::rules(&cfg), api::nodes(&cfg));
    let rules = rules.map_err(error)?;
    let nodes = nodes.map_err(error)?;
    Ok(explain::explain(&rules, &nodes, &host))
}

/// 读取当前订阅覆写；缺失时返回空文本，前端不用先创建文件。
#[tauri::command]
pub(crate) async fn proxy_overlay(app: AppHandle) -> Result<overlay::View, Error> {
    overlay::view(&data_dir(&app)).map_err(error)
}

/// 保存覆写并原地应用。先按当前方案试编译，语法、引用与严格保护
/// 的拒绝都发生在落盘之前；保存成功后走与订阅更新相同的应用链路。
#[tauri::command]
pub(crate) async fn proxy_overlay_save(
    app: AppHandle,
    text: String,
) -> Result<overlay::View, Error> {
    let environment = app.state::<EnvironmentOperation>();
    let _environment = environment.lock().await;
    let operation = app.state::<ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    let cfg = config(&app);
    let directory = data_dir(&app);
    let parsed = overlay::parse(&text).map_err(|e| invalid(format!("{e:#}")))?;
    let content = ProfileManager::new(&directory)
        .current_content()
        .map_err(error)?;
    core::generate_config_with_overlay(&cfg, content.as_deref(), &parsed)
        .map_err(|e| invalid(format!("{e:#}")))?;
    overlay::save(&directory, &text).map_err(error)?;
    if core::child_alive(&app) {
        core::apply_config(&app, &cfg).await.map_err(error)?;
    }
    overlay::view(&directory).map_err(error)
}

/// 清空覆写并应用；与保存共用一条校验与提交流程。
#[tauri::command]
pub(crate) async fn proxy_overlay_reset(app: AppHandle) -> Result<overlay::View, Error> {
    proxy_overlay_save(app, String::new()).await
}
