//! 内核宿主。本机会话与服务会话的差异只允许出现在本文件。
//! 业务层（engine）通过同一组方法使用宿主，不再出现 `service_managed` 分支。
use super::{controller, service};
use crate::config::AppConfig;
use anyhow::{Context, Result, bail};
use std::net::SocketAddr;
use std::path::Path;

pub(super) enum Host {
    /// 普通代理：内嵌运行时。`None` 表示已预占、尚未启动。
    Local(Option<proxy_runtime::Runtime>),
    /// TUN：系统服务托管。创建即有请求标识，用于清理“响应丢失但内核已起”。
    Service(service::ServiceRuntime),
}

impl Host {
    /// 启动前预占宿主。服务宿主的请求标识必须早于 `/start` 请求产生。
    /// `keep_service` 让已经建立的提权会话在关闭 TUN 后继续托管：TUN 只是
    /// 内核里的一个入站，没必要为了关它把整个会话搬回非提权进程。
    pub(super) fn reserve(cfg: &AppConfig, keep_service: bool) -> Self {
        if cfg.enable_tun || keep_service {
            Self::Service(service::ServiceRuntime::new())
        } else {
            Self::Local(None)
        }
    }

    pub(super) async fn start(
        &mut self,
        cfg: &AppConfig,
        yaml: &str,
        directory: &Path,
    ) -> Result<()> {
        match self {
            Self::Local(slot) => {
                let runtime = proxy_runtime::Runtime::start_with_policy(
                    proxy_runtime::StartOptions {
                        yaml: yaml.into(),
                        work_dir: directory.into(),
                    },
                    super::policy::Plan::resolve(cfg)?.execution_policy,
                )
                .await
                .context("云阔mihomo 启动失败")?;
                *slot = Some(runtime);
                Ok(())
            }
            Self::Service(runtime) => runtime.start(cfg, yaml).await,
        }
    }

    pub(super) fn service_managed(&self) -> bool {
        matches!(self, Self::Service(_))
    }

    /// 本机内核的退出通知。服务宿主没有本地句柄，由监督者用廉价存活判断兜底。
    pub(super) fn exit_signal(&self) -> Option<std::sync::Arc<tokio::sync::Notify>> {
        match self {
            Self::Local(Some(runtime)) => Some(runtime.exit_signal()),
            _ => None,
        }
    }

    /// 不产生 IO 的存活判断；只用于“能否复用/是否需要清理”的粗判。
    pub(super) fn alive(&self) -> bool {
        match self {
            Self::Local(Some(runtime)) => runtime.snapshot().phase == proxy_runtime::Phase::Ready,
            Self::Local(None) => false,
            // 通信失败不等于进程退出，未知时保留所有权。
            Self::Service(runtime) => runtime.is_running(),
        }
    }

    /// 当前已归属本宿主的 PID。启动中可能尚不可知。
    pub(super) fn known_pid(&self) -> Option<u32> {
        match self {
            Self::Local(Some(runtime)) => Some(runtime.snapshot().pid),
            Self::Local(None) => None,
            Self::Service(runtime) => runtime.known_pid(),
        }
    }

    /// 权威 PID：用于端口归属校验，取不到即视为不可用。
    pub(super) async fn pid(&self) -> Result<u32> {
        match self {
            Self::Local(Some(runtime)) => {
                let snapshot = runtime.snapshot();
                if snapshot.phase != proxy_runtime::Phase::Ready {
                    bail!("云阔mihomo 进程已停止");
                }
                Ok(snapshot.pid)
            }
            Self::Local(None) => bail!("云阔mihomo 尚未启动"),
            Self::Service(runtime) => runtime.pid().await,
        }
    }

    pub(super) async fn stop(&mut self) -> Result<()> {
        match self {
            Self::Local(slot) => {
                if let Some(runtime) = slot.as_mut() {
                    runtime.stop().await?;
                }
                *slot = None;
                Ok(())
            }
            Self::Service(runtime) => runtime.stop().await,
        }
    }

    pub(super) async fn adopt_service(pid: u32) -> Result<Self> {
        Ok(Self::Service(service::ServiceRuntime::adopt(pid).await?))
    }
}

/// 两个端口都必须由同一 PID 监听，避免接管无关进程的控制器。
pub(super) fn verify_owner(cfg: &AppConfig, pid: u32) -> Result<()> {
    for port in [cfg.mixed_port, cfg.external_controller_port] {
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        if proxy_runtime::tcp_listener_owner(address)? != Some(pid) {
            bail!("端口 {port} 未由当前云阔mihomo 会话监听，未接管系统代理");
        }
    }
    Ok(())
}

/// 只收养属于本配置、且已通过认证探测的服务会话。
pub(super) async fn service_owned_listener(cfg: &AppConfig) -> Result<Option<u32>> {
    let owner =
        proxy_runtime::tcp_listener_owner(SocketAddr::from(([127, 0, 0, 1], cfg.mixed_port)))?;
    if owner.is_none() {
        return Ok(None);
    }
    let Ok((true, pid)) = service::status().await else {
        return Ok(None);
    };
    if owner != Some(pid) {
        return Ok(None);
    }
    verify_owner(cfg, pid)?;
    controller::probe_controller(cfg).await?;
    Ok(Some(pid))
}
