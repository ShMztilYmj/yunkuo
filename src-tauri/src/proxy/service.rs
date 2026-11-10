//! Windows 服务客户端。注册、认证、版本兼容和内核运行分别查询。
use crate::config::AppConfig;
use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use serde_json::Value;
use std::{path::PathBuf, time::Duration};

const BASE: &str = "http://127.0.0.1:33215";
const ROOT: &str = r"C:\ProgramData\Yunkuo";

fn token() -> Result<String> {
    let value = std::fs::read_to_string(PathBuf::from(ROOT).join("service_token.txt"))
        .context("安全服务令牌不可读，请安装或修复 TUN 服务")?;
    let value = value.trim().to_owned();
    if value.is_empty() {
        bail!("安全服务令牌为空，请修复 TUN 服务");
    }
    Ok(value)
}

fn client() -> Result<reqwest::Client> {
    static CLIENT: once_cell::sync::OnceCell<reqwest::Client> = once_cell::sync::OnceCell::new();
    Ok(CLIENT
        .get_or_try_init(|| {
            reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
        })?
        .clone())
}

pub(super) fn process_alive(pid: u32) -> Result<bool> {
    use windows::Win32::{
        Foundation::{CloseHandle, ERROR_INVALID_PARAMETER},
        System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    let handle = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        Ok(handle) => handle,
        Err(error)
            if error.code() == windows::core::HRESULT::from_win32(ERROR_INVALID_PARAMETER.0) =>
        {
            return Ok(false);
        }
        Err(error) => return Err(error.into()),
    };
    let mut code = 0;
    let result = unsafe { GetExitCodeProcess(handle, &mut code) };
    let _ = unsafe { CloseHandle(handle) };
    result?;
    Ok(code == 259)
}

pub fn registered() -> bool {
    use windows::{Win32::System::Registry::*, core::w};
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            w!("SYSTEM\\CurrentControlSet\\Services\\YunkuoService"),
            None,
            KEY_READ,
            &mut key,
        )
        .is_ok()
        {
            let _ = RegCloseKey(key);
            true
        } else {
            false
        }
    }
}

async fn request(
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
    timeout: Duration,
) -> Result<Value> {
    request_with_session(method, path, body, timeout, None).await
}
async fn request_with_session(
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
    timeout: Duration,
    request_id: Option<&str>,
) -> Result<Value> {
    let mut request = client()?
        .request(method, format!("{BASE}{path}"))
        .timeout(timeout)
        .header("X-Token", token()?);
    if let Some(id) = request_id {
        request = request.header("X-Runtime-Request-Id", id);
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .context("无法连接 TUN 服务，请在设置中安装或修复服务")?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        bail!("TUN 服务认证失败，请安装或修复服务");
    }
    let value: Value = response.json().await.context("TUN 服务响应格式不正确")?;
    if !status.is_success() {
        let detail = value["error"].as_str().unwrap_or("服务未提供原因");
        bail!(
            "TUN 服务请求失败（{status}）：{}",
            detail.chars().take(2048).collect::<String>()
        );
    }
    Ok(value)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    pub installed: bool,
    pub running: bool,
    pub compatible: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

pub async fn health() -> Health {
    let installed = registered();
    let result = request(
        reqwest::Method::GET,
        "/ping",
        None,
        Duration::from_millis(1500),
    )
    .await;
    match result {
        Ok(value) => {
            let compatible = value["version"].as_str()
                == Some(crate::service_protocol::SERVICE_VERSION)
                && value["engine"].as_str() == Some("mihomo")
                && value["sessionOwnership"].as_bool() == Some(true);
            Health {
                installed,
                running: true,
                compatible,
                version: value["version"].as_str().map(str::to_owned),
                error: (!compatible).then(|| "TUN 服务版本不兼容，请安装或修复新版服务".into()),
            }
        }
        Err(error) => Health {
            installed,
            running: false,
            compatible: false,
            version: None,
            error: Some(format!("{error:#}")),
        },
    }
}

pub async fn require_ready() -> Result<()> {
    let health = health().await;
    if health.installed && health.running && health.compatible {
        Ok(())
    } else {
        Err(anyhow!(
            health.error.unwrap_or_else(|| "TUN 服务未就绪".into())
        ))
    }
}
pub async fn service_ping() -> Option<String> {
    health().await.version
}
pub async fn service_embedded_available() -> bool {
    let value = health().await;
    value.installed && value.running && value.compatible
}
pub async fn secure_service_available() -> bool {
    service_embedded_available().await
}

struct RuntimeStatus {
    running: bool,
    pid: u32,
    session_id: Option<String>,
}
async fn runtime_status() -> Result<RuntimeStatus> {
    let value = request(
        reqwest::Method::GET,
        "/status",
        None,
        Duration::from_millis(1500),
    )
    .await?;
    let running = value["running"].as_bool().context("服务未返回内核状态")?;
    let pid = value["pid"]
        .as_u64()
        .context("服务未返回内核 PID")?
        .try_into()
        .context("服务内核 PID 超出范围")?;
    if running && (pid == 0 || value["engine"].as_str() != Some("mihomo")) {
        bail!("服务内核身份不正确");
    }
    Ok(RuntimeStatus {
        running,
        pid,
        session_id: value["sessionId"].as_str().map(str::to_owned),
    })
}
pub async fn status() -> Result<(bool, u32)> {
    let value = runtime_status().await?;
    Ok((value.running, value.pid))
}
pub async fn service_query_status() -> (bool, bool, u32) {
    status()
        .await
        .map(|(running, pid)| (true, running, pid))
        .unwrap_or((false, false, 0))
}

/// 请求 ID 在发送 /start 之前就归会话持有；响应丢失也可以仅清理这次启动。
pub(super) struct ServiceRuntime {
    request_id: Option<String>,
    pid: Option<u32>,
    active: bool,
}
impl ServiceRuntime {
    pub fn new() -> Self {
        Self {
            request_id: Some(uuid::Uuid::new_v4().to_string()),
            pid: None,
            active: false,
        }
    }
    pub fn known_pid(&self) -> Option<u32> {
        self.pid
    }
    pub fn is_running(&self) -> bool {
        // 只有拿到 PID 才算“已启动”。请求已发出但失败时 pid 为空，
        // 此时 `active` 只表示“仍持有请求标识用于清理”，不能当成内核在跑。
        self.pid
            .is_some_and(|pid| self.active && process_alive(pid).unwrap_or(true))
    }
    pub async fn pid(&self) -> Result<u32> {
        if !self.active {
            bail!("服务会话未启动");
        }
        let status = runtime_status().await?;
        if !status.running || !self.matches(&status) {
            bail!("TUN 服务中的会话已停止或被替换");
        }
        Ok(status.pid)
    }
    fn matches(&self, status: &RuntimeStatus) -> bool {
        self.pid.is_none_or(|pid| pid == status.pid)
            && self
                .request_id
                .as_deref()
                .is_none_or(|id| status.session_id.as_deref() == Some(id))
    }
    pub async fn start(&mut self, cfg: &AppConfig, yaml: &str) -> Result<()> {
        require_ready().await?;
        let config = crate::service_protocol::sanitized_config(yaml)?;
        let execution_policy = match super::policy::Plan::resolve(cfg)?.execution_policy {
            proxy_runtime::ExecutionPolicy::Standard => {
                crate::service_protocol::RuntimePolicy::Standard
            }
            proxy_runtime::ExecutionPolicy::Protected => {
                crate::service_protocol::RuntimePolicy::Protected
            }
        };
        let body = serde_json::to_value(crate::service_protocol::StartRequest {
            config,
            execution_policy,
        })?;
        // 从这一刻起，错误路径必须保留 request_id，不能丢弃可能已启动的资源。
        self.active = true;
        let response = request_with_session(
            reqwest::Method::POST,
            "/start",
            Some(body),
            Duration::from_secs(175),
            self.request_id.as_deref(),
        )
        .await;
        let value = match response {
            Ok(value) => value,
            Err(error) => {
                if let Ok(status) = runtime_status().await {
                    if status.running && self.matches(&status) {
                        self.pid = Some(status.pid);
                        return Ok(());
                    }
                }
                return Err(error);
            }
        };
        if value["engine"].as_str() != Some("mihomo")
            || value["sessionId"].as_str() != self.request_id.as_deref()
        {
            bail!("服务启动响应的会话身份不匹配");
        }
        let pid = value["pid"]
            .as_u64()
            .and_then(|pid| u32::try_from(pid).ok())
            .filter(|pid| *pid > 0)
            .context("服务启动响应缺少有效 PID")?;
        self.pid = Some(pid);
        Ok(())
    }
    pub async fn stop(&mut self) -> Result<()> {
        if !self.active {
            return Ok(());
        }
        let mut body = serde_json::Map::new();
        if let Some(id) = &self.request_id {
            body.insert("expectedSessionId".into(), id.clone().into());
        }
        if let Some(pid) = self.pid {
            body.insert("expectedPid".into(), pid.into());
        }
        let result = request(
            reqwest::Method::POST,
            "/stop",
            Some(Value::Object(body)),
            Duration::from_secs(30),
        )
        .await;
        if let Err(error) = result {
            // 已有另一个会话时绝不停止它；旧会话已不再属于服务，可结束旧所有权。
            match runtime_status().await {
                Ok(status) if !status.running || !self.matches(&status) => {}
                Err(_)
                    if self
                        .pid
                        .is_some_and(|pid| process_alive(pid).ok() == Some(false)) => {}
                _ => return Err(error),
            }
        }
        self.active = false;
        self.pid = None;
        Ok(())
    }
    pub async fn adopt(pid: u32) -> Result<Self> {
        require_ready().await?;
        let status = runtime_status().await?;
        if !status.running || status.pid != pid {
            bail!("服务会话已变化，未接管其他内核");
        }
        let session_id = status
            .session_id
            .context("服务未返回会话标识，请先退出旧版云阔再启动新版")?;
        uuid::Uuid::parse_str(&session_id).context("服务返回了无效的会话标识")?;
        Ok(Self {
            request_id: Some(session_id),
            pid: Some(pid),
            active: true,
        })
    }
}

pub async fn privacy_request(action: &str) -> Result<Value> {
    let version = service_ping().await.unwrap_or_default();
    if !crate::service_protocol::protection_action_supported(&version, action) {
        bail!("请先安装或修复云阔mihomo TUN 服务");
    }
    if !matches!(action, "arm" | "release") {
        bail!("未知保护操作");
    }
    request(
        reqwest::Method::POST,
        &format!("/privacy/{action}"),
        None,
        Duration::from_secs(30),
    )
    .await
}
pub async fn privacy_status() -> Result<Value> {
    request(
        reqwest::Method::GET,
        "/privacy/status",
        None,
        Duration::from_millis(1500),
    )
    .await
}
