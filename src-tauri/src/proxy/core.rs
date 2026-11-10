//! Tauri 适配层：所有代理生命周期统一进入 Engine。
pub use super::controller::{
    ConnectionTotals, build_client_with_proxy, connection_totals, controller_request,
    probe_controller, wait_tun_ready,
};
pub use super::service::{
    privacy_request, privacy_status, secure_service_available, service_ping, service_query_status,
};
use crate::config::AppConfig;
use anyhow::{Context, Result};
use parking_lot::Mutex;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager};

/// 正在进行的网络操作。引擎锁被长操作持有时，读取方靠它给出准确相位，
/// 而不是让界面停在“正在读取”或误报“已停止”。
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Busy {
    Idle = 0,
    Starting = 1,
    Stopping = 2,
}

/// 作用域结束时复位“正在操作”标志。
struct BusyGuard<'a> {
    state: &'a CoreState,
}
impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.state.set_busy(Busy::Idle);
    }
}

pub struct CoreState {
    /// 代理运行时状态的唯一序列化点：启停、替换、观测都经过它。
    /// `ConfigUpdateMutex` 是跨模块的「配置持久化事务」锁，两者职责不同——
    /// 只改运行中内核的命令（切节点、关连接）不再占用配置事务锁。
    engine: tokio::sync::Mutex<super::engine::Engine>,
    running: AtomicBool,
    busy: AtomicU8,
    /// 最近一次观测，锁被长操作占用时作为只读兜底。
    last: Mutex<Option<super::engine::ObservedStatus>>,
    /// 宿主变化（启动/替换/停止）：监督者据此重新挂接退出信号。
    host_changed: tokio::sync::Notify,
    pub(super) proxy: Mutex<super::sysproxy::Lease>,
    pub data_dir: Mutex<PathBuf>,
    pub generation: AtomicU64,
    pub network_revision: AtomicU64,
    /// 最近一次成功应用配置的方式：界面用来说明是否换了进程、连接是否保持。
    last_apply: Mutex<Option<ApplyRecord>>,
}
/// 应用结果的一次性投影，不参与状态机判断。
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyRecord {
    pub hot: bool,
    pub at: String,
    pub note: &'static str,
}
impl CoreState {
    pub fn new() -> Self {
        Self {
            engine: tokio::sync::Mutex::new(Default::default()),
            running: AtomicBool::new(false),
            busy: AtomicU8::new(Busy::Idle as u8),
            last: Mutex::new(None),
            host_changed: tokio::sync::Notify::new(),
            proxy: Mutex::new(Default::default()),
            data_dir: Mutex::new(PathBuf::new()),
            generation: AtomicU64::new(0),
            network_revision: AtomicU64::new(0),
            last_apply: Mutex::new(None),
        }
    }
    fn record_apply(&self, kind: super::engine::ApplyKind) {
        *self.last_apply.lock() = Some(ApplyRecord {
            hot: matches!(
                kind,
                super::engine::ApplyKind::HotPatch | super::engine::ApplyKind::HotReload
            ),
            at: chrono::Utc::now().to_rfc3339(),
            note: kind.note(),
        });
    }
    pub fn last_apply(&self) -> Option<ApplyRecord> {
        self.last_apply.lock().clone()
    }
    pub fn is_running(&self) -> bool {
        self.engine
            .try_lock()
            .map(|engine| engine.is_running())
            .unwrap_or_else(|_| self.running.load(Ordering::SeqCst))
    }
    pub fn applied_tun(&self, mode: &str) -> bool {
        self.engine
            .try_lock()
            .is_ok_and(|engine| engine.applied_tun(mode))
    }
    fn set_busy(&self, busy: Busy) {
        self.busy.store(busy as u8, Ordering::SeqCst);
    }
    /// 让“正在操作”标志随作用域复位：命令被取消或任务 panic 时也不会永久
    /// 停在 Starting/Stopping 上，否则界面永远看不到真实相位。
    fn busy_guard(&self, busy: Busy) -> BusyGuard<'_> {
        self.set_busy(busy);
        BusyGuard { state: self }
    }
    fn publish(&self, observed: &super::engine::ObservedStatus) {
        *self.last.lock() = Some(observed.clone());
    }
    /// 锁被占用时的近似状态：只有确实有操作在进行时才改写相位，
    /// 否则沿用上一次观测（并发读取不应被误报成“重配置中”）。
    fn busy_status(&self) -> super::engine::ObservedStatus {
        let override_phase = match self.busy.load(Ordering::SeqCst) {
            value if value == Busy::Starting as u8 => Some(super::api::Lifecycle::Starting),
            value if value == Busy::Stopping as u8 => Some(super::api::Lifecycle::Stopping),
            _ => None,
        };
        let mut observed = self
            .last
            .lock()
            .clone()
            .unwrap_or(super::engine::ObservedStatus {
                phase: super::api::Lifecycle::Stopped,
                controller_ready: false,
                service_managed: false,
                adopted: false,
                started_at: None,
                version: None,
                last_error: None,
                generation: 0,
                network_revision: 0,
            });
        if let Some(phase) = override_phase {
            // 有操作在进行就以它为准：重试启动时应当显示“启动中”，而不是上一次的失败。
            observed.phase = phase;
        }
        observed.generation = self.generation.load(Ordering::SeqCst);
        observed.network_revision = self.network_revision.load(Ordering::SeqCst);
        observed
    }
}
pub fn ensure_geodata(resource_dir: &Path, data_dir: &Path) -> Result<()> {
    let src = resource_dir.join("resources/geodata");
    let fallback = resource_dir.join("geodata");
    let dir = if src.is_dir() { src } else { fallback };
    if !dir.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(data_dir)?;
    for name in ["geosite.dat", "Country.mmdb", "ASN.mmdb"] {
        let from = dir.join(name);
        let to = data_dir.join(name);
        if from.is_file() && !to.exists() {
            std::fs::copy(from, to)?;
        }
    }
    Ok(())
}
#[allow(dead_code, reason = "测试与诊断入口沿用不带覆写的编译路径")]
pub fn generate_config(cfg: &AppConfig, profile: Option<&str>) -> Result<String> {
    generate_config_with_overlay(cfg, profile, &super::overlay::Overlay::default())
}

pub fn generate_config_with_overlay(
    cfg: &AppConfig,
    profile: Option<&str>,
    overlay: &super::overlay::Overlay,
) -> Result<String> {
    super::policy::Plan::resolve(cfg)?.compile_with(profile, overlay)
}

async fn apply(app: &AppHandle, cfg: &AppConfig, force: bool, take_proxy: bool) -> Result<()> {
    let state = app.state::<CoreState>();
    let directory = state.data_dir.lock().clone();
    let content = super::profile::ProfileManager::new(&directory).current_content()?;
    let overlay = super::overlay::load(&directory)?;
    let yaml = generate_config_with_overlay(cfg, content.as_deref(), &overlay)?;
    let previous = app.state::<crate::config::ConfigMutex>().lock().clone();
    let busy = state.busy_guard(Busy::Starting);
    let result = async {
        let mut engine = state.engine.lock().await;
        engine.recover(&previous, &directory).await?;
        state.network_revision.fetch_add(1, Ordering::SeqCst);
        let outcome = engine.apply(cfg, yaml, &directory, force).await;
        state.running.store(engine.is_running(), Ordering::SeqCst);
        state.generation.fetch_add(1, Ordering::SeqCst);
        let result = match outcome {
            Ok(kind) => {
                state.record_apply(kind);
                if take_proxy {
                    super::sysproxy::apply(app, cfg)
                } else {
                    Ok(())
                }
            }
            Err(error) => Err(error),
        };
        state.publish(&engine.snapshot(
            state.generation.load(Ordering::SeqCst),
            state.network_revision.load(Ordering::SeqCst),
        ));
        result
    }
    .await;
    drop(busy);
    // 宿主已变化：监督者要重新挂接到新的退出信号上。
    state.host_changed.notify_one();
    let _ = app.emit(
        "core-event",
        if result.is_ok() { "started" } else { "failed" },
    );
    result
}
pub async fn apply_config(app: &AppHandle, cfg: &AppConfig) -> Result<()> {
    apply(app, cfg, false, false).await
}
pub async fn start_core(app: &AppHandle) -> Result<()> {
    let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
    super::sysproxy::recover_stale(app, &cfg)?;
    apply(app, &cfg, false, true).await
}
pub async fn restart_core(app: &AppHandle) -> Result<()> {
    let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
    apply(app, &cfg, true, true).await
}
pub async fn verify_proxy_ready(app: &AppHandle, cfg: &AppConfig) -> Result<String> {
    let state = app.state::<CoreState>();
    // 有界等待：TUN 启动可能长时间持有运行时锁，调用方（托盘、旧控制器入口）
    // 不该被一起挂住。
    let Ok(engine) = tokio::time::timeout(Duration::from_secs(5), state.engine.lock()).await else {
        anyhow::bail!("代理正在执行其它操作，请稍后再试");
    };
    engine.verify(cfg).await
}
pub fn child_alive(app: &AppHandle) -> bool {
    app.state::<CoreState>().is_running()
}
pub fn stop_core(app: &AppHandle) {
    if let Err(e) = tauri::async_runtime::block_on(stop_core_async(app)) {
        crate::platform::notifications::report(app, format!("代理停止失败：{e:#}"));
    }
}
async fn stop(app: &AppHandle, recover: bool) -> Result<()> {
    let state = app.state::<CoreState>();
    let busy = state.busy_guard(Busy::Stopping);
    let result = async {
        let mut engine = state.engine.lock().await;
        if recover {
            let cfg = { app.state::<crate::config::ConfigMutex>().lock().clone() };
            engine.recover_for_stop(&cfg).await?;
        }
        // 与启停共用运行时锁，避免恢复代理后另一个启动入口再次写回已停止端口。
        super::sysproxy::release(app)?;
        state.network_revision.fetch_add(1, Ordering::SeqCst);
        let result = engine.stop().await;
        state.running.store(engine.is_running(), Ordering::SeqCst);
        state.generation.fetch_add(1, Ordering::SeqCst);
        state.publish(&engine.snapshot(
            state.generation.load(Ordering::SeqCst),
            state.network_revision.load(Ordering::SeqCst),
        ));
        result
    }
    .await;
    drop(busy);
    state.host_changed.notify_one();
    let _ = app.emit("core-event", "stopped");
    result
}
pub async fn stop_core_async(app: &AppHandle) -> Result<()> {
    stop(app, false).await
}
pub async fn stop_for_protection_release(app: &AppHandle) -> Result<()> {
    // 显式解除也只停止被本配置认证、且监听端口归属已确认的服务会话。
    stop(app, true).await
}
pub(crate) async fn observe_status(
    state: &CoreState,
    cfg: &AppConfig,
) -> super::engine::ObservedStatus {
    // 读取有界等待：长操作（TUN 启动、服务安装、内核替换）持有运行时锁时，
    // 界面拿上一次观测继续渲染，而不是一起卡住。
    let Ok(mut engine) =
        tokio::time::timeout(Duration::from_millis(400), state.engine.lock()).await
    else {
        return state.busy_status();
    };
    let observed = engine
        .observe(
            cfg,
            state.generation.load(Ordering::SeqCst),
            state.network_revision.load(Ordering::SeqCst),
        )
        .await;
    state.running.store(engine.is_running(), Ordering::SeqCst);
    state.publish(&observed);
    observed
}

/// 内核监督：本机内核由进程退出事件唤醒，服务宿主退化为低频存活确认。
/// 内核正常运行期间本任务一直挂在等待上，没有周期探测。
pub fn spawn_supervisor(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let state = app.state::<CoreState>();
            let signal = state
                .engine
                .try_lock()
                .ok()
                .and_then(|engine| engine.exit_signal());
            let wait = async {
                match signal {
                    // 本机内核：一直挂到进程真正退出。
                    Some(signal) => signal.notified().await,
                    // 服务宿主没有本地句柄，只做低频存活确认。
                    None => tokio::time::sleep(Duration::from_secs(30)).await,
                }
            };
            tokio::select! {
                _ = wait => {}
                _ = state.host_changed.notified() => {}
            }
            // 退出通知是一次性的：收尾没拿到锁必须重试，不能把事件丢掉。
            while !reconcile(&app).await {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    });
}

/// 只在内核确实从「运行中」翻到失败时才还原系统代理；主动停止不在此处理。
/// 返回 false 表示运行时锁被长操作占用，调用方需要重试。
async fn reconcile(app: &AppHandle) -> bool {
    let state = app.state::<CoreState>();
    let Ok(mut engine) = tokio::time::timeout(Duration::from_secs(5), state.engine.lock()).await
    else {
        return false;
    };
    if !engine.note_exit() {
        return true;
    }
    state.publish(&engine.snapshot(
        state.generation.load(Ordering::SeqCst),
        state.network_revision.load(Ordering::SeqCst),
    ));
    state.running.store(false, Ordering::SeqCst);
    drop(engine);
    // 守卫开关的语义现在是“内核异常退出时自动还原系统代理”，默认开启。
    // 关掉它只影响还原动作，不影响状态上报。
    if app
        .state::<crate::config::ConfigMutex>()
        .lock()
        .enable_proxy_guard
        && let Err(error) = super::sysproxy::release(app)
    {
        crate::platform::notifications::report(
            app,
            format!("内核已退出，但系统代理还原失败：{error:#}"),
        );
    }
    let _ = app.emit("core-event", "failed");
    true
}
#[cfg(test)]
pub async fn refresh_running_state(state: &CoreState, cfg: &AppConfig) -> bool {
    observe_status(state, cfg).await.controller_ready
}
pub async fn install_service(app: &AppHandle) -> Result<()> {
    super::service_install::install(app).await
}
pub async fn uninstall_service(app: &AppHandle) -> Result<()> {
    super::service_install::uninstall(app).await
}
/// 诊断日志：把应用内探测的原始结果写入数据目录，便于远程排查
pub fn write_probe_log(data_dir: &Path, line: &str) {
    use std::io::Write;
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(data_dir.join("probe.log"))
    {
        let _ = writeln!(f, "[{ts}] {line}");
    }
}

/// 读取磁盘 config.yaml 的端口与密钥长度（与内存配置比对用）
pub fn disk_cfg_snapshot(config_path: &Path) -> (String, usize) {
    let mut port = String::new();
    let mut secret_len = 0usize;
    if let Ok(text) = std::fs::read_to_string(config_path) {
        for l in text.lines() {
            let l = l.trim();
            if let Some(v) = l.strip_prefix("external_controller_port:") {
                port = v.trim().to_string();
            }
            if let Some(v) = l.strip_prefix("secret:") {
                secret_len = v.trim().len();
            }
        }
    }
    (port, secret_len)
}

/// 探测失败后以相同认证与响应校验重试一次，并记录不含密钥的诊断。
pub async fn probe_with_fallback(cfg: &AppConfig, data_dir: &Path) -> Result<String> {
    match probe_controller(cfg).await {
        Ok(v) => Ok(v),
        Err(_) => {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let retry = probe_controller(cfg).await;
            write_probe_log(
                data_dir,
                &format!(
                    "authenticated_probe_retry: ok={} controller_port={}",
                    retry.is_ok(),
                    cfg.external_controller_port
                ),
            );
            retry.context("内核控制器认证、HTTP 状态或版本检查未通过")
        }
    }
}

/// 当前进程是否已提权
pub fn is_elevated() -> bool {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows::core::PCWSTR;

    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut ret_len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        );
        let _ = PCWSTR::null();
        let _ = ret_len;
        windows::Win32::Foundation::CloseHandle(token).ok();
        ok.is_ok() && elevation.TokenIsElevated != 0
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use std::sync::atomic::AtomicBool;
    #[test]
    fn routing_choices_restore_subscription_rules_in_generated_configuration() {
        use crate::network_mode::{NetworkMode, RoutingMode, routing_target};
        for tun in [false, true] {
            for mode in [RoutingMode::Global, RoutingMode::Rule, RoutingMode::Direct] {
                let before = AppConfig {
                    network_mode: Some(NetworkMode::Daily),
                    enable_tun: tun,
                    ..Default::default()
                };
                let target = routing_target(&before, mode, false, false).unwrap();
                let yaml = generate_config(
                    &target,
                    Some("proxies: []\nproxy-groups: []\nrules: ['MATCH,DIRECT']\n"),
                )
                .unwrap();
                let value: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
                assert_eq!(value["mode"].as_str(), Some(mode.as_str()));
                assert_eq!(value["rules"][0], "MATCH,DIRECT");
                assert!(!yaml.contains("云阔·海外节点"));
                let sanitized = crate::service_protocol::sanitized_config(&yaml).unwrap();
                let sanitized: serde_yaml::Value = serde_yaml::from_str(&sanitized).unwrap();
                assert_eq!(sanitized["mode"].as_str(), Some(mode.as_str()));
            }
        }
    }

    use super::super::controller::wait_tun_ready_for;
    use super::super::profile::ProfileManager;
    use super::*;

    #[test]
    fn connection_totals_skips_large_connection_rows() {
        let row = r#"{"id":"fixture","metadata":{"host":"example.invalid","processPath":"fixture.exe"},"chains":["fixture"],"upload":123,"download":456}"#;
        let payload = format!(
            r#"{{"uploadTotal":123456,"downloadTotal":789012,"connections":[{}]}}"#,
            vec![row; 10_000].join(",")
        );
        let started = std::time::Instant::now();
        for _ in 0..10 {
            let full: Value = serde_json::from_str(std::hint::black_box(&payload)).unwrap();
            assert_eq!(full["uploadTotal"], 123456);
            std::hint::black_box(full);
        }
        let full = started.elapsed();
        let started = std::time::Instant::now();
        for _ in 0..10 {
            let totals: ConnectionTotals =
                serde_json::from_str(std::hint::black_box(&payload)).unwrap();
            assert_eq!(
                totals,
                ConnectionTotals {
                    upload_total: 123456,
                    download_total: 789012
                }
            );
            assert_eq!(
                serde_json::to_value(totals)
                    .unwrap()
                    .as_object()
                    .unwrap()
                    .len(),
                2
            );
        }
        println!(
            "connections projection: payload={} rows=10000 iterations=10 full_ms={:.3} projected_ms={:.3}",
            payload.len(),
            full.as_secs_f64() * 1000.,
            started.elapsed().as_secs_f64() * 1000.
        );
        assert!(serde_json::from_str::<ConnectionTotals>(r#"{"uploadTotal":"invalid"}"#).is_err());
        assert_eq!(
            serde_json::from_str::<ConnectionTotals>("{}")
                .unwrap()
                .upload_total,
            0
        );
    }

    #[tokio::test]
    async fn connection_totals_uses_authenticated_local_controller() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = [0; 4096];
            let length = socket.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..length]).to_ascii_lowercase();
            assert!(request.starts_with("get /connections http/1.1"));
            assert!(request.contains("authorization: bearer fixture-secret"));
            let body = r#"{"uploadTotal":12,"downloadTotal":34,"connections":[{"metadata":{"host":"example.invalid"}}]}"#;
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let cfg = AppConfig {
            external_controller_port: port,
            secret: "fixture-secret".into(),
            ..Default::default()
        };
        assert_eq!(
            connection_totals(&cfg).await.unwrap(),
            ConnectionTotals {
                upload_total: 12,
                download_total: 34
            }
        );
        server.join().unwrap();
    }

    #[tokio::test]
    async fn tun_waits_for_async_initialization_and_own_device() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            for body in [
                r#"{"tun":{"enable":false,"device":""}}"#,
                r#"{"tun":{"enable":true,"device":"FlClash"}}"#,
                r#"{"tun":{"enable":true,"device":"云阔TUN"}}"#,
            ] {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let _ = socket.read(&mut [0; 4096]);
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let cfg = AppConfig {
            external_controller_port: port,
            ..Default::default()
        };
        wait_tun_ready_for(&cfg, Duration::from_secs(2))
            .await
            .expect("the first controller reply may precede TUN initialization");
        server.join().unwrap();
    }

    #[tokio::test]
    async fn tun_wait_is_bounded_and_unknown_never_passes() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let _ = socket.read(&mut [0; 4096]);
            std::thread::sleep(Duration::from_millis(600));
            let body = r#"{"tun":{"enable":true,"device":"云阔TUN"}}"#;
            let _ = write!(
                socket,
                "HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        });
        let cfg = AppConfig {
            external_controller_port: port,
            ..Default::default()
        };
        let started = std::time::Instant::now();
        assert!(
            wait_tun_ready_for(&cfg, Duration::from_millis(150))
                .await
                .is_err()
        );
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "the wait budget must include slow HTTP calls"
        );
        server.join().unwrap();
    }

    #[tokio::test]
    async fn unowned_controller_never_becomes_a_running_session() {
        let state = CoreState::new();
        let cfg = AppConfig::default();
        assert!(!refresh_running_state(&state, &cfg).await);
        assert!(!state.is_running());
    }

    #[test]
    #[ignore = "显式启用：读取当前订阅，使用独立端口测试真实 HTTPS；不创建 TUN 或改系统网络"]
    fn isolated_custom_tun_subscription_https_smoke() {
        assert_eq!(
            std::env::var("YUNKUO_SUBSCRIPTION_SMOKE").as_deref(),
            Ok("1")
        );
        fn unused_port() -> u16 {
            std::net::TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
                .port()
        }
        let fixture = crate::test_support::FixtureDir::new();
        let data = PathBuf::from(std::env::var_os("APPDATA").unwrap()).join("com.yunkuo.app");
        let mut cfg: AppConfig =
            serde_yaml::from_str(&std::fs::read_to_string(data.join("config.yaml")).unwrap())
                .unwrap();
        cfg.network_mode = None;
        cfg.enable_tun = true;
        cfg.mixed_port = unused_port();
        cfg.external_controller_port = unused_port();
        let subscription = ProfileManager::new(&data)
            .current_content()
            .unwrap()
            .expect("当前订阅");
        let mut value: serde_yaml::Value =
            serde_yaml::from_str(&generate_config(&cfg, Some(&subscription)).unwrap()).unwrap();
        assert_eq!(value["tun"]["enable"], true);
        // Exercise the exact production DNS plan while keeping all OS interception off.
        value["tun"]["enable"] = false.into();
        value["dns"]["listen"] = format!("127.0.0.1:{}", unused_port()).into();
        ensure_geodata(Path::new(env!("CARGO_MANIFEST_DIR")), fixture.path()).unwrap();
        std::fs::create_dir_all(fixture.path().join("mihomo")).unwrap();
        let cache = data.join("mihomo/cache.db");
        if cache.is_file() {
            std::fs::copy(&cache, fixture.path().join("mihomo/cache.db")).unwrap();
        }
        let runner = tokio::runtime::Runtime::new().unwrap();
        runner.block_on(async {
            let mut runtime=proxy_runtime::Runtime::start(proxy_runtime::StartOptions {
                yaml:serde_yaml::to_string(&value).unwrap(),work_dir:fixture.path().into()
            }).await.unwrap();
            let client=reqwest::Client::builder().proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{}",cfg.mixed_port)).unwrap()).timeout(Duration::from_secs(10)).build().unwrap();
            let mut results=Vec::new();
            for target in ["https://example.com/","https://ippure.com/"] {
                let response=client.get(target).send().await;
                results.push(response.is_ok_and(|r|r.status().is_success()));
            }
            runtime.stop().await.unwrap();
            assert!(results.iter().all(|passed|*passed),"隔离订阅 HTTPS 请求结果：{results:?}");
            println!("isolated subscription: example.com=200, ippure.com=200; TUN disabled; no system changes");
        });
    }

    #[test]
    fn daily_policy_keeps_domestic_direct_when_selected_node_fails() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::{Arc, atomic::AtomicUsize};
        fn unused_port() -> u16 {
            TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
                .port()
        }
        let fixture = crate::test_support::FixtureDir::new();
        let origin = TcpListener::bind("127.0.0.1:0").unwrap();
        origin.set_nonblocking(true).unwrap();
        let origin_port = origin.local_addr().unwrap().port();
        let received = Arc::new(AtomicUsize::new(0));
        let finished = Arc::new(AtomicBool::new(false));
        let worker_count = received.clone();
        let worker_done = finished.clone();
        let server = std::thread::spawn(move || {
            while !worker_done.load(Ordering::SeqCst) {
                if let Ok((mut socket, _)) = origin.accept() {
                    socket
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let _ = socket.read(&mut [0u8; 4096]);
                    worker_count.fetch_add(1, Ordering::SeqCst);
                    let _ = socket.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nfixture",
                    );
                } else {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        });
        let cfg = AppConfig {
            mixed_port: unused_port(),
            external_controller_port: unused_port(),
            enable_tun: true,
            enable_system_proxy: true,
            ipv6: false,
            mode: "rule".into(),
            network_mode: Some(crate::network_mode::NetworkMode::Daily),
            daily_proxy: Some("offline-node".into()),
            ..Default::default()
        };
        let profile = format!(
            "proxies:\n- {{name: offline-node, type: http, server: 127.0.0.1, port: {}}}\n",
            unused_port()
        );
        let mut yaml: serde_yaml::Value =
            serde_yaml::from_str(&generate_config(&cfg, Some(&profile)).unwrap()).unwrap();
        // Compile the real managed plan, then disable only the OS-interception fixture.
        yaml["tun"]["enable"] = false.into();
        yaml["dns"]["listen"] = "127.0.0.1:0".into();
        yaml["dns"]["use-hosts"] = true.into();
        // 只在测试配置启用回环 hosts，所有 DNS 意外请求也限定到本机不可用端口。
        yaml["hosts"] = serde_yaml::to_value(
            serde_json::json!({"www.baidu.com":"127.0.0.1", "claude.ai":"127.0.0.1"}),
        )
        .unwrap();
        for key in [
            "nameserver",
            "default-nameserver",
            "proxy-server-nameserver",
            "direct-nameserver",
        ] {
            yaml["dns"][key] = serde_yaml::to_value(["127.0.0.1:1"]).unwrap();
        }
        yaml["dns"]["nameserver-policy"] = serde_yaml::Value::Mapping(Default::default());
        yaml["geo-auto-update"] = false.into();
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        for name in ["geosite.dat", "Country.mmdb", "ASN.mmdb"] {
            std::fs::copy(
                root.join("resources/geodata").join(name),
                fixture.path().join(name),
            )
            .unwrap();
        }
        let config_path = fixture.path().join("daily-loopback.yaml");
        std::fs::write(&config_path, serde_yaml::to_string(&yaml).unwrap()).unwrap();
        let runner = tokio::runtime::Runtime::new().unwrap();
        let mut native = runner
            .block_on(proxy_runtime::Runtime::start(proxy_runtime::StartOptions {
                yaml: serde_yaml::to_string(&yaml).unwrap(),
                work_dir: fixture.path().into(),
            }))
            .unwrap();
        let result = std::panic::catch_unwind(|| {
            let direct = reqwest::blocking::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(1))
                .build()
                .unwrap();
            let mut ready = false;
            // 与 295 个测试并行时内核启动会变慢，留足 15 秒再判定失败。
            for _ in 0..150 {
                if direct
                    .get(format!(
                        "http://127.0.0.1:{}/version",
                        cfg.external_controller_port
                    ))
                    .bearer_auth(&cfg.secret)
                    .send()
                    .is_ok()
                {
                    ready = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            assert!(ready, "隔离测试内核未就绪");
            let client = reqwest::blocking::Client::builder()
                .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{}", cfg.mixed_port)).unwrap())
                .timeout(Duration::from_secs(8))
                .build()
                .unwrap();
            let domestic = client
                .get(format!("http://www.baidu.com:{origin_port}/"))
                .send()
                .unwrap();
            assert_eq!(domestic.status(), 200);
            let ai = client
                .get(format!("http://claude.ai:{origin_port}/"))
                .send();
            assert!(
                !ai.is_ok_and(|response| response.status().is_success()),
                "AI 域名在代理节点失败时不得走局域网直连例外"
            );
            assert_eq!(
                received.load(Ordering::SeqCst),
                1,
                "只有国内请求应到达回环测试站点"
            );
        });
        finished.store(true, Ordering::SeqCst);
        server.join().unwrap();
        runner.block_on(native.stop()).unwrap();
        if let Err(error) = result {
            std::panic::resume_unwind(error);
        }
    }

    #[test]
    fn daily_config_is_accepted_by_native_schema() {
        let cfg = AppConfig {
            enable_tun: true,
            enable_system_proxy: true,
            ipv6: false,
            mode: "rule".into(),
            network_mode: Some(crate::network_mode::NetworkMode::Daily),
            ..Default::default()
        };
        let yaml=generate_config(&cfg,Some("proxies:\n- {name: US, type: trojan, server: 203.0.113.10, port: 443, password: test-only}\n")).unwrap();
        proxy_runtime::validate_yaml(&crate::service_protocol::sanitized_config(&yaml).unwrap())
            .unwrap();
    }

    #[test]
    fn protected_plan_is_accepted_by_native_schema() {
        let cfg = super::super::policy::managed_target(
            &AppConfig::default(),
            crate::network_mode::NetworkMode::Strict,
        );
        let yaml=generate_config(&cfg,Some("proxies:\n- {name: test-node, type: trojan, server: 203.0.113.10, port: 443, password: test-only}\n")).unwrap();
        crate::privacy::validate(&yaml).unwrap();
        proxy_runtime::validate_yaml_with_policy(&yaml, proxy_runtime::ExecutionPolicy::Protected)
            .unwrap();
    }

    #[test]
    fn generated_ipv6_bypass_survives_both_service_checks_and_native_schema() {
        let yaml = generate_config(
            &AppConfig::default(),
            Some("skip-proxy: ['localhost', '::1']\nproxies: []\nrules: ['MATCH,DIRECT']\n"),
        )
        .unwrap();
        let checked = crate::service_protocol::sanitized_config(
            &crate::service_protocol::sanitized_config(&yaml).unwrap(),
        )
        .unwrap();
        proxy_runtime::validate_yaml(&checked).unwrap();
    }

    use std::io::{Read, Write};

    #[test]
    fn generated_config_applies_both_ports_and_overrides_subscription_controller() {
        let cfg = AppConfig {
            mixed_port: 6789,
            external_controller_port: 6790,
            ..Default::default()
        };
        let yaml = generate_config(
            &cfg,
            Some("proxies: []\nmixed-port: 1\nexternal-controller: 0.0.0.0:2\n"),
        )
        .unwrap();
        let value: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(value["mixed-port"].as_u64(), Some(6789));
        assert_eq!(
            value["external-controller"].as_str(),
            Some("127.0.0.1:6790")
        );
    }

    #[tokio::test]
    async fn controller_auth_failure_never_means_running() {
        let fixture = crate::test_support::FixtureDir::new();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buf = [0; 1024];
                while !bytes.windows(4).any(|b| b == b"\r\n\r\n") {
                    let count = socket.read(&mut buf).unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buf[..count]);
                }
                socket.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            }
        });
        let cfg = AppConfig {
            external_controller_port: port,
            ..Default::default()
        };
        let result = probe_with_fallback(&cfg, fixture.path()).await;
        server.join().unwrap();
        assert!(result.is_err(), "HTTP 401 must fail the controller probe");
    }
}
