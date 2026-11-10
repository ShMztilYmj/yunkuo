//! 代理运行时：单一状态机，独占宿主与已应用配置。
//! 状态只有一份，读取即当前值；不存在代际号、复用标志等并行事实。
use super::{
    api, controller,
    host::{self, Host},
    network_preflight, policy, service,
};
use crate::config::AppConfig;
use anyhow::{Context, Result, anyhow, bail};
use std::path::Path;

#[derive(Clone)]
struct Applied {
    config: AppConfig,
    yaml: String,
}

#[derive(Clone, Copy)]
enum FailureKind {
    /// 配置应用失败：必须等下一次显式操作才能重新判定成功。
    Operation,
    /// 控制器不可达：一次成功探测即可恢复。
    Controller,
    /// 运行时退出。
    Runtime,
}

enum Phase {
    Stopped,
    Starting,
    Reconfiguring,
    Ready,
    Stopping,
    Failed { kind: FailureKind, message: String },
}

impl Phase {
    fn lifecycle(&self) -> api::Lifecycle {
        match self {
            Self::Stopped => api::Lifecycle::Stopped,
            Self::Starting => api::Lifecycle::Starting,
            Self::Reconfiguring => api::Lifecycle::Reconfiguring,
            Self::Ready => api::Lifecycle::Ready,
            Self::Stopping => api::Lifecycle::Stopping,
            Self::Failed { .. } => api::Lifecycle::Failed,
        }
    }
}

/// 对外观测结果。字段名是前端契约，不要随意更名。
#[derive(Clone, Debug)]
pub(crate) struct ObservedStatus {
    pub phase: api::Lifecycle,
    pub controller_ready: bool,
    pub service_managed: bool,
    pub adopted: bool,
    pub started_at: Option<i64>,
    pub version: Option<String>,
    pub last_error: Option<String>,
    pub generation: u64,
    pub network_revision: u64,
}

/// 一次应用的落地方式；界面据此说明「有没有断线」。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ApplyKind {
    /// 配置没变，沿用现有会话。
    Reused,
    /// 只改路由方式/日志等级，控制器原地改写。
    HotPatch,
    /// 节点、分组或规则变了，控制器原地重载，进程与连接保留。
    HotReload,
    /// 接管、端口或宿主变化，只能重建会话。
    Rebuilt,
}

impl ApplyKind {
    pub(crate) fn note(self) -> &'static str {
        match self {
            Self::Reused => "配置未变化，沿用当前会话",
            Self::HotPatch => "路由方式已原地更新，连接保持",
            Self::HotReload => "订阅已原地热更新，进程与现有连接保持",
            Self::Rebuilt => "已重建内核会话，连接需要重新建立",
        }
    }
}

pub(super) struct Engine {
    host: Option<Host>,
    applied: Option<Applied>,
    phase: Phase,
    controller_ready: bool,
    version: Option<String>,
    started_at: Option<i64>,
    adopted: bool,
    /// 收养或失败后必须完整对账，不能走同配置快路径。
    dirty: bool,
    /// 最近一次真实探测的时刻，用于复用短时间内的重复状态读取。
    checked_at: Option<std::time::Instant>,
    /// 宿主代际：回滚时据此判断旧会话是否仍是同一个。
    host_generation: u64,
}

impl Default for Engine {
    fn default() -> Self {
        Self {
            host: None,
            applied: None,
            phase: Phase::Stopped,
            controller_ready: false,
            version: None,
            started_at: None,
            adopted: false,
            dirty: false,
            checked_at: None,
            host_generation: 0,
        }
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// 收养前复核监听面：进程可能在盘上配置被改过之后仍在运行，不能只看端口归属。
fn adopted_binding_is_safe(value: &serde_yaml::Value, cfg: &AppConfig) -> bool {
    let bind_ok = match value["bind-address"].as_str() {
        None => true,
        Some(address) => address
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback()),
    };
    let tun_ok =
        !cfg.enable_tun || value["tun"]["device"].as_str() == Some(crate::privacy::TUN_NAME);
    bind_ok && tun_ok && value["allow-lan"].as_bool().unwrap_or(false) == cfg.allow_lan
}

/// 两份渲染结果除 `mode` / `log-level` 外完全一致时，返回可原地生效的补丁。
/// 其它字段（端口、TUN、订阅节点、规则）只能靠重建会话生效。
fn hot_patch(previous: &str, next: &str) -> Option<serde_json::Value> {
    let mut before: serde_yaml::Value = serde_yaml::from_str(previous).ok()?;
    let after: serde_yaml::Value = serde_yaml::from_str(next).ok()?;
    let mode = after
        .get("mode")
        .and_then(serde_yaml::Value::as_str)?
        .to_owned();
    let level = after
        .get("log-level")
        .and_then(serde_yaml::Value::as_str)?
        .to_owned();
    let before_mode = before
        .get("mode")
        .and_then(serde_yaml::Value::as_str)?
        .to_owned();
    let before_level = before
        .get("log-level")
        .and_then(serde_yaml::Value::as_str)?
        .to_owned();
    // 先把差异抹平再比较：剩下的任何不同都意味着这次不能原地改。
    *before.get_mut("mode")? = after.get("mode")?.clone();
    *before.get_mut("log-level")? = after.get("log-level")?.clone();
    if before != after {
        return None;
    }
    let mut patch = serde_json::Map::new();
    if before_mode != mode {
        patch.insert("mode".into(), mode.into());
    }
    if before_level != level {
        patch.insert("log-level".into(), level.into());
    }
    (!patch.is_empty()).then(|| serde_json::Value::Object(patch))
}

/// 允许控制器原地替换的数据面键：节点、分组、provider 与规则。
/// 其余键（端口、TUN、DNS 落点、模式、日志等级）都要靠重建或 PATCH 生效。
const RELOADABLE_KEYS: [&str; 6] = [
    "proxies",
    "proxy-groups",
    "proxy-providers",
    "rules",
    "rule-providers",
    "sub-rules",
];

/// 两份渲染结果去掉可热更新键之后完全相同，才允许原地重载。
/// 只要有一个接管参数或 DNS 落点不同，就必须重建会话。
pub(crate) fn reload_compatible(previous: &str, next: &str) -> bool {
    let (Ok(mut before), Ok(mut after)) = (
        serde_yaml::from_str::<serde_yaml::Value>(previous),
        serde_yaml::from_str::<serde_yaml::Value>(next),
    ) else {
        return false;
    };
    for key in RELOADABLE_KEYS {
        if let Some(map) = before.as_mapping_mut() {
            map.remove(key);
        }
        if let Some(map) = after.as_mapping_mut() {
            map.remove(key);
        }
    }
    before == after
}

fn expected_groups(yaml: &str) -> Vec<String> {
    serde_yaml::from_str::<serde_yaml::Value>(yaml)
        .ok()
        .and_then(|value| {
            value["proxy-groups"].as_sequence().map(|rows| {
                rows.iter()
                    .filter_map(|group| group["name"].as_str().map(str::to_owned))
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// `PUT /configs` 按 payload 重载；空路径表示不读磁盘文件。
async fn push_config(cfg: &AppConfig, yaml: &str) -> Result<()> {
    controller::controller_request(
        cfg,
        "PUT",
        "/configs",
        Some(serde_json::json!({ "path": "", "payload": yaml })),
    )
    .await?;
    Ok(())
}

impl Engine {
    pub(super) fn is_running(&self) -> bool {
        self.host.as_ref().is_some_and(Host::alive)
    }

    pub(super) fn applied_tun(&self, mode: &str) -> bool {
        self.is_running()
            && self
                .applied
                .as_ref()
                .is_some_and(|p| p.config.enable_tun && p.config.mode == mode)
    }

    pub(super) fn exit_signal(&self) -> Option<std::sync::Arc<tokio::sync::Notify>> {
        self.host.as_ref()?.exit_signal()
    }

    /// 内核自行退出后的收尾。返回 true 表示本次从“运行中”翻到失败，
    /// 调用方需要还原系统代理；主动停止与配置应用失败都不在这里处理。
    pub(super) fn note_exit(&mut self) -> bool {
        if self.host.is_none()
            || matches!(
                &self.phase,
                Phase::Stopping
                    | Phase::Stopped
                    | Phase::Failed {
                        kind: FailureKind::Runtime,
                        ..
                    }
            )
            || self.is_running()
        {
            return false;
        }
        self.controller_ready = false;
        self.version = None;
        self.phase = Phase::Failed {
            kind: FailureKind::Runtime,
            message: "云阔mihomo 内核已退出".into(),
        };
        true
    }

    /// 只收养属于本配置的认证服务会话，并保留可恢复的旧配置。
    pub(super) async fn recover(&mut self, cfg: &AppConfig, directory: &Path) -> Result<()> {
        if self.host.is_some() || self.applied.is_some() {
            return Ok(());
        }
        let Some(pid) = host::service_owned_listener(cfg).await? else {
            return Ok(());
        };
        let yaml = std::fs::read_to_string(directory.join("clash-config.yaml"))
            .context("现有服务会话缺少可恢复的配置，请先退出旧版云阔再启动新版")?;
        let value: serde_yaml::Value = serde_yaml::from_str(&yaml)?;
        if value["mixed-port"].as_u64() != Some(u64::from(cfg.mixed_port))
            || value["external-controller"].as_str() != Some(cfg.controller_addr().as_str())
            || value["secret"].as_str() != Some(cfg.secret.as_str())
            || value["mode"].as_str() != Some(cfg.mode.as_str())
            || value["tun"]["enable"].as_bool().unwrap_or(false) != cfg.enable_tun
            || !adopted_binding_is_safe(&value, cfg)
        {
            bail!("现有服务配置与已保存配置不一致，请先退出旧版云阔再启动新版");
        }
        proxy_runtime::validate_yaml_with_policy(
            &yaml,
            policy::Plan::resolve(cfg)?.execution_policy,
        )?;
        self.host = Some(Host::adopt_service(pid).await?);
        self.host_generation += 1;
        let version = controller::probe_controller(cfg).await?;
        self.applied = Some(Applied {
            config: cfg.clone(),
            yaml,
        });
        self.adopted = true;
        self.controller_ready = true;
        self.version = Some(version);
        self.started_at = Some(now());
        self.phase = Phase::Ready;
        // 现有服务可能停在上次进程退出前未提交的配置，不能走无变更快路径。
        self.dirty = true;
        Ok(())
    }

    /// 显式解除时收回重启前的服务资源，但不接管无关 PID 或控制器。
    pub(super) async fn recover_for_stop(&mut self, cfg: &AppConfig) -> Result<()> {
        if self.host.is_none()
            && let Some(pid) = host::service_owned_listener(cfg).await?
        {
            self.host = Some(Host::adopt_service(pid).await?);
            self.host_generation += 1;
            self.started_at.get_or_insert_with(now);
        }
        Ok(())
    }

    pub(super) async fn verify(&self, cfg: &AppConfig) -> Result<String> {
        let host = self.host.as_ref().context("云阔没有运行中的代理会话")?;
        let pid = host.pid().await?;
        host::verify_owner(cfg, pid)?;
        if cfg.enable_tun && !proxy_runtime::adapter_up(crate::privacy::TUN_NAME) {
            bail!("云阔mihomo 控制器在线，但 TUN 网卡未运行");
        }
        controller::probe_controller(cfg).await
    }

    pub(super) async fn observe(
        &mut self,
        cfg: &AppConfig,
        generation: u64,
        revision: u64,
    ) -> ObservedStatus {
        // 首页、代理页、托盘可能同时读状态；就绪态下 1 秒内复用同一结论，
        // 避免把一个探测变成一串重复的端口查询与控制器请求。
        if matches!(&self.phase, Phase::Ready)
            && self
                .checked_at
                .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(1))
        {
            return self.snapshot(generation, revision);
        }
        if self.host.is_some() {
            // 归属与认证一律按“已应用的配置”校验：配置事务提交前 ConfigMutex
            // 仍是旧值，拿旧端口去验证新会话必然失败，界面会闪一次假故障。
            let outcome = {
                let target = self
                    .applied
                    .as_ref()
                    .map(|applied| &applied.config)
                    .unwrap_or(cfg);
                self.verify(target).await
            };
            self.checked_at = Some(std::time::Instant::now());
            match outcome {
                Ok(version) => {
                    self.controller_ready = true;
                    self.version = Some(version);
                    if matches!(
                        &self.phase,
                        Phase::Stopped
                            | Phase::Starting
                            | Phase::Reconfiguring
                            | Phase::Failed {
                                kind: FailureKind::Controller,
                                ..
                            }
                    ) {
                        self.phase = Phase::Ready;
                    }
                }
                Err(error) => {
                    self.controller_ready = false;
                    if matches!(
                        &self.phase,
                        Phase::Ready
                            | Phase::Failed {
                                kind: FailureKind::Controller,
                                ..
                            }
                    ) {
                        self.phase = Phase::Failed {
                            kind: FailureKind::Controller,
                            message: error.to_string(),
                        };
                    }
                }
            }
            if !self.is_running()
                && !matches!(&self.phase, Phase::Stopping | Phase::Stopped)
                && !matches!(
                    &self.phase,
                    Phase::Failed {
                        kind: FailureKind::Operation,
                        ..
                    }
                )
            {
                self.controller_ready = false;
                self.phase = Phase::Failed {
                    kind: FailureKind::Runtime,
                    message: "云阔mihomo 会话已停止".into(),
                };
            }
        }
        self.snapshot(generation, revision)
    }

    pub(super) fn snapshot(&self, generation: u64, revision: u64) -> ObservedStatus {
        ObservedStatus {
            phase: self.phase.lifecycle(),
            controller_ready: self.controller_ready,
            service_managed: self.host.as_ref().is_some_and(Host::service_managed),
            adopted: self.adopted,
            started_at: self.started_at,
            version: self
                .controller_ready
                .then(|| self.version.clone())
                .flatten(),
            last_error: match &self.phase {
                Phase::Failed { message, .. } => Some(message.clone()),
                _ => None,
            },
            generation,
            network_revision: revision,
        }
    }

    /// 会话内宿主是粘性的：TUN 打开就需要服务宿主，但关掉 TUN 不必搬回去。
    /// 只在这两种情况下换宿主：首次开启（本机 → 服务）和会话结束后重新启动
    /// （按当前配置重新判定）。避免每次开关 TUN 都跨权限边界重建一次会话。
    fn desired_service_managed(&self, cfg: &AppConfig) -> bool {
        cfg.enable_tun || self.host.as_ref().is_some_and(Host::service_managed)
    }

    /// 准备失败不触碰旧会话；切换失败自动恢复旧会话。
    pub(super) async fn apply(
        &mut self,
        cfg: &AppConfig,
        yaml: String,
        directory: &Path,
        force: bool,
    ) -> Result<ApplyKind> {
        // 配置文本已由 Plan::compile 一次校验并渲染，这里不再重复解析。
        let service_host = self.desired_service_managed(cfg);
        if cfg.enable_tun {
            network_preflight::ensure_tun_available()?;
            service::require_ready().await?;
        }
        let reusable = !force
            && !self.dirty
            && self.applied.as_ref().is_some_and(|old| old.yaml == yaml)
            && self
                .host
                .as_ref()
                .is_some_and(|host| host.service_managed() == service_host);
        if reusable && let Ok(version) = self.verify(cfg).await {
            self.controller_ready = true;
            self.version = Some(version);
            self.started_at.get_or_insert_with(now);
            self.phase = Phase::Ready;
            return Ok(ApplyKind::Reused);
        }
        // 路由方式与日志等级可以交给控制器原地改写：会话保留，现有连接不受影响。
        if let Some(patch) = self.hot_candidate(&yaml, force, service_host) {
            match self.hot_apply(cfg, &yaml, patch, directory).await {
                Ok(()) => {
                    log::info!(
                        "路由方式原地切换：mode={} log-level={}",
                        cfg.mode,
                        cfg.log_level
                    );
                    return Ok(ApplyKind::HotPatch);
                }
                // 内核不接受（模式组合受限）时退回重建会话，结果仍然正确。
                Err(error) => log::warn!("内核原地更新未生效，改为重建会话：{error:#}"),
            }
        }
        // 节点/分组/规则变化优先走控制器原地重载：进程、TUN 与已建立连接都保留。
        // 严格保护不走这条路：WFP 的放行名单是按已冻结的节点端点下发的，
        // 换节点必须先重建会话再重新冻结，否则放行名单与内核实际出站脱节。
        if self.reload_candidate(cfg, &yaml, service_host) {
            match self.hot_reload(cfg, &yaml, directory).await {
                Ok(()) => {
                    log::info!("订阅内容原地热更新：进程与连接保持");
                    return Ok(ApplyKind::HotReload);
                }
                Err(error) => log::warn!("原地热更新未生效，改为重建会话：{error:#}"),
            }
        }
        self.ensure_ports_available(cfg).await?;
        let previous = self.applied.clone();
        let previous_generation = self.host_generation;
        let previous_service = self.host.as_ref().is_some_and(Host::service_managed);
        self.phase = if self.host.is_some() {
            Phase::Reconfiguring
        } else {
            Phase::Starting
        };
        let result = async {
            self.replace(cfg, &yaml, directory, service_host).await?;
            // 生成配置的落盘也是本次会话切换的一部分，失败必须恢复旧会话。
            crate::config::atomic_write(&directory.join("clash-config.yaml"), yaml.as_bytes())
                .context("生成的代理配置保存失败")
        }
        .await;
        match result {
            Ok(()) => {
                self.applied = Some(Applied {
                    config: cfg.clone(),
                    yaml,
                });
                self.dirty = false;
                self.started_at.get_or_insert_with(now);
                self.phase = Phase::Ready;
                Ok(ApplyKind::Rebuilt)
            }
            Err(error) => {
                let restoration = match previous {
                    // 停止失败可能意味着旧会话仍在正常转发，不能再启动第二个内核。
                    Some(old)
                        if self.host_generation == previous_generation
                            && self.verify(&old.config).await.is_ok() =>
                    {
                        Ok(())
                    }
                    Some(old) => {
                        self.replace(&old.config, &old.yaml, directory, previous_service)
                            .await
                    }
                    None => self.stop_host().await,
                };
                let message = match restoration {
                    Ok(()) => {
                        // 旧会话仍在跑，运行时长要延续，不能因为一次失败就清零。
                        self.started_at.get_or_insert_with(now);
                        format!("代理配置未应用：{error:#}")
                    }
                    Err(rollback) => {
                        // 旧会话也没能恢复：磁盘配置不再代表任何在跑的会话，
                        // 必须让下一次应用走完整路径，也不能再报“已恢复”。
                        self.applied = None;
                        self.dirty = true;
                        self.started_at = None;
                        format!("代理配置未应用：{error:#}；旧会话恢复失败：{rollback:#}")
                    }
                };
                self.phase = Phase::Failed {
                    kind: FailureKind::Operation,
                    message: message.clone(),
                };
                Err(anyhow!(message))
            }
        }
    }

    /// 只有路由方式与日志等级不同时才值得原地更新；其余字段都必须重建会话。
    fn hot_candidate(
        &self,
        yaml: &str,
        force: bool,
        service_host: bool,
    ) -> Option<serde_json::Value> {
        if force || self.dirty || !self.is_running() {
            return None;
        }
        // 宿主种类要变（首次开启 TUN）不是原地能改的，先排除。
        if !self
            .host
            .as_ref()
            .is_some_and(|host| host.service_managed() == service_host)
        {
            return None;
        }
        let previous = self.applied.as_ref().filter(|old| old.yaml != yaml)?;
        hot_patch(&previous.yaml, yaml)
    }

    /// 控制器原地改写后读回确认；任何一步失败都不改内存状态，交给调用方重建会话。
    async fn hot_apply(
        &mut self,
        cfg: &AppConfig,
        yaml: &str,
        patch: serde_json::Value,
        directory: &Path,
    ) -> Result<()> {
        let previous = self.applied.clone().context("没有可原地更新的会话")?;
        controller::controller_request(cfg, "PATCH", "/configs", Some(patch)).await?;
        // 内核可能收下请求却拒绝该值（或静默忽略）；以读回的运行态为准，不信 204。
        let live = controller::controller_request(cfg, "GET", "/configs", None).await?;
        if live["mode"].as_str() != Some(cfg.mode.as_str()) {
            bail!("内核未接受路由方式切换");
        }
        if live["tun"]["enable"].as_bool() != Some(cfg.enable_tun) {
            bail!("内核的接管状态与配置不一致");
        }
        if let Err(error) =
            crate::config::atomic_write(&directory.join("clash-config.yaml"), yaml.as_bytes())
        {
            // 落盘失败就把内核改回旧值，磁盘与运行态不能各说各话。
            let _ = controller::controller_request(
                cfg,
                "PATCH",
                "/configs",
                Some(serde_json::json!({
                    "mode": previous.config.mode,
                    "log-level": previous.config.log_level,
                })),
            )
            .await;
            return Err(error.context("生成的代理配置保存失败"));
        }
        self.applied = Some(Applied {
            config: cfg.clone(),
            yaml: yaml.to_string(),
        });
        self.controller_ready = true;
        self.started_at.get_or_insert_with(now);
        self.phase = Phase::Ready;
        Ok(())
    }

    /// 原地重载只接受「数据面」变化：节点、分组、provider 与规则。
    /// 接管开关、端口、DNS 落点或宿主变化仍然必须重建会话。
    fn reload_candidate(&self, cfg: &AppConfig, yaml: &str, service_host: bool) -> bool {
        if !self.is_running()
            || self.dirty
            || cfg.network_mode == Some(super::network_mode::NetworkMode::Strict)
        {
            return false;
        }
        if !self
            .host
            .as_ref()
            .is_some_and(|host| host.service_managed() == service_host)
        {
            return false;
        }
        let Some(previous) = self.applied.as_ref().filter(|old| old.yaml != yaml) else {
            return false;
        };
        reload_compatible(&previous.yaml, yaml)
    }

    /// 控制器原地重载后逐项读回确认；任何一步失败都回滚到旧配置，
    /// 再由调用方决定是否重建会话。
    async fn hot_reload(&mut self, cfg: &AppConfig, yaml: &str, directory: &Path) -> Result<()> {
        let previous = self.applied.clone().context("没有可原地更新的会话")?;
        push_config(cfg, yaml).await?;
        let outcome = async {
            let live = controller::controller_request(cfg, "GET", "/configs", None).await?;
            if live["mixed-port"].as_u64() != Some(u64::from(cfg.mixed_port)) {
                bail!("内核没有按新配置监听混合端口");
            }
            if live["mode"].as_str() != Some(cfg.mode.as_str()) {
                bail!("内核路由方式与配置不一致");
            }
            let proxies = controller::controller_request(cfg, "GET", "/proxies", None).await?;
            for group in expected_groups(yaml) {
                if proxies["proxies"][&group].is_null() {
                    bail!("原地更新后缺少分组「{group}」");
                }
            }
            if cfg.enable_tun {
                controller::wait_tun_ready_for(cfg, std::time::Duration::from_secs(6)).await?;
                if !proxy_runtime::adapter_up(crate::privacy::TUN_NAME) {
                    bail!("原地更新后 TUN 网卡未恢复");
                }
            }
            Ok(())
        }
        .await;
        if let Err(error) = outcome {
            // 恢复旧配置；恢复失败时交给上层直接重建，避免内核停在新旧之间。
            return match push_config(cfg, &previous.yaml).await {
                Ok(()) => Err(error),
                Err(restore) => {
                    Err(error.context(format!("原地更新失败，且旧配置回滚失败：{restore:#}")))
                }
            };
        }
        if let Err(error) =
            crate::config::atomic_write(&directory.join("clash-config.yaml"), yaml.as_bytes())
        {
            let _ = push_config(cfg, &previous.yaml).await;
            return Err(error.context("生成的代理配置保存失败"));
        }
        self.applied = Some(Applied {
            config: cfg.clone(),
            yaml: yaml.to_string(),
        });
        self.controller_ready = true;
        self.started_at.get_or_insert_with(now);
        self.phase = Phase::Ready;
        Ok(())
    }

    async fn replace(
        &mut self,
        cfg: &AppConfig,
        yaml: &str,
        directory: &Path,
        keep_service: bool,
    ) -> Result<()> {
        self.stop_host().await?;
        // 先持有宿主再启动：服务请求标识要早于内核可能已启动的时刻。
        self.set_host(Host::reserve(cfg, keep_service));
        let Some(host) = self.host.as_mut() else {
            bail!("代理宿主未建立");
        };
        host.start(cfg, yaml, directory).await?;
        let version = self.verify(cfg).await?;
        if cfg.enable_tun {
            controller::wait_tun_ready(cfg).await?;
        }
        self.version = Some(version);
        self.controller_ready = true;
        Ok(())
    }

    async fn ensure_ports_available(&self, cfg: &AppConfig) -> Result<()> {
        let owned = match self.host.as_ref() {
            Some(host) => match host.known_pid() {
                Some(pid) => Some(pid),
                None => host.pid().await.ok(),
            },
            None => None,
        };
        for port in [cfg.mixed_port, cfg.external_controller_port] {
            let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
            if let Some(pid) = proxy_runtime::tcp_listener_owner(address)?
                && Some(pid) != owned
            {
                bail!("本机端口 {port} 已被进程 {pid} 占用；现有代理连接保持不变");
            }
        }
        Ok(())
    }

    fn set_host(&mut self, host: Host) {
        self.host = Some(host);
        self.host_generation += 1;
    }

    async fn stop_host(&mut self) -> Result<()> {
        if let Some(host) = self.host.as_mut() {
            host.stop().await?;
        }
        if self.host.take().is_some() {
            self.host_generation += 1;
        }
        self.controller_ready = false;
        self.version = None;
        self.adopted = false;
        Ok(())
    }

    pub(super) async fn stop(&mut self) -> Result<()> {
        self.phase = Phase::Stopping;
        let result = self.stop_host().await;
        match &result {
            Ok(()) => {
                self.applied = None;
                self.dirty = false;
                self.started_at = None;
                self.phase = Phase::Stopped;
            }
            // 停止失败时宿主仍被持有：状态必须落到失败，不能停在 Stopping 上。
            Err(error) => {
                self.phase = Phase::Failed {
                    kind: FailureKind::Operation,
                    message: format!("代理停止失败：{error:#}"),
                };
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        time::Duration,
    };

    fn port() -> u16 {
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn cfg() -> AppConfig {
        AppConfig {
            mixed_port: port(),
            external_controller_port: port(),
            enable_tun: false,
            mode: "rule".into(),
            ..Default::default()
        }
    }

    fn yaml(cfg: &AppConfig) -> String {
        crate::proxy::core::generate_config(
            cfg,
            Some("dns: {enable: false}\nrules: ['MATCH,DIRECT']\n"),
        )
        .unwrap()
    }

    /// 只有数据面（节点/分组/provider/规则）变化才允许原地重载；
    /// 只要接管参数或 DNS 落点变了，就必须重建会话。
    #[test]
    fn reload_only_accepts_data_plane_changes() {
        let base = "mixed-port: 7890\nexternal-controller: 127.0.0.1:19097\nmode: rule\n\
                    tun: {enable: false, device: X}\ndns: {listen: 127.0.0.1:11053, nameserver: [1.1.1.1]}\n\
                    proxies: [{name: A, type: ss}]\nproxy-groups: [{name: G, type: select, proxies: [A]}]\n\
                    rules: [MATCH, G]\n";
        let swapped = base
            .replace("{name: A, type: ss}", "{name: B, type: ss}")
            .replace("proxies: [A]", "proxies: [B]")
            .replace("MATCH, G", "MATCH, B");
        assert!(reload_compatible(base, &swapped));
        // 端口、TUN 开关与 DNS 落点都不属于数据面。
        for changed in [
            base.replace("mixed-port: 7890", "mixed-port: 7891"),
            base.replace("enable: false", "enable: true"),
            base.replace("listen: 127.0.0.1:11053", "listen: 127.0.0.1:11054"),
        ] {
            assert!(!reload_compatible(base, &changed));
        }
        // 无法解析的配置一律不原地更新。
        assert!(!reload_compatible(base, "mixed-port: ["));
    }

    #[test]
    fn update_kinds_explain_whether_connections_survive() {
        assert!(ApplyKind::HotReload.note().contains("连接保持"));
        assert!(ApplyKind::Rebuilt.note().contains("重新建立"));
        assert!(!ApplyKind::Rebuilt.note().contains("保持"));
    }

    /// 经混合端口发一次真实请求，确认内核确实在转发而不是仅控制器在线。
    async fn forward(cfg: &AppConfig) {
        let origin = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = origin.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            origin.set_nonblocking(true).unwrap();
            let start = std::time::Instant::now();
            while start.elapsed() < Duration::from_secs(5) {
                if let Ok((mut stream, _)) = origin.accept() {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let _ = stream.read(&mut [0; 4096]);
                    stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nmihomo!")
                        .unwrap();
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            panic!("代理请求未到达本地测试站点");
        });
        let result = controller::build_client_with_proxy(cfg.mixed_port)
            .unwrap()
            .get(format!("http://{addr}/"))
            .send()
            .await;
        worker.join().unwrap();
        assert_eq!(result.unwrap().text().await.unwrap(), "mihomo!");
    }

    #[test]
    fn tun_off_keeps_an_established_service_host() {
        let cfg = cfg();
        let mut engine = Engine::default();
        // 没有会话时按配置判定：TUN 关闭就该用非提权内核。
        assert!(!engine.desired_service_managed(&cfg));
        // 服务宿主一旦建立，关闭 TUN 不把会话搬回应用进程。
        engine.set_host(Host::reserve(&cfg, true));
        assert!(engine.desired_service_managed(&cfg));
        // 会话结束后重新启动仍按配置判定，SYSTEM 内核不会跨启动常驻。
        assert!(!Engine::default().desired_service_managed(&cfg));
        // 首次开启 TUN 必须选服务宿主。
        let mut with_tun = cfg.clone();
        with_tun.enable_tun = true;
        assert!(Engine::default().desired_service_managed(&with_tun));
        assert!(Host::reserve(&with_tun, false).service_managed());
    }

    #[tokio::test]
    async fn start_reuse_restart_rollback_stop_and_cold_start() {
        let root = crate::test_support::FixtureDir::new();
        let cfg = cfg();
        let text = yaml(&cfg);
        let mut engine = Engine::default();
        engine
            .apply(&cfg, text.clone(), root.path(), false)
            .await
            .unwrap();
        assert!(engine.is_running());
        assert!(!engine.snapshot(0, 0).service_managed);
        forward(&cfg).await;
        let pid = engine.host.as_ref().unwrap().pid().await.unwrap();
        // 同配置再应用不应重启内核。
        engine
            .apply(&cfg, text.clone(), root.path(), false)
            .await
            .unwrap();
        assert_eq!(engine.host.as_ref().unwrap().pid().await.unwrap(), pid);
        // 强制重启必须换进程。
        engine
            .apply(&cfg, text.clone(), root.path(), true)
            .await
            .unwrap();
        assert_ne!(engine.host.as_ref().unwrap().pid().await.unwrap(), pid);
        forward(&cfg).await;
        // 端口冲突必须在停旧会话之前返回。
        let busy = TcpListener::bind("127.0.0.1:0").unwrap();
        let before = engine.host.as_ref().unwrap().pid().await.unwrap();
        let mut next = cfg.clone();
        next.mixed_port = busy.local_addr().unwrap().port();
        assert!(
            engine
                .apply(&next, yaml(&next), root.path(), false)
                .await
                .is_err()
        );
        assert_eq!(engine.host.as_ref().unwrap().pid().await.unwrap(), before);
        engine.verify(&cfg).await.expect("失败后旧会话必须仍能转发");
        forward(&cfg).await;
        engine.stop().await.unwrap();
        assert!(TcpListener::bind(("127.0.0.1", cfg.mixed_port)).is_ok());
        let mut cold = Engine::default();
        cold.apply(&cfg, text, root.path(), false).await.unwrap();
        forward(&cfg).await;
        cold.stop().await.unwrap();
    }

    #[tokio::test]
    async fn routing_and_log_level_switch_in_place_and_keep_the_session() {
        let root = crate::test_support::FixtureDir::new();
        let cfg = cfg();
        let mut engine = Engine::default();
        engine
            .apply(&cfg, yaml(&cfg), root.path(), false)
            .await
            .unwrap();
        let pid = engine.host.as_ref().unwrap().pid().await.unwrap();
        forward(&cfg).await;

        // 路由方式变化：控制器原地切换，进程与连接都不动。
        let mut direct = cfg.clone();
        direct.mode = "direct".into();
        engine
            .apply(&direct, yaml(&direct), root.path(), false)
            .await
            .unwrap();
        assert_eq!(
            engine.host.as_ref().unwrap().pid().await.unwrap(),
            pid,
            "仅路由方式变化必须复用同一会话"
        );
        let live = controller::controller_request(&direct, "GET", "/configs", None)
            .await
            .unwrap();
        assert_eq!(live["mode"].as_str(), Some("direct"));
        // 落盘配置要跟上，下次收养/崩溃恢复才不会回到旧路由。
        let saved: serde_yaml::Value = serde_yaml::from_str(
            &std::fs::read_to_string(root.path().join("clash-config.yaml")).unwrap(),
        )
        .unwrap();
        assert_eq!(saved["mode"].as_str(), Some("direct"));

        // 日志等级同理。
        let mut quiet = direct.clone();
        quiet.log_level = "warning".into();
        engine
            .apply(&quiet, yaml(&quiet), root.path(), false)
            .await
            .unwrap();
        assert_eq!(engine.host.as_ref().unwrap().pid().await.unwrap(), pid);
        let live = controller::controller_request(&quiet, "GET", "/configs", None)
            .await
            .unwrap();
        assert_eq!(live["log-level"].as_str(), Some("warning"));
        forward(&quiet).await;

        // 非路由字段仍然重建会话。
        let mut lan = quiet.clone();
        lan.allow_lan = true;
        engine
            .apply(&lan, yaml(&lan), root.path(), false)
            .await
            .unwrap();
        let replaced = engine.host.as_ref().unwrap().pid().await.unwrap();
        assert_ne!(replaced, pid, "监听范围变化必须重建会话");

        // 显式重启无论配置是否变化都换进程。
        engine
            .apply(&lan, yaml(&lan), root.path(), true)
            .await
            .unwrap();
        assert_ne!(engine.host.as_ref().unwrap().pid().await.unwrap(), replaced);
        engine.stop().await.unwrap();
    }

    #[tokio::test]
    async fn foreign_listener_is_never_reported_ready_or_stopped() {
        let root = crate::test_support::FixtureDir::new();
        let cfg = cfg();
        let foreign = TcpListener::bind(("127.0.0.1", cfg.mixed_port)).unwrap();
        let mut engine = Engine::default();
        assert!(
            engine
                .apply(&cfg, yaml(&cfg), root.path(), false)
                .await
                .is_err()
        );
        assert!(!engine.observe(&cfg, 0, 0).await.controller_ready);
        engine.stop().await.unwrap();
        assert!(TcpListener::bind(foreign.local_addr().unwrap()).is_err());
    }

    #[tokio::test]
    async fn config_save_failure_restores_the_old_routing_on_the_same_ports() {
        let root = crate::test_support::FixtureDir::new();
        let cfg = cfg();
        let mut engine = Engine::default();
        engine
            .apply(&cfg, yaml(&cfg), root.path(), false)
            .await
            .unwrap();
        let generated = root.path().join("clash-config.yaml");
        std::fs::remove_file(&generated).unwrap();
        std::fs::create_dir(&generated).unwrap();
        let mut next = cfg.clone();
        next.mode = "direct".into();
        assert!(
            engine
                .apply(&next, yaml(&next), root.path(), false)
                .await
                .is_err()
        );
        engine.verify(&cfg).await.unwrap();
        let live = controller::controller_request(&cfg, "GET", "/configs", None)
            .await
            .unwrap();
        assert_eq!(
            live["mode"].as_str(),
            Some("rule"),
            "相同端口的响应不能把失败后的新配置误认成已恢复的旧配置"
        );
        engine.stop().await.unwrap();
    }
}
