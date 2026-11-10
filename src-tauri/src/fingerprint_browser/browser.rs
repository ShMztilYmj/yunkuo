use crate::platform::files::{self, environment_root as root, error};
use crate::platform::state::EnvironmentOperation as Operation;
use anyhow::{Result, anyhow, bail};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    os::windows::process::CommandExt,
    path::PathBuf,
    process::Command,
    sync::{Arc, OnceLock},
};
use tauri::{Emitter, Manager};

/// 由应用编排提供真实网络和环境检查；浏览器不依赖检测实现或原始内核 API。
#[derive(Clone, Default)]
pub(crate) struct NetworkEvidence {
    pub(crate) mixed_port: u16,
}

pub(crate) trait LaunchGate: Send + Sync {
    fn authorize<'a>(
        &'a self,
        app: &'a tauri::AppHandle,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<NetworkEvidence, String>> + Send + 'a>,
    >;
    fn profile_slot(&self, app: &tauri::AppHandle) -> Result<String, String>;
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub seed: u32,
    pub language: String,
    pub timezone: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            seed: 495574189,
            language: "en-US".into(),
            timezone: "America/New_York".into(),
        }
    }
}

// 浏览器支持的是引擎启动组合；系统修复预设由 diagnostics 独立管理。
fn presets() -> Value {
    json!([
        {"id":"US","language":"en-US","timezone":"America/New_York","windowsTimezone":"Eastern Standard Time"},
        {"id":"US-WEST","language":"en-US","timezone":"America/Los_Angeles","windowsTimezone":"Pacific Standard Time"},
        {"id":"GB","language":"en-GB","timezone":"Europe/London","windowsTimezone":"GMT Standard Time"},
        {"id":"CA","language":"en-CA","timezone":"America/Toronto","windowsTimezone":"Eastern Standard Time"},
        {"id":"SG","language":"en-SG","timezone":"Asia/Singapore","windowsTimezone":"Singapore Standard Time"},
        {"id":"AU","language":"en-AU","timezone":"Australia/Sydney","windowsTimezone":"AUS Eastern Standard Time"},
        {"id":"JP","language":"ja-JP","timezone":"Asia/Tokyo","windowsTimezone":"Tokyo Standard Time"},
        {"id":"KR","language":"ko-KR","timezone":"Asia/Seoul","windowsTimezone":"Korea Standard Time"},
        {"id":"DE","language":"de-DE","timezone":"Europe/Berlin","windowsTimezone":"W. Europe Standard Time"},
        {"id":"FR","language":"fr-FR","timezone":"Europe/Paris","windowsTimezone":"Romance Standard Time"},
        {"id":"TW","language":"zh-TW","timezone":"Asia/Taipei","windowsTimezone":"Taipei Standard Time"}
    ])
}
fn config(path: &std::path::Path, slot: &str) -> Result<Config> {
    if path.join("config.json").try_exists()? {
        return files::read(&path.join("config.json"));
    }
    let digest = ring::digest::digest(&ring::digest::SHA256, slot.as_bytes());
    Ok(Config {
        seed: u32::from_be_bytes(digest.as_ref()[..4].try_into().unwrap()).max(1),
        ..Default::default()
    })
}
fn validate(cfg: &Config) -> Result<()> {
    if cfg.seed == 0
        || !presets()
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["language"] == cfg.language && p["timezone"] == cfg.timezone)
    {
        bail!("请选择支持的语言与时区组合，指纹种子必须为非零整数")
    };
    Ok(())
}
pub(crate) const HOME_URL: &str = "https://ippure.com/";

#[derive(Default)]
pub struct Runtime {
    /// 用户资料实例；同一资料目录只允许一个。
    child: Mutex<Option<std::process::Child>>,
    /// 自检用的临时实例，跑在独立资料目录，不参与上面的单实例判断。
    selfcheck: Mutex<Option<std::process::Child>>,
    pending: Mutex<Pending>,
    notice: Mutex<Option<String>>,
    gate: OnceLock<Arc<dyn LaunchGate>>,
}
#[derive(Default)]
struct Pending {
    slot: String,
    urls: Vec<String>,
}
impl Pending {
    fn push(&mut self, slot: &str, url: String) -> Result<(), String> {
        if self.slot != slot {
            self.urls.clear();
            self.slot = slot.into();
        }
        if self.urls.contains(&url) {
            return Ok(());
        }
        if self.urls.len() >= 16 {
            return Err("待处理链接已达 16 个，请先处理后重试".into());
        }
        self.urls.push(url);
        Ok(())
    }
}
impl Runtime {
    pub(crate) fn invalidate(&self) {
        *self.pending.lock() = Pending::default();
        *self.notice.lock() = None;
        stop_slot(&self.selfcheck, false);
    }

    pub(crate) fn install_gate(&self, gate: Arc<dyn LaunchGate>) -> Result<(), String> {
        self.gate
            .set(gate)
            .map_err(|_| "浏览器启动策略已装配".into())
    }

    fn gate(&self) -> Result<&Arc<dyn LaunchGate>, String> {
        self.gate
            .get()
            .ok_or_else(|| "浏览器启动策略未就绪，请重启应用".into())
    }

    fn live_child(&self) -> bool {
        alive_slot(&self.child)
    }

    /// 结束用户资料的浏览器。定制内核的单实例转发会崩在 CHECK 上
    /// （0x80000003），所以这里主动收口：先关掉旧窗口，再带着新地址启动。
    fn stop_user(&self) {
        stop_slot(&self.child, true);
    }

    /// 自检窗口是临时的，重新自检、回传完成、换身份或打开正式浏览器时都可以直接结束。
    pub(crate) fn stop_selfcheck(&self) {
        stop_slot(&self.selfcheck, false);
    }
}

fn alive_slot(slot: &Mutex<Option<std::process::Child>>) -> bool {
    slot.lock()
        .as_mut()
        .is_some_and(|child| matches!(child.try_wait(), Ok(None)))
}

fn stop_slot(slot: &Mutex<Option<std::process::Child>>, graceful: bool) {
    let Some(mut child) = slot.lock().take() else {
        return;
    };
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    if !graceful {
        // 临时窗口（自检）直接结束，不值得为它等一轮关闭流程。
        let _ = child.kill();
        let _ = child.wait();
        return;
    }
    // 先让内核自己正常退出：直接 TerminateProcess 会被记成崩溃，
    // 下次启动就弹“恢复页面”气泡，等于给指纹面多画了一笔。
    let pid = child.id().to_string();
    let _ = Command::new("taskkill")
        .args(["/PID", &pid, "/T"])
        .creation_flags(0x08000000)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1500);
    while std::time::Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

// 注册表传来的 URL 只作数据；不交给 shell，也不接受引擎参数或本地文件。
fn validate_url(value: &str) -> Result<String, String> {
    if value.len() > 8192 || value.chars().any(|c| c.is_control() || c == '"') {
        return Err("链接格式无效或过长，仅支持 HTTP/HTTPS 网页".into());
    }
    let url = reqwest::Url::parse(value).map_err(|_| "链接格式无效")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("仅支持不含账号密码的 HTTP/HTTPS 网页链接".into());
    }
    Ok(url.to_string())
}
pub fn url_argument(args: &[String]) -> Result<Option<String>, String> {
    if !args.iter().skip(1).any(|a| a == "--browser-url") {
        return Ok(None);
    }
    if args.len() != 3 || args[1] != "--browser-url" {
        return Err("浏览器入口参数无效，请重新打开链接".into());
    }
    validate_url(&args[2]).map(Some)
}
#[tauri::command]
pub fn browser_request_status(app: tauri::AppHandle) -> Value {
    let runtime = app.state::<Runtime>();
    json!({"pendingCount":runtime.pending.lock().urls.len(),"notice":runtime.notice.lock().clone()})
}
fn scope(app: &tauri::AppHandle) -> Result<(String, PathBuf)> {
    let base = root(app)?;
    let slot = app
        .state::<Runtime>()
        .gate()
        .map_err(anyhow::Error::msg)?
        .profile_slot(app)
        .map_err(|e| anyhow!(e))?;
    let path = base.join("browser-profiles").join(&slot);
    files::safe(&path)?;
    Ok((slot, path))
}
pub(crate) fn engine(app: &tauri::AppHandle) -> Option<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(debug_assertions)]
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/fingerprint-chromium"));
    if let Ok(resource) = app.path().resource_dir() {
        roots.insert(0, resource.join("resources/fingerprint-chromium"));
        roots.insert(1, resource.join("fingerprint-chromium"));
    }
    roots
        .into_iter()
        .map(|p| p.join("chrome.exe"))
        .find(|p| p.is_file())
}
/// 随包发布的扩展目录：`--load-extension` 需要指向含 manifest.json 的文件夹。
pub(crate) fn extensions(app: &tauri::AppHandle) -> Option<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(debug_assertions)]
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/extensions"));
    if let Ok(resource) = app.path().resource_dir() {
        roots.insert(0, resource.join("resources/extensions"));
        roots.insert(1, resource.join("extensions"));
    }
    roots
        .into_iter()
        .map(|dir| dir.join("webrtc-control"))
        .find(|dir| dir.join("manifest.json").is_file())
}
pub fn ensure_closed(app: &tauri::AppHandle) -> Result<()> {
    let runtime = app.state::<Runtime>();
    if let Some(child) = runtime.child.lock().as_mut() {
        if child.try_wait()?.is_none() {
            bail!("请先关闭本软件的指纹浏览器窗口，再修改资料或切换身份")
        }
    }
    // 重启宿主后仍识别遗留的独立 Chromium，不能仅依赖内存 PID。
    if let Some(engine) = engine(app) {
        files::shell(r#"$ErrorActionPreference='Stop'; $p=@(Get-CimInstance Win32_Process -Filter "Name='chrome.exe'" | Where-Object { $_.ExecutablePath -ieq $env:COC_ENGINE }); if($p.Count){exit 9}"#,&[("COC_ENGINE",&engine.to_string_lossy())]).map_err(|_|anyhow!("请先关闭本软件的指纹浏览器，再执行此操作"))?;
    }
    Ok(())
}
fn arguments(
    cfg: &Config,
    profile: &std::path::Path,
    port: u16,
    extension: Option<&std::path::Path>,
    pages: &[String],
) -> Vec<String> {
    let mut args = vec![
        format!("--user-data-dir={}", profile.display()),
        "--profile-directory=Default".into(),
        format!("--fingerprint={}", cfg.seed),
        "--fingerprint-platform=windows".into(),
        "--fingerprint-brand=Chrome".into(),
        "--fingerprint-brand-version=148.0.7778.215".into(),
        format!(
            "--fingerprint-hardware-concurrency={}",
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
        ),
        "--disable-spoofing=font,audio,gpu".into(),
        format!("--lang={}", cfg.language),
        format!(
            "--accept-lang={},{}",
            cfg.language,
            cfg.language.split('-').next().unwrap()
        ),
        format!("--timezone={}", cfg.timezone),
        format!("--proxy-server=http://127.0.0.1:{port}"),
        "--proxy-bypass-list=127.0.0.1;localhost;[::1]".into(),
        "--force-webrtc-ip-handling-policy=disable_non_proxied_udp".into(),
        "--disable-non-proxied-udp".into(),
        "--disable-quic".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
    ];
    // 扩展在每次启动时以解包目录装载；缺少目录时静默跳过，不影响指纹参数。
    if let Some(extension) = extension {
        args.push(format!("--load-extension={}", extension.display()));
    }
    args.push("--".into());
    args.extend(pages.iter().cloned());
    args
}
#[tauri::command]
pub async fn browser_environment_status(app: tauri::AppHandle) -> Result<Value, String> {
    let (slot, path) = scope(&app).map_err(error)?;
    let cfg = config(&path, &slot).map_err(error)?;
    // 默认值只在第一次保存时固定；仅查看状态不创建资料目录。
    let runtime = app.state::<Runtime>();
    // 旧 measurement 字段保留为空；检测信息由应用层复用 diagnostics 数据。
    Ok(
        json!({"installed":engine(&app).is_some(),"config":cfg,"slot":slot,"running":runtime.live_child(),"measurement":null}),
    )
}
#[tauri::command]
pub async fn browser_environment_save(
    app: tauri::AppHandle,
    mut config: Config,
    rotate: bool,
) -> Result<(), String> {
    let operation = app.state::<Operation>();
    let _operation = operation.lock().await;
    ensure_closed(&app).map_err(error)?;
    validate(&config).map_err(error)?;
    if rotate {
        config.seed = rand::random::<u32>().max(1)
    }
    let (_, path) = scope(&app).map_err(error)?;
    files::write(&path.join("config.json"), &config).map_err(error)?;
    app.state::<Runtime>().invalidate();
    Ok(())
}
async fn network(app: &tauri::AppHandle) -> Result<NetworkEvidence, String> {
    let gate = app.state::<Runtime>().gate()?.clone();
    gate.authorize(app).await
}
#[tauri::command]
pub async fn browser_environment_launch(
    app: tauri::AppHandle,
    open_claude: bool,
) -> Result<(), String> {
    let operation = app.state::<Operation>();
    let _operation = operation.lock().await;
    let network_operation = app.state::<crate::platform::state::ConfigUpdateMutex>();
    let _network_operation = network_operation.lock().await;
    launch_locked(
        &app,
        if open_claude {
            Target::Claude
        } else {
            Target::Home
        },
    )
    .await
}

enum Target {
    Home,
    Claude,
    Pending,
    /// 内部页面（自检）：URL 由本程序生成，不来自注册表或用户输入。
    Url(String),
}

/// 在指纹浏览器里打开一个本程序生成的页面；与正常启动共用同一套前置校验。
pub(crate) async fn launch_url(app: &tauri::AppHandle, url: &str) -> Result<(), String> {
    launch_locked(app, Target::Url(url.to_string())).await
}

#[tauri::command]
pub async fn browser_open_pending(app: tauri::AppHandle) -> Result<(), String> {
    let operation = app.state::<Operation>();
    let _operation = operation.lock().await;
    let network_operation = app.state::<crate::platform::state::ConfigUpdateMutex>();
    let _network_operation = network_operation.lock().await;
    let result = launch_locked(&app, Target::Pending).await;
    *app.state::<Runtime>().notice.lock() = Some(match &result {
        Ok(()) => "待处理链接已在当前身份的指纹浏览器中打开。".into(),
        Err(e) => e.clone(),
    });
    result
}

// 冷启动与单实例转发共用同一个入口，URL 不写入日志、配置或事件载荷。
pub async fn handle_arguments(app: tauri::AppHandle, args: Vec<String>) {
    let request = match url_argument(&args) {
        Ok(None) => return,
        other => other,
    };
    let operation = app.state::<Operation>();
    let _operation = operation.lock().await;
    let result = async {
        let url = request?.ok_or("没有待打开的链接")?;
        let (slot, _) = scope(&app).map_err(error)?;
        app.state::<Runtime>().pending.lock().push(&slot, url)?;
        let network_operation = app.state::<crate::platform::state::ConfigUpdateMutex>();
        let _network_operation = network_operation.lock().await;
        launch_locked(&app, Target::Pending).await?;
        Ok::<_, String>("外部链接已打开。".to_string())
    }
    .await;
    *app.state::<Runtime>().notice.lock() = Some(result.unwrap_or_else(|e| e));
    let _ = app.emit("browser-request", ());
    crate::show_main(&app);
}

// 调用方始终依次持有环境锁、网络配置锁。
async fn launch_locked(app: &tauri::AppHandle, target: Target) -> Result<(), String> {
    let (slot, path) = scope(app).map_err(error)?;
    let native = network(&app).await?;
    let browser =
        engine(&app).ok_or("未准备指纹浏览器引擎，请运行 pnpm browser:prepare 后重新打包")?;
    let config = config(&path, &slot).map_err(error)?;
    validate(&config).map_err(error)?;
    let runtime = app.state::<Runtime>();
    let urls = match &target {
        Target::Home => vec![HOME_URL.to_string()],
        Target::Claude => vec!["https://claude.ai/".to_string()],
        Target::Url(url) => vec![url.clone()],
        Target::Pending => {
            let pending = runtime.pending.lock();
            if pending.slot != slot || pending.urls.is_empty() {
                return Err("当前身份没有待处理链接".into());
            }
            pending.urls.clone()
        }
    };
    // 自检始终跑在独立资料目录：用户可能正开着浏览器，同一目录起第二个实例
    // 会触发定制内核的单实例转发崩溃（0x80000003）。
    let selfcheck = matches!(target, Target::Url(_));
    let profile = if selfcheck {
        path.join("selfcheck-profile")
    } else {
        path.join("profile")
    };
    if selfcheck {
        runtime.stop_selfcheck();
        if profile.try_exists().map_err(error)? {
            std::fs::remove_dir_all(&profile).map_err(error)?;
        }
    } else {
        // 单实例：先结束现有窗口，再带着本轮地址启动，绝不让两个进程抢同一个资料目录。
        runtime.stop_user();
        runtime.stop_selfcheck();
        // 宿主重启后仍检查遗留进程，避免与未知资料实例混用。
        ensure_closed(app).map_err(error)?;
    }
    files::write(&path.join("config.json"), &config).map_err(error)?;
    let extension = extensions(app);
    if !selfcheck {
        // 收藏栏与工具栏按钮都属于资料本身：先补齐，失败不影响启动。
        let _ = super::bookmarks::ensure(&profile);
        // 只在引擎未运行时写偏好：运行中的实例退出时会用内存里的副本覆盖文件。
        match extension.as_deref().map(super::pinned::id) {
            Some(Ok(id)) => {
                if let Err(error) = super::pinned::ensure(&profile, &id) {
                    log::warn!("固定随包扩展按钮失败：{error:#}");
                }
            }
            Some(Err(error)) => log::warn!("读取随包扩展 ID 失败：{error:#}"),
            None => (),
        }
    }
    let mut child = Command::new(&browser)
        .args(arguments(
            &config,
            &profile,
            native.mixed_port,
            extension.as_deref(),
            &urls,
        ))
        .creation_flags(0x08000000)
        .spawn()
        .map_err(error)?;
    // 启动阶段就退出说明内核自己崩了（常见 0x80000003）：直接报给界面，
    // 不要把用户晾在“等待回传 60 秒”里。
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    if let Ok(Some(status)) = child.try_wait() {
        if !status.success() {
            let code = match status.code() {
                Some(code) => format!("0x{:08X}", code as u32),
                None => "异常终止".into(),
            };
            return Err(format!(
                "指纹浏览器启动失败（{code}）。请关闭已打开的浏览器窗口后重试。"
            ));
        }
    }
    if selfcheck {
        *runtime.selfcheck.lock() = Some(child);
    } else {
        *runtime.child.lock() = Some(child);
    }
    if matches!(target, Target::Pending) {
        let mut pending = runtime.pending.lock();
        for url in &urls {
            pending.urls.retain(|queued| queued != url);
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn browser_environment_clear(
    app: tauri::AppHandle,
    all: bool,
    confirmed: bool,
) -> Result<(), String> {
    if !confirmed {
        return Err("请先确认清理范围".into());
    }
    let operation = app.state::<Operation>();
    let _operation = operation.lock().await;
    ensure_closed(&app).map_err(error)?;
    let (_, path) = scope(&app).map_err(error)?;
    let profile = path.join("profile");
    let targets = if all {
        vec![profile]
    } else {
        vec![
            profile.join("Default/Cache"),
            profile.join("Default/Code Cache"),
            profile.join("Default/GPUCache"),
            profile.join("ShaderCache"),
        ]
    };
    fn preflight(path: &std::path::Path) -> Result<()> {
        files::safe(path)?;
        if path.is_dir() {
            for entry in std::fs::read_dir(path)? {
                preflight(&entry?.path())?
            }
        }
        Ok(())
    }
    for target in &targets {
        preflight(target).map_err(error)?
    }
    for target in targets {
        if target.try_exists().map_err(error)? {
            std::fs::remove_dir_all(target).map_err(error)?
        }
    }
    app.state::<Runtime>().invalidate();
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn external_urls_reject_switches_files_and_ambiguous_arguments() {
        for url in [
            "--proxy-server=direct://",
            "file:///C:/secret",
            "javascript:alert(1)",
            "https://user:password@example.com/",
            "https://example.com/\" --flag",
            "https://example.com/\n",
        ] {
            assert!(validate_url(url).is_err());
        }
        assert!(validate_url(&format!("https://example.com/{}", "x".repeat(8192))).is_err());
        assert_eq!(
            validate_url("http://127.0.0.1:4123/callback?code=fixture").unwrap(),
            "http://127.0.0.1:4123/callback?code=fixture"
        );
        let args = |values: &[&str]| values.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(
            url_argument(&args(&["app", "--browser-url", "https://example.com/"]))
                .unwrap()
                .is_some()
        );
        assert!(url_argument(&args(&["app", "--browser-url"])).is_err());
        assert!(
            url_argument(&args(&[
                "app",
                "--browser-url",
                "https://example.com/",
                "--extra"
            ]))
            .is_err()
        );
        assert!(
            url_argument(&args(&["app", "--autostart"]))
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn pending_is_bounded_deduplicated_and_bound_to_identity() {
        let mut queue = Pending::default();
        for _ in 0..20 {
            queue.push("first", "https://example.com/0".into()).unwrap();
        }
        assert_eq!(queue.urls.len(), 1);
        for n in 1..16 {
            queue
                .push("first", format!("https://example.com/{n}"))
                .unwrap();
        }
        assert!(
            queue
                .push("first", "https://example.com/overflow".into())
                .is_err()
        );
        queue
            .push("second", "https://example.com/new".into())
            .unwrap();
        assert_eq!(queue.urls, vec!["https://example.com/new"]);
        let runtime = Runtime::default();
        *runtime.pending.lock() = queue;
        *runtime.notice.lock() = Some("fixture".into());
        runtime.invalidate();
        assert!(runtime.pending.lock().urls.is_empty());
        assert!(runtime.notice.lock().is_none());
    }
    #[test]
    fn arguments_open_ippure_with_fixed_proxy() {
        let args = arguments(
            &Config::default(),
            std::path::Path::new("C:/fixture/profile"),
            7897,
            None,
            &[HOME_URL.to_string()],
        );
        assert!(args.contains(&"--proxy-server=http://127.0.0.1:7897".into()));
        assert!(args.contains(&"--proxy-bypass-list=127.0.0.1;localhost;[::1]".into()));
        assert!(!args.iter().any(|a| a.contains("<-loopback>")));
        assert!(!args.iter().any(|a| a.starts_with("--load-extension=")));
        assert_eq!(args[args.len() - 2], "--");
        assert_eq!(args.last().unwrap(), "https://ippure.com/");
    }
    #[test]
    fn extension_is_loaded_before_the_url_separator() {
        let args = arguments(
            &Config::default(),
            std::path::Path::new("C:/fixture/profile"),
            7897,
            Some(std::path::Path::new("C:/fixture/extensions/webrtc-control")),
            &[HOME_URL.to_string()],
        );
        let flag = args
            .iter()
            .position(|a| a == "--load-extension=C:/fixture/extensions/webrtc-control")
            .expect("扩展目录必须以 --load-extension 传入");
        assert!(flag < args.len() - 2, "扩展参数必须位于 -- 之前");
    }
}
