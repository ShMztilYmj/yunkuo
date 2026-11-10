//! 认证的 mihomo 控制器通信；不持有进程、不推断会话归属。
use crate::config::AppConfig;
use anyhow::{Result, anyhow};
use serde_json::Value;
use std::time::Duration;
pub(super) fn quick_client() -> Result<reqwest::Client> {
    static CLIENT: once_cell::sync::OnceCell<reqwest::Client> = once_cell::sync::OnceCell::new();
    Ok(CLIENT
        .get_or_try_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_millis(1500))
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
        })?
        .clone())
}

fn build_client() -> Result<reqwest::Client> {
    static CLIENT: once_cell::sync::OnceCell<reqwest::Client> = once_cell::sync::OnceCell::new();
    Ok(CLIENT
        .get_or_try_init(|| {
            reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(15))
                .build()
        })?
        .clone())
}

pub fn build_client_with_proxy(port: u16) -> Result<reqwest::Client> {
    crate::http::via_loopback(port, Duration::from_secs(12))
}

fn auth_header(cfg: &AppConfig) -> String {
    format!("Bearer {}", cfg.secret)
}

/// 探测 external-controller 是否在线
pub async fn probe_controller(cfg: &AppConfig) -> Result<String> {
    let client = quick_client()?;
    let url = format!("http://{}/version", cfg.controller_addr());
    let resp: Value = client
        .get(&url)
        .header("Authorization", auth_header(cfg))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let version = resp
        .get("version")
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| anyhow!("控制器未返回有效版本信息"))?;
    if resp["meta"].as_bool() != Some(true) {
        return Err(anyhow!("控制器不是云阔mihomo，端口可能被其他程序占用"));
    }
    Ok(version.to_string())
}

/// 控制器监听成功不代表异步创建的 TUN 已就绪。
pub async fn wait_tun_ready(cfg: &AppConfig) -> Result<()> {
    wait_tun_ready_for(cfg, Duration::from_secs(8)).await
}

pub(super) async fn wait_tun_ready_for(cfg: &AppConfig, timeout: Duration) -> Result<()> {
    let client = quick_client()?;
    let ready = tokio::time::timeout(timeout, async {
        loop {
            let response = async {
                client
                    .get(format!("http://{}/configs", cfg.controller_addr()))
                    .header("Authorization", auth_header(cfg))
                    .send()
                    .await?
                    .error_for_status()?
                    .json::<Value>()
                    .await
            }
            .await;
            if response.is_ok_and(|value| {
                value["tun"]["enable"].as_bool() == Some(true)
                    && value["tun"]["device"].as_str() == Some(crate::privacy::TUN_NAME)
            }) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    })
    .await;
    ready.map_err(|_| anyhow!("{} 秒内未确认 TUN 实际启动。请检查其他代理软件的 TUN 及 DNS 端口 11053 占用；本次配置未提交", timeout.as_secs_f32()))
}

/// 概览页累计值；未知字段由 serde 跳过，不建立连接行对象。
#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionTotals {
    #[serde(default)]
    pub upload_total: u64,
    #[serde(default)]
    pub download_total: u64,
}

/// 只反序列化累计字段，跳过连接列表，避免每两秒构建整棵 JSON 对象树。
pub async fn connection_totals(cfg: &AppConfig) -> Result<ConnectionTotals> {
    Ok(build_client()?
        .get(format!("http://{}/connections", cfg.controller_addr()))
        .header("Authorization", auth_header(cfg))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

/// 通过控制器代理 REST 请求（供前端使用的通用通道）。
pub async fn controller_request(
    cfg: &AppConfig,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<Value> {
    let client = build_client()?;
    if !path.starts_with('/') || path.starts_with("//") {
        return Err(anyhow!("控制器请求必须是本机相对路径"));
    }
    let url = format!("http://{}{}", cfg.controller_addr(), path);
    let method = reqwest::Method::from_bytes(method.as_bytes())
        .map_err(|_| anyhow!("无效的 HTTP 方法: {method}"))?;
    let mut req = client
        .request(method, &url)
        .header("Authorization", auth_header(cfg));
    if let Some(b) = body {
        if !b.is_null() {
            req = req.json(&b);
        }
    }
    let resp = req.send().await?;
    let status = resp.status();
    let text = resp.text().await?;
    if status.is_success() {
        if text.is_empty() {
            Ok(Value::Null)
        } else {
            serde_json::from_str(&text).map_err(|e| anyhow!("响应解析失败: {e}"))
        }
    } else {
        Err(anyhow!("HTTP {status}: {text}"))
    }
}
