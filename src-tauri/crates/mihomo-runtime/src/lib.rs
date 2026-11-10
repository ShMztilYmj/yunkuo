//! The application and service own a real, pinned mihomo process. No embedded
//! protocol implementation, shared global resolver or native controller facade.
use anyhow::{Context, Result, bail};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    net::{Ipv4Addr, SocketAddr, TcpListener},
    os::windows::{
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::HANDLE,
    System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    },
};

pub const ENGINE_ID: &str = "mihomo";
pub const ENGINE_VERSION: &str = "v1.19.32";
pub const ENGINE_SHA256: &str = "04F8D7FC2B314771E1ECBF15951D59F0E1CB7914503C88CFB6972FF6A824E314";
mod dns_recovery;
pub use dns_recovery::recover_dns;
mod listener_owner;
pub use listener_owner::{
    listener_owned_by_engine, process_image, tcp_listener_owner, tcp_listener_owners, terminate,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ExecutionPolicy {
    Standard,
    Protected,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Phase {
    Starting,
    Ready,
    Stopping,
    Stopped,
    Failed,
}
#[derive(Clone)]
pub struct StartOptions {
    pub yaml: String,
    pub work_dir: PathBuf,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub phase: Phase,
    pub execution_policy: ExecutionPolicy,
    pub engine_version: &'static str,
    pub pid: u32,
    pub listeners: Vec<SocketAddr>,
    pub controller: Option<SocketAddr>,
    pub tun_ready: bool,
    pub error: Option<String>,
}
struct Job(OwnedHandle);
impl Job {
    fn own(child: &Child) -> Result<Self> {
        unsafe {
            let raw = CreateJobObjectW(None, None)?;
            let job = Self(OwnedHandle::from_raw_handle(raw.0));
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                raw,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as _,
                std::mem::size_of_val(&limits) as u32,
            )?;
            AssignProcessToJobObject(HANDLE(job.0.as_raw_handle()), HANDLE(child.as_raw_handle()))?;
            Ok(job)
        }
    }
}
pub struct Runtime {
    child: Mutex<Option<Child>>,
    _job: Job,
    state: Mutex<Snapshot>,
    secret: String,
    /// 子进程退出时触发。等待方是 Windows 句柄线程，不是定时器。
    exit: Arc<tokio::sync::Notify>,
}

impl Runtime {
    /// 内核退出信号。`notify_one` 会保留一次许可，因此“先退出后等待”不会丢事件。
    pub fn exit_signal(&self) -> Arc<tokio::sync::Notify> {
        self.exit.clone()
    }
}

/// 用进程句柄等待替代轮询：内核一退出就唤醒监督者，空闲时没有周期开销。
fn watch_exit(pid: u32, exit: Arc<tokio::sync::Notify>) {
    std::thread::spawn(move || {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{
            INFINITE, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
        };
        // 句柄拿不到时重试：直接放行会和“真的退出了”混为一谈。
        let mut handle = Err(());
        for attempt in 0..5u64 {
            handle = (unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) }).map_err(|_| ());
            if handle.is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50 * (attempt + 1)));
        }
        let Ok(handle) = handle else {
            // 五次都打不开句柄，只能放行让监督者用 try_wait 复核。
            exit.notify_one();
            return;
        };
        unsafe { WaitForSingleObject(handle, INFINITE) };
        let _ = unsafe { CloseHandle(handle) };
        exit.notify_one();
    });
}

pub fn engine_path() -> Result<PathBuf> {
    let current = std::env::current_exe()?;
    let parent = current.parent().context("无法定位程序目录")?;
    let installed = parent.join("云阔mihomo.exe");
    if installed.is_file() {
        return Ok(installed);
    }
    // Cargo test executables live in release/deps, alongside the packaged host.
    if parent.file_name() == Some(std::ffi::OsStr::new("deps")) {
        if let Some(root) = parent.parent() {
            let executable = root.join("云阔mihomo.exe");
            if executable.is_file() {
                return Ok(executable);
            }
        }
    }
    #[cfg(debug_assertions)]
    {
        let development = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../binaries/云阔mihomo-x86_64-pc-windows-msvc.exe");
        if development.is_file() {
            return Ok(development);
        }
    }
    bail!("mihomo 内核缺失，请安装完整软件包")
}
/// 内核文件在本进程内的校验指纹：长度 + 修改时间 + 创建时间。
/// 同一份文件不重复读 60 MB 重算摘要；换包（升级或替换）后指纹变化，会重新校验。
type EngineStamp = (u64, u64, u64);
static VERIFIED_ENGINE: std::sync::LazyLock<
    std::sync::Mutex<Option<(std::path::PathBuf, EngineStamp)>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

fn engine_stamp(metadata: &fs::Metadata) -> EngineStamp {
    use std::os::windows::fs::MetadataExt;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |value| value.as_nanos() as u64);
    (metadata.len(), modified, metadata.creation_time())
}

pub fn verify_engine(path: &Path) -> Result<()> {
    use std::os::windows::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_attributes() & 0x400 != 0 {
        bail!("mihomo 内核不能是重解析点");
    }
    let stamp = engine_stamp(&metadata);
    let mut verified = VERIFIED_ENGINE
        .lock()
        .map_err(|_| anyhow::anyhow!("内核校验缓存不可用"))?;
    if verified
        .as_ref()
        .is_some_and(|(cached, cached_stamp)| cached == path && *cached_stamp == stamp)
    {
        return Ok(());
    }
    let digest = ring::digest::digest(&ring::digest::SHA256, &fs::read(path)?);
    let hash: String = digest.as_ref().iter().map(|b| format!("{b:02X}")).collect();
    if hash != ENGINE_SHA256 {
        bail!("mihomo 内核完整性校验失败，请重新安装完整软件包");
    }
    *verified = Some((path.to_path_buf(), stamp));
    Ok(())
}
/// 配置校验结果的指纹：配置字节 + 内核二进制 + geodata 元数据。
/// 内核、配置或地理数据任一变化都会重新校验；三者都不变时不必再起一个
/// mihomo 进程做 `-t`。
fn validation_fingerprint(directory: &Path, rendered: &str) -> String {
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    context.update(ENGINE_SHA256.as_bytes());
    context.update(rendered.as_bytes());
    for name in [
        "Country.mmdb",
        "ASN.mmdb",
        "geosite.dat",
        "GeoIP.dat",
        "geoip.metadb",
    ] {
        context.update(name.as_bytes());
        match fs::metadata(directory.join(name)) {
            Ok(metadata) => {
                context.update(&metadata.len().to_le_bytes());
                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|elapsed| elapsed.as_secs())
                    .unwrap_or(0);
                context.update(&modified.to_le_bytes());
            }
            // 缺失也要进指纹：补上文件之后必须重新校验。
            Err(_) => context.update(&[0u8]),
        }
    }
    context
        .finish()
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect()
}

pub fn validate_yaml(yaml: &str) -> Result<()> {
    let value: serde_yaml::Value = serde_yaml::from_str(yaml)?;
    if !value.is_mapping() {
        bail!("代理配置必须为 YAML 映射");
    }
    Ok(())
}
pub fn validate_yaml_with_policy(yaml: &str, policy: ExecutionPolicy) -> Result<()> {
    let value: serde_yaml::Value = serde_yaml::from_str(yaml)?;
    check_value(&value, policy)
}

/// 一次校验 + 一次序列化。调用方已持有配置树时走这里，避免
/// “解析→序列化→再解析”的重复成本；返回可直接交给内核的 YAML 文本。
pub fn validate_and_render(value: &serde_yaml::Value, policy: ExecutionPolicy) -> Result<String> {
    check_value(value, policy)?;
    Ok(serde_yaml::to_string(value)?)
}

fn check_value(value: &serde_yaml::Value, _policy: ExecutionPolicy) -> Result<()> {
    if !value.is_mapping() {
        bail!("代理配置必须为 YAML 映射");
    }
    Ok(())
}

fn vacant_address() -> Result<SocketAddr> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?)
}
fn normalize_test_ports(value: &mut serde_yaml::Value) -> Result<()> {
    if value["external-controller"]
        .as_str()
        .is_some_and(|s| s.ends_with(":0"))
    {
        value["external-controller"] = vacant_address()?.to_string().into();
    }
    if let Some(listeners) = value["listeners"].as_sequence_mut() {
        for listener in listeners {
            if listener["port"].as_u64() == Some(0) {
                listener["port"] = vacant_address()?.port().into();
            }
        }
    }
    Ok(())
}
impl Runtime {
    pub async fn start(options: StartOptions) -> Result<Self> {
        Self::start_with_policy(options, ExecutionPolicy::Standard).await
    }
    pub async fn start_with_policy(options: StartOptions, policy: ExecutionPolicy) -> Result<Self> {
        // 强制结束内核后 TUN 网卡可能留下幽灵设备（SDW\Wintun 实例），先清一次。
        let tun_requested = tun_enabled(&options.yaml);
        if tun_requested {
            cleanup_stale_wintun();
        }
        match Self::start_once(options.clone(), policy).await {
            Ok(runtime) => Ok(runtime),
            Err(_error) if tun_requested => {
                // 只有 TUN 没起来时才值得再试一次：清掉残留设备后通常就能新建网卡。
                cleanup_stale_wintun();
                Self::start_once(options, policy).await.map_err(|error| {
                    error.context("TUN 网卡可能被上次异常退出留下；已尝试清理残留设备，仍失败请重启电脑后再启动云阔")
                })
            }
            Err(error) => Err(error),
        }
    }

    async fn start_once(options: StartOptions, policy: ExecutionPolicy) -> Result<Self> {
        validate_yaml(&options.yaml)?;
        let executable = engine_path()?;
        verify_engine(&executable)?;
        recover_dns(&options.work_dir)?;
        // Isolate mihomo's BoltDB cache from the former runtime's SQLite cache.db.
        let directory = options.work_dir.join("mihomo");
        fs::create_dir_all(directory.join("providers"))?;
        for name in [
            "Country.mmdb",
            "ASN.mmdb",
            "geosite.dat",
            "GeoIP.dat",
            "geoip.metadb",
        ] {
            let source = options.work_dir.join(name);
            let target = directory.join(name);
            if source.is_file() && !target.exists() {
                fs::copy(source, target)?;
            }
        }
        let mut value: serde_yaml::Value = serde_yaml::from_str(&options.yaml)?;
        normalize_test_ports(&mut value)?;
        let controller: SocketAddr = value["external-controller"]
            .as_str()
            .context("未配置本机控制器")?
            .parse()?;
        if !controller.ip().is_loopback() {
            bail!("控制器只能监听本机回环地址");
        }
        let secret = value["secret"].as_str().unwrap_or("").to_string();
        if secret.len() < 16 {
            bail!("控制器需要至少 16 字节的认证密钥");
        }
        let tun_requested = value["tun"]["enable"].as_bool() == Some(true);
        let tun_device = value["tun"]["device"]
            .as_str()
            .unwrap_or("Mihomo")
            .to_owned();
        let mut listeners = vec![];
        if let Some(port) = value["mixed-port"].as_u64().filter(|p| *p > 0) {
            listeners.push(SocketAddr::from(([127, 0, 0, 1], port as u16)));
        }
        if let Some(rows) = value["listeners"].as_sequence() {
            for row in rows {
                if !matches!(row["type"].as_str(), Some("mixed" | "http" | "socks")) {
                    continue;
                }
                if let Some(port) = row["port"].as_u64() {
                    listeners.push(SocketAddr::from(([127, 0, 0, 1], port as u16)));
                }
            }
        }
        let owned_addresses: Vec<_> = listeners.iter().copied().chain(Some(controller)).collect();
        for (address, owner) in owned_addresses
            .iter()
            .zip(tcp_listener_owners(&owned_addresses)?)
        {
            let Some(owner) = owner else { continue };
            // 上一轮异常退出留下的自家内核：直接回收，用户不该看到“端口被占用”。
            if listener_owner::listener_owned_by_engine(*address, &executable) {
                let _ = listener_owner::terminate(owner);
                let started = Instant::now();
                while started.elapsed() < Duration::from_secs(3)
                    && matches!(tcp_listener_owner(*address), Ok(Some(current)) if current == owner)
                {
                    std::thread::sleep(Duration::from_millis(100));
                }
                if matches!(tcp_listener_owner(*address), Ok(Some(current)) if current == owner) {
                    bail!(
                        "云阔mihomo 的上一轮内核（进程 {owner}）未能退出，请重启电脑后再启动云阔"
                    );
                }
                continue;
            }
            bail!(
                "本机端口 {address} 已被进程 {owner} 占用，请调整云阔端口或先停止占用程序；系统代理尚未接管"
            );
        }
        let config = directory.join("config.yaml");
        let rendered = serde_yaml::to_string(&value)?;
        fs::write(&config, &rendered)?;
        // 同一份配置配上同一个内核版本，`mihomo -t` 的结论不会变。以前每次启动
        // 都为此多起一个 mihomo 进程，自动启动和模式切换时这段是纯等待。
        let fingerprint = validation_fingerprint(&directory, &rendered);
        let stamp = directory.join("validation.stamp");
        let stale = std::fs::read_to_string(&stamp)
            .map(|value| value.trim().to_owned())
            .ok()
            .as_deref()
            != Some(fingerprint.as_str());
        if stale {
            let checked = Command::new(&executable)
                .args(["-t", "-d"])
                .arg(&directory)
                .arg("-f")
                .arg(&config)
                .creation_flags(0x0800_0000)
                .output()?;
            if !checked.status.success() {
                // Raw config and node credentials must not cross the service/UI boundary.
                fs::write(
                    directory.join("validation.log"),
                    [&checked.stdout[..], &checked.stderr[..]].concat(),
                )?;
                bail!("云阔mihomo 配置校验失败，请查看运行目录的 validation.log");
            }
            let _ = fs::write(&stamp, &fingerprint);
        }
        let logfile = OpenOptions::new()
            .create(true)
            .append(true)
            .open(directory.join("core.log"))?;
        let mut child = Command::new(&executable)
            .arg("-d")
            .arg(&directory)
            .arg("-f")
            .arg(&config)
            .stdout(Stdio::from(logfile.try_clone()?))
            .stderr(Stdio::from(logfile))
            .creation_flags(0x0800_0000)
            .spawn()?;
        let job = match Job::own(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.context("无法托管云阔mihomo 子进程"));
            }
        };
        let exit = Arc::new(tokio::sync::Notify::new());
        watch_exit(child.id(), exit.clone());
        let runtime = Self {
            state: Mutex::new(Snapshot {
                phase: Phase::Starting,
                execution_policy: policy,
                engine_version: ENGINE_VERSION,
                pid: child.id(),
                listeners,
                controller: Some(controller),
                tun_ready: false,
                error: None,
            }),
            child: Mutex::new(Some(child)),
            _job: job,
            secret,
            exit,
        };
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()?;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(30) {
            if runtime.snapshot().phase == Phase::Failed {
                bail!("云阔mihomo 启动后退出，请查看运行目录的 core.log");
            }
            let version = client
                .get(format!("http://{controller}/version"))
                .bearer_auth(&runtime.secret)
                .send()
                .await;
            if let Ok(response) = version {
                if let Ok(version) = response.json::<Value>().await {
                    if version["meta"] == true && version["version"].is_string() {
                        let snapshot = runtime.snapshot();
                        let listening = tcp_listener_owners(&owned_addresses).is_ok_and(|owners| {
                            owners.into_iter().all(|owner| owner == Some(snapshot.pid))
                        });
                        if !listening {
                            tokio::time::sleep(Duration::from_millis(150)).await;
                            continue;
                        }
                        let tun_ready = if tun_requested {
                            match client
                                .get(format!("http://{controller}/configs"))
                                .bearer_auth(&runtime.secret)
                                .send()
                                .await
                            {
                                Ok(response) => response.json::<Value>().await.is_ok_and(|v| {
                                    v["tun"]["enable"] == true && adapter_up(&tun_device)
                                }),
                                _ => false,
                            }
                        } else {
                            false
                        };
                        if !tun_requested || tun_ready {
                            let mut state = runtime.state.lock();
                            state.phase = Phase::Ready;
                            state.tun_ready = tun_ready;
                            drop(state);
                            return Ok(runtime);
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        bail!(
            "云阔mihomo 控制器或 TUN 未在 30 秒内就绪，请查看 core.log；若 TUN 反复失败，重启电脑后再启动云阔"
        );
    }
    pub fn snapshot(&self) -> Snapshot {
        if let Some(child) = self.child.lock().as_mut() {
            if let Ok(Some(status)) = child.try_wait() {
                let mut state = self.state.lock();
                state.phase = if status.success() {
                    Phase::Stopped
                } else {
                    Phase::Failed
                };
                state.error = Some(format!("云阔mihomo 已退出（{status}）"));
                state.tun_ready = false;
            }
        }
        self.state.lock().clone()
    }
    pub async fn stop(&mut self) -> Result<Snapshot> {
        self.state.lock().phase = Phase::Stopping;
        let controller = self.state.lock().controller;
        if let Some(controller) = controller {
            // Let mihomo tear down the TUN adapter/routes before terminating it.
            let client = reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(3))
                .build()?;
            let _ = client
                .patch(format!("http://{controller}/configs"))
                .bearer_auth(&self.secret)
                .json(&json!({"tun":{"enable":false}}))
                .send()
                .await;
        }
        if let Some(mut child) = self.child.lock().take() {
            if child.try_wait()?.is_none() {
                child.kill()?;
            }
            child.wait()?;
        }
        let mut state = self.state.lock();
        state.phase = Phase::Stopped;
        state.tun_ready = false;
        Ok(state.clone())
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.lock().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Compatibility seam for the isolated measurement host; all control uses the
/// real authenticated HTTP API rather than a process-local native router.
pub async fn local_request(
    address: &str,
    secret: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Option<Result<Value>> {
    Some(
        async {
            let client = reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(15))
                .build()?;
            let mut request = client
                .request(
                    reqwest::Method::from_bytes(method.as_bytes())?,
                    format!("http://{address}{path}"),
                )
                .bearer_auth(secret);
            if let Some(body) = body {
                request = request.json(&body);
            }
            let response = request.send().await?.error_for_status()?;
            let bytes = response.bytes().await?;
            Ok(if bytes.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&bytes)?
            })
        }
        .await,
    )
}

pub async fn bootstrap_ipv4(host: &str) -> Result<Vec<Ipv4Addr>> {
    let response: Value = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(8))
        .build()?
        .get("https://1.1.1.1/dns-query")
        .header("Accept", "application/dns-json")
        .query(&[("name", host), ("type", "A")])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(response["Answer"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["type"] == 1)
        .filter_map(|r| r["data"].as_str()?.parse().ok())
        .collect())
}
/// 配置是否要求启用 TUN。只做一次轻量解析，失败按“否”处理。
fn tun_enabled(yaml: &str) -> bool {
    serde_yaml::from_str::<serde_yaml::Value>(yaml)
        .ok()
        .and_then(|value| value["tun"]["enable"].as_bool())
        .unwrap_or(false)
}

/// 清掉 Wintun 留下的非在线设备实例。
///
/// 内核被强制结束后，`云阔TUN` 可能只剩一条“幽灵设备”：设备实例还在，
/// 驱动绑定已经断了。此时再新建网卡会得到“系统找不到指定的文件”，
/// 而普通重启内核也修不好，只有删掉残留实例才能重建。返回是否清理过。
fn cleanup_stale_wintun() -> bool {
    let pnputil = r"C:\Windows\System32\pnputil.exe";
    let Ok(listed) = Command::new(pnputil)
        .args(["/enum-devices", "/class", "Net", "/disconnected"])
        .creation_flags(0x0800_0000)
        .output()
    else {
        return false;
    };
    let text = String::from_utf8_lossy(&listed.stdout);
    let mut removed = false;
    for line in text.lines() {
        let Some(instance) = line.trim().strip_prefix("Instance ID:") else {
            continue;
        };
        let instance = instance.trim();
        if !instance.starts_with("SWD\\Wintun") && !instance.starts_with("ROOT\\WINTUN") {
            continue;
        }
        if Command::new(pnputil)
            .args(["/remove-device", instance])
            .creation_flags(0x0800_0000)
            .output()
            .is_ok_and(|result| result.status.success())
        {
            removed = true;
        }
    }
    removed
}

pub fn adapter_up(device: &str) -> bool {
    use windows::Win32::NetworkManagement::{
        IpHelper::{ConvertInterfaceAliasToLuid, GetIfEntry2, MIB_IF_ROW2},
        Ndis::{IfOperStatusUp, NET_LUID_LH},
    };
    let name: Vec<u16> = device.encode_utf16().chain(Some(0)).collect();
    let mut luid = NET_LUID_LH::default();
    if unsafe { ConvertInterfaceAliasToLuid(windows::core::PCWSTR(name.as_ptr()), &mut luid) }.0
        != 0
    {
        return false;
    }
    let mut row = MIB_IF_ROW2 {
        InterfaceLuid: luid,
        ..Default::default()
    };
    unsafe { GetIfEntry2(&mut row) }.0 == 0 && row.OperStatus == IfOperStatusUp
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn occupied_mixed_port_cannot_report_ready() {
        let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = occupied.local_addr().unwrap().port();
        let controller = vacant_address().unwrap();
        let root =
            std::env::temp_dir().join(format!("mihomo-port-conflict-test-{}", std::process::id()));
        let result=Runtime::start(StartOptions {
            yaml:format!("mixed-port: {port}\nexternal-controller: {controller}\nsecret: fixture-controller-secret-32\ndns: {{enable: false}}\nrules: ['MATCH,DIRECT']\n"),
            work_dir:root.clone()
        }).await;
        if let Ok(mut runtime) = result {
            runtime.stop().await.unwrap();
            panic!("foreign process owns mixed port, but mihomo was reported Ready");
        }
        let _ = fs::remove_dir_all(root);
    }
    #[tokio::test]
    async fn real_mihomo_process_proxies_http_and_releases_ports() {
        use std::io::{Read, Write};
        let root = std::env::temp_dir().join(format!("mihomo-runtime-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let mixed = vacant_address().unwrap();
        let control = vacant_address().unwrap();
        let origin = TcpListener::bind("127.0.0.1:0").unwrap();
        let target = origin.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = origin.accept().unwrap();
            let _ = socket.read(&mut [0; 4096]);
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nmihomo!",
                )
                .unwrap();
        });
        let yaml = format!(
            "mixed-port: {}\nexternal-controller: {}\nsecret: fixture-controller-secret-32\ndns: {{enable: false}}\nrules: ['MATCH,DIRECT']\n",
            mixed.port(),
            control
        );
        let mut runtime = Runtime::start(StartOptions {
            yaml,
            work_dir: root.clone(),
        })
        .await
        .unwrap();
        assert_ne!(runtime.snapshot().pid, std::process::id());
        assert_eq!(runtime.snapshot().phase, Phase::Ready);
        let client = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(format!("http://{mixed}")).unwrap())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        assert_eq!(
            client
                .get(format!("http://{target}/"))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "mihomo!"
        );
        server.join().unwrap();
        runtime.stop().await.unwrap();
        assert!(TcpListener::bind(mixed).is_ok());
        drop(runtime);
        let _ = fs::remove_dir_all(root);
    }
}
