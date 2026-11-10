// yunkuo-service —— Windows 系统服务
// 作用：以 SYSTEM 权限托管 mihomo 子进程，TUN 无需每次 UAC。
// 由 GUI 安装（sc create），通过 127.0.0.1:33215 的本地 HTTP 接口控制，
// 接口使用 ProgramData 下 service_token.txt 中的令牌鉴权。

#![windows_subsystem = "windows"]

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

// 网络保护的实现来自 workspace 成员 crate，与主程序共用同一份代码：
// 服务程序因此不必链接 Tauri，也不再用 #[path] 包含主程序的源文件。
use protection::{config, faults, network_lock, privacy_service, service_protocol};
use service_protocol::StartRequest;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

#[path = "../service_host/runtime.rs"]
mod runtime;
use runtime::ManagedCore;

const SERVICE_NAME: &str = "YunkuoService";
const PORT: u16 = 33215;

static STOP: AtomicBool = AtomicBool::new(false);
static CHILD: Mutex<Option<ManagedCore>> = Mutex::new(None);
// HTTP 修改与 SCM 退出共用此锁，观察身份和停止/替换必须是一个操作。
static CONTROL: Mutex<()> = Mutex::new(());

fn data_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(r"C:\ProgramData\Yunkuo")
}

fn log(msg: &str) {
    let _ = std::fs::create_dir_all(data_dir());
    let line = format!(
        "[{}] {}\r\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        msg
    );
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(data_dir().join("service.log"))
    {
        let _ = f.write_all(line.as_bytes());
    }
}

fn read_token() -> Option<String> {
    std::fs::read_to_string(data_dir().join("service_token.txt"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn kill_core() -> Result<(), String> {
    let _operation = CONTROL.lock().map_err(|_| "服务操作锁失效")?;
    kill_core_inner(None, None)
}

fn kill_core_inner(
    expected_pid: Option<u32>,
    expected_session: Option<&str>,
) -> Result<(), String> {
    let mut guard = CHILD.lock().map_err(|_| "内核状态锁失效")?;
    if let Some(child) = guard.as_mut() {
        let running = child.try_wait().map_err(|_| "查询内核状态失败")?.is_none();
        if running
            && (expected_pid.is_some_and(|pid| pid != child.id())
                || expected_session.is_some_and(|id| child.session_id() != Some(id)))
        {
            return Err("服务会话已变化，未停止其他会话".into());
        }
    }
    privacy_service::seal("代理内核已停止，防泄露阻断继续保留").map_err(|e| format!("{e:#}"))?;
    if let Some(child) = guard.as_mut() {
        if child.try_wait().map_err(|_| "查询内核状态失败")?.is_none()
            && child.kill().is_err()
            && child.try_wait().map_err(|_| "查询内核状态失败")?.is_none()
        {
            return Err("内核未能停止".into());
        }
        child.wait().map_err(|_| "内核资源尚未确认回收")?;
    }
    guard.take();
    log("core stopped");
    Ok(())
}

fn handle_stop(body: &str) -> (u16, String) {
    let parsed = (|| -> anyhow::Result<(Option<u32>, Option<String>)> {
        if body.trim().is_empty() {
            return Ok((None, None));
        }
        let value: serde_json::Value = serde_json::from_str(body)?;
        if !value.is_object() {
            anyhow::bail!("stop request must be an object");
        }
        let pid = match value.get("expectedPid") {
            Some(value) => Some(u32::try_from(
                value
                    .as_u64()
                    .ok_or_else(|| anyhow::anyhow!("invalid expected PID"))?,
            )?),
            None => None,
        };
        let session = match value.get("expectedSessionId") {
            Some(value) => {
                let id = value
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("invalid expected session"))?;
                uuid::Uuid::parse_str(id)?;
                Some(id.to_owned())
            }
            None => None,
        };
        Ok((pid, session))
    })();
    let (pid, session) = match parsed {
        Ok(value) => value,
        Err(_) => return (400, r#"{"error":"invalid stop request"}"#.into()),
    };
    let _operation = match CONTROL.lock() {
        Ok(lock) => lock,
        Err(_) => return (500, r#"{"error":"service operation unavailable"}"#.into()),
    };
    match kill_core_inner(pid, session.as_deref()) {
        Ok(()) => (200, r#"{"ok":true}"#.into()),
        Err(error) => (
            if error == "服务会话已变化，未停止其他会话" {
                409
            } else {
                500
            },
            serde_json::json!({"error":error}).to_string(),
        ),
    }
}

define_windows_service!(ffi_service_main, service_main_fn);

fn service_main_fn(_args: Vec<std::ffi::OsString>) {
    if let Err(e) = run_service() {
        log(&format!("service error: {e}"));
    }
}

fn run_service() -> windows_service::Result<()> {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let status_handle = service_control_handler::register(SERVICE_NAME, move |ctrl| match ctrl {
        ServiceControl::Stop | ServiceControl::Shutdown | ServiceControl::Preshutdown => {
            STOP.store(true, Ordering::SeqCst);
            let _ = tx.send(());
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })?;

    let initialize = std::thread::spawn(|| {
        privacy_service::install_bootstrap_resolver(|host| {
            runtime::block_on(proxy_runtime::bootstrap_ipv4(host))
        });
        if service_protocol::runtime_dir().is_dir() {
            if let Err(error) = proxy_runtime::recover_dns(&service_protocol::runtime_dir()) {
                log(&format!("native DNS recovery pending: {error:#}"));
            }
        }
        if let Err(error) = privacy_service::initialize() {
            log(&format!(
                "network protection initialization failed: {error:#}"
            ));
        }
    });
    let mut checkpoint = 1;
    while !initialize.is_finished() {
        set_status(
            &status_handle,
            ServiceState::StartPending,
            ServiceControlAccept::empty(),
            checkpoint,
            ServiceExitCode::Win32(0),
        )?;
        checkpoint += 1;
        std::thread::sleep(Duration::from_millis(1000));
    }
    let _ = initialize.join();

    set_status(
        &status_handle,
        ServiceState::Running,
        ServiceControlAccept::STOP
            | ServiceControlAccept::SHUTDOWN
            | ServiceControlAccept::PRESHUTDOWN,
        0,
        ServiceExitCode::Win32(0),
    )?;

    let http_thread = std::thread::spawn(run_http);
    let watch_thread = std::thread::spawn(|| {
        while !STOP.load(Ordering::SeqCst) {
            privacy_service::watch();
            std::thread::sleep(Duration::from_millis(1000));
        }
    });

    let _ = rx.recv();
    let cleanup = std::thread::spawn(move || {
        let _ = kill_core();
        let _ = http_thread.join();
        let stopped = kill_core();
        let _ = watch_thread.join();
        stopped
    });
    let mut checkpoint = 1;
    while !cleanup.is_finished() {
        set_status(
            &status_handle,
            ServiceState::StopPending,
            ServiceControlAccept::empty(),
            checkpoint,
            ServiceExitCode::Win32(0),
        )?;
        checkpoint += 1;
        std::thread::sleep(Duration::from_millis(1000));
    }
    let result = cleanup
        .join()
        .unwrap_or_else(|_| Err("service cleanup panicked".into()));
    if let Err(error) = &result {
        log(&format!("service cleanup incomplete: {error}"));
    }
    set_status(
        &status_handle,
        ServiceState::Stopped,
        ServiceControlAccept::empty(),
        0,
        if result.is_ok() {
            ServiceExitCode::Win32(0)
        } else {
            ServiceExitCode::ServiceSpecific(1)
        },
    )?;
    Ok(())
}

fn set_status(
    handle: &windows_service::service_control_handler::ServiceStatusHandle,
    state: ServiceState,
    accepts: ServiceControlAccept,
    checkpoint: u32,
    exit_code: ServiceExitCode,
) -> windows_service::Result<()> {
    handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: accepts,
        exit_code,
        checkpoint,
        wait_hint: Duration::from_secs(
            if matches!(
                state,
                ServiceState::StopPending | ServiceState::StartPending
            ) {
                30
            } else {
                5
            },
        ),
        process_id: None,
    })
}

fn handle_start(body: &str, request_id: Option<String>) -> (u16, String) {
    let _operation = match CONTROL.lock() {
        Ok(lock) => lock,
        Err(_) => return (500, r#"{"error":"service operation unavailable"}"#.into()),
    };
    if STOP.load(Ordering::SeqCst) {
        return (503, r#"{"error":"service stopping"}"#.into());
    }
    if let Some(id) = request_id.as_deref() {
        if uuid::Uuid::parse_str(id).is_err() {
            return (400, r#"{"error":"invalid runtime request ID"}"#.into());
        }
        let mut guard = match CHILD.lock() {
            Ok(guard) => guard,
            Err(_) => return (500, r#"{"error":"runtime owner unavailable"}"#.into()),
        };
        if let Some(child) = guard.as_mut() {
            match child.try_wait() {
                Ok(None) if child.session_id()==Some(id) => {
                    let snapshot=child.snapshot();
                    return (200,serde_json::json!({"ok":true,"pid":child.id(),"sessionId":id,"engine":proxy_runtime::ENGINE_ID,"embedded":false,"snapshot":snapshot}).to_string());
                }
                Ok(None)=>return (409,r#"{"error":"another runtime is active; stop its owned session before starting"}"#.into()),
                Ok(Some(_))=>{},
                Err(_)=>return (500,r#"{"error":"runtime liveness unknown"}"#.into()),
            }
        }
    }
    let prepared = (|| -> anyhow::Result<(String, proxy_runtime::ExecutionPolicy)> {
        let request: StartRequest = serde_json::from_str(body)?;
        let config = service_protocol::sanitized_config(&request.config)?;
        let policy = match request.execution_policy {
            service_protocol::RuntimePolicy::Standard => proxy_runtime::ExecutionPolicy::Standard,
            service_protocol::RuntimePolicy::Protected => proxy_runtime::ExecutionPolicy::Protected,
        };
        proxy_runtime::validate_yaml_with_policy(&config, policy)?;
        if policy == proxy_runtime::ExecutionPolicy::Protected {
            protection::privacy::validate(&config)?;
        }
        let directory = service_protocol::runtime_dir();
        std::fs::create_dir_all(directory.join("providers"))?;
        use std::os::windows::fs::MetadataExt;
        if std::fs::symlink_metadata(&directory)?.file_attributes() & 0x400 != 0 {
            anyhow::bail!("服务目录不能是重解析点");
        }
        config::atomic_write(&directory.join("config.yaml"), config.as_bytes())?;
        Ok((config, policy))
    })();
    let (config, policy) = match prepared {
        Ok(value) => value,
        Err(error) => {
            return (
                422,
                serde_json::json!({"error":format!("云阔mihomo 配置校验失败：{error:#}")})
                    .to_string(),
            );
        }
    };
    if kill_core_inner(None, None).is_err() {
        return (
            500,
            r#"{"error":"previous runtime cleanup unconfirmed"}"#.into(),
        );
    }
    let result = runtime::block_on(proxy_runtime::Runtime::start_with_policy(
        proxy_runtime::StartOptions {
            yaml: config,
            work_dir: service_protocol::runtime_dir(),
        },
        policy,
    ));
    match result {
        Ok(native) => {
            let snapshot = native.snapshot();
            if let Err(mut native) = runtime::publish_if_running(
                &CHILD,
                &STOP,
                ManagedCore::with_session(native, request_id.clone()),
            ) {
                if native.kill().is_err() {
                    return (
                        500,
                        r#"{"error":"startup cancellation cleanup unconfirmed"}"#.into(),
                    );
                }
                return (
                    503,
                    r#"{"error":"service stopping or runtime owner unavailable"}"#.into(),
                );
            }
            (200,serde_json::json!({"ok":true,"pid":snapshot.pid,"sessionId":request_id,"engine":proxy_runtime::ENGINE_ID,"embedded":false,"snapshot":snapshot}).to_string())
        }
        Err(error) => (
            422,
            serde_json::json!({"error":format!("云阔mihomo 代理启动失败：{error:#}")}).to_string(),
        ),
    }
}

fn run_http() {
    let server = match tiny_http::Server::http(format!("127.0.0.1:{PORT}")) {
        Ok(s) => s,
        Err(e) => {
            log(&format!("http bind failed: {e}"));
            return;
        }
    };
    log(&format!("http listening on {PORT}"));

    while !STOP.load(Ordering::SeqCst) {
        let mut req = match server.recv_timeout(Duration::from_millis(250)) {
            Ok(Some(req)) => req,
            Ok(None) => continue,
            Err(_) => break,
        };
        let path = req.url().split('?').next().unwrap_or("").to_string();
        let method = req.method().as_str().to_string();

        let header_token = req
            .headers()
            .iter()
            .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case("x-token"))
            .map(|h| h.value.as_str().to_string());
        let token_ok = header_token.is_some() && header_token == read_token();

        let mut body = String::new();
        let declared = req
            .headers()
            .iter()
            .find(|h| {
                h.field
                    .as_str()
                    .as_str()
                    .eq_ignore_ascii_case("content-length")
            })
            .and_then(|h| h.value.as_str().parse::<usize>().ok())
            .unwrap_or(0);
        let body_ok = if token_ok && declared <= service_protocol::MAX_REQUEST_BYTES {
            use std::io::Read;
            req.as_reader()
                .take((service_protocol::MAX_REQUEST_BYTES + 1) as u64)
                .read_to_string(&mut body)
                .is_ok()
                && body.len() <= service_protocol::MAX_REQUEST_BYTES
        } else {
            false
        };
        let (code, json) = if !token_ok {
            (401, r#"{"error":"unauthorized"}"#.to_string())
        } else if !body_ok {
            (
                413,
                r#"{"error":"request too large or unreadable"}"#.to_string(),
            )
        } else {
            match (method.as_str(), path.as_str()) {
                ("GET", "/ping") => (
                    200,
                    serde_json::json!({"ok":true,"service":"yunkuo-service","version":service_protocol::SERVICE_VERSION,"nativeProxy":false,"mihomoProxy":true,"engine":proxy_runtime::ENGINE_ID,"sessionOwnership":true}).to_string(),
                ),
                ("POST", "/start") => {
                    let request_id=req.headers().iter().find(|h|h.field.as_str().as_str().eq_ignore_ascii_case("x-runtime-request-id")).map(|h|h.value.as_str().to_owned());
                    handle_start(&body,request_id)
                },

                ("GET", "/privacy/status") => match privacy_service::status() {
                    Ok(status) => status_response(status),
                    Err(error) => (500, serde_json::json!({"error":format!("{error:#}")}).to_string()),
                },
                ("POST", "/privacy/arm") => match (|| -> anyhow::Result<privacy_service::Status> {
                    let _operation=CONTROL.lock().map_err(|_|anyhow::anyhow!("服务操作锁失效"))?;
                    if STOP.load(Ordering::SeqCst) {anyhow::bail!("服务正在停止，保护未重新放行");}
                    let mut child = CHILD.lock().map_err(|_| anyhow::anyhow!("内核状态锁失效"))?;
                    if !child.as_mut().is_some_and(|c| c.supports_strict_protection() && c.try_wait().ok() == Some(None)) { anyhow::bail!("请先启动使用受保护执行策略的服务 TUN 运行时"); }
                    drop(child);
                    privacy_service::arm()
                })() {
                    Ok(status) => status_response(status),
                    Err(error) => (422, serde_json::json!({"error":format!("{error:#}；防泄露阻断保留，可重新启用保护或在设置中解除")}).to_string()),
                },
                ("POST", "/privacy/release") => match (|| -> anyhow::Result<()> {
                    let _operation=CONTROL.lock().map_err(|_|anyhow::anyhow!("服务操作锁失效"))?;
                    privacy_service::release()
                })() {
                    Ok(()) => (200, r#"{"ok":true}"#.into()),
                    Err(error) => (500, serde_json::json!({"error":format!("{error:#}")}).to_string()),
                },
                ("POST", "/stop") => handle_stop(&body),
                ("GET", "/status") => {
                    match CHILD.lock() {
                        Ok(mut guard) => {
                            if guard.as_mut().is_some_and(|child| child.try_wait().ok().flatten().is_some()) { guard.take(); }
                            let running = guard.is_some();
                            let pid = guard.as_ref().map(|c| c.id()).unwrap_or(0);
                            let engine=guard.as_ref().map(|c|c.engine());
                            let session_id=guard.as_ref().and_then(|c|c.session_id());
                            (200, serde_json::json!({"running":running,"pid":pid,"sessionId":session_id,"engine":engine,"embedded":false}).to_string())
                        }
                        Err(_) => (500, r#"{"error":"core state unavailable"}"#.into()),
                    }
                }
                _ => (404, r#"{"error":"not found"}"#.to_string()),
            }
        };
        let mut response = tiny_http::Response::from_string(json).with_status_code(code);
        if let Ok(header) = tiny_http::Header::from_bytes("Content-Type", "application/json") {
            response = response.with_header(header);
        }
        let _ = req.respond(response);
    }
    log("http stopped");
}

fn status_response(status: privacy_service::Status) -> (u16, String) {
    match serde_json::to_string(&status) {
        Ok(json) => (200, json),
        Err(_) => (500, r#"{"error":"status serialization failed"}"#.into()),
    }
}

fn main() {
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("--self-test-crash-journal"))
    {
        if let Some(root) = std::env::args_os().nth(2) {
            faults::panic_probe(root.into());
        }
        std::process::exit(2);
    }
    faults::install(data_dir().join("diagnostics"), "service");
    // 应急恢复：服务无法启动时，由用户在管理员终端明确执行；正常启动不进入此分支。
    if std::env::args().any(|a| a == "--release-network-lock") {
        match network_lock::NetworkLock::open()
            .and_then(|lock| lock.release())
            .and_then(|_| recover_native_dns())
        {
            Ok(()) => std::process::exit(0),
            Err(_) => std::process::exit(1),
        }
    }
    // 卸载：由安装器清理流程在停止并删除服务前以管理员身份执行，连同子层和提供程序一起移除。
    if std::env::args().any(|a| a == "--remove-network-lock") {
        match network_lock::NetworkLock::open()
            .and_then(|lock| lock.remove())
            .and_then(|_| recover_native_dns())
        {
            Ok(()) => std::process::exit(0),
            Err(_) => std::process::exit(1),
        }
    }
    faults::start();
    // 以服务方式运行；在 SCM 之外直接双击运行时给出提示
    if let Err(e) = service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
        eprintln!("本程序是 Windows 服务组件，请通过「设置 → TUN 服务模式」安装后使用。");
        eprintln!("直接运行错误: {e}");
        std::process::exit(1);
    }
    faults::finish();
}

fn recover_native_dns() -> anyhow::Result<()> {
    let directory = service_protocol::runtime_dir();
    if directory.is_dir() {
        proxy_runtime::recover_dns(&directory)?;
    }
    Ok(())
}
