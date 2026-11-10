//! 固定版本的 tokscale 子进程：校验、启动、超时、取消与退出清理。
//! 只负责进程边界，结果解析与缓存由引擎模块持有。

use super::Cache;
use crate::ai::agent_tools::MonitoringScope;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::AsyncReadExt;

pub(super) fn clients(scope: &MonitoringScope) -> String {
    scope
        .keys()
        .flat_map(crate::ai::agent_roster::usage_clients)
        .collect::<Vec<_>>()
        .join(",")
}
#[derive(Clone)]
pub(super) struct Job {
    pub(super) scope: MonitoringScope,
    pub(super) generation: u64,
    pub(super) cancel: tokio::sync::watch::Receiver<bool>,
}
impl Job {
    pub(super) fn current(&self, cache: &Cache) -> bool {
        cache.generation == self.generation && cache.scope.as_ref() == Some(&self.scope)
    }
}
pub(super) const SHA256: &str = "887113d6adeb4aa15bb298e80d316a1dc1c961d4aa469cec64533ac606b9d173";
pub(super) static BINARY: once_cell::sync::OnceCell<PathBuf> = once_cell::sync::OnceCell::new();

pub(crate) fn init(resource_dir: &Path) {
    let candidates = [
        resource_dir.join("resources/tokscale/tokscale.exe"),
        resource_dir.join("tokscale/tokscale.exe"),
    ];
    #[cfg(debug_assertions)]
    let development = std::iter::once(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/tokscale/tokscale.exe"),
    );
    #[cfg(not(debug_assertions))]
    let development = std::iter::empty::<PathBuf>();
    if let Some(path) = candidates
        .into_iter()
        .chain(development)
        .find(|p| p.is_file())
    {
        let _ = BINARY.set(path);
    }
}
pub(super) fn verified_binary() -> Result<PathBuf, String> {
    // Verify once, before executing the bundled provider. Never use PATH or a downloaded update pointer.
    static VERIFIED: once_cell::sync::OnceCell<PathBuf> = once_cell::sync::OnceCell::new();
    VERIFIED
        .get_or_try_init(|| {
            let path = BINARY.get().ok_or("用量采集引擎未随软件安装")?;
            let bytes = std::fs::read(path).map_err(|_| "无法读取用量采集引擎")?;
            let actual = crate::platform::digest::sha256_lower(&bytes);
            if actual != SHA256 {
                return Err("用量采集引擎校验失败，请修复安装".into());
            }
            Ok(path.clone())
        })
        .cloned()
}
pub(crate) fn verify_bundle() -> Result<(), String> {
    verified_binary().map(|_| ())
}
pub(super) async fn run(
    bin: &Path,
    args: &[String],
    base: &Path,
    timeout: u64,
    job: &Job,
    profile: Option<&Path>,
) -> Result<Value, String> {
    if *job.cancel.borrow() {
        return Err("Agent 名单已更新".into());
    }
    let mut command = tokio::process::Command::new(bin);
    // --home is an isolated-report option and is rejected by cursor/antigravity/usage.
    // Production uses normal roots, including XDG/custom paths, with the current user's native home.
    command
        .args(args)
        .env("HOME", base)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if let Some(profile) = profile {
        command.env("TOKSCALE_CONFIG_DIR", profile);
    }
    // 额度查询要打 api.anthropic.com / chatgpt.com：必须经本机代理出网，不能直连暴露真实 IP。
    // 引擎（reqwest）默认读这些环境变量；端口不通时请求失败并保留旧读数，不回退直连。
    if let Some(port) = crate::ai::aimon::proxy_port() {
        for (name, value) in crate::ai::aimon::proxy_env(port) {
            command.env(name, value);
        }
    }
    #[cfg(windows)]
    // CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS：批量解析数万条记录是 CPU 密集的，
    // 降到低于正常优先级，避免一次扫描把界面线程饿住。
    command.creation_flags(0x0800_0000 | 0x0000_4000);
    // Blank XDG values otherwise accidentally redirect the scanner into the working directory.
    for key in ["XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"] {
        if std::env::var_os(key).is_some_and(|v| v.is_empty()) {
            command.env_remove(key);
        }
    }
    let started = chrono::Utc::now().timestamp();
    let mut child = command.spawn().map_err(|_| "用量采集引擎无法启动")?;
    let pid = child.id();
    let stdout = child.stdout.take().ok_or("用量采集管道不可用")?;
    let work = async {
        let mut bytes = Vec::new();
        stdout
            .take(64 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| "用量结果读取失败")?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err("用量结果超出读取上限".to_string());
        }
        let status = child.wait().await.map_err(|_| "用量进程未正常结束")?;
        if !status.success() {
            return Err("用量来源读取失败，保留最近成功结果".into());
        }
        if args.first().is_some_and(|s| s == "antigravity")
            && args.get(1).is_some_and(|s| s == "sync")
        {
            Ok(Value::Bool(true)) // The upstream sync command intentionally prints a human-readable summary.
        } else {
            serde_json::from_slice(&bytes).map_err(|_| "用量来源未返回有效数据".into())
        }
    };
    let mut cancelled = job.cancel.clone();
    let outcome = tokio::select! {
        result = tokio::time::timeout(Duration::from_secs(timeout), work) => result,
        _ = cancelled.changed() => return cancelled_run(&mut child, args, base, pid, started).await,
    };
    let result = match outcome {
        Ok(result) => {
            if result.is_err() {
                let _ = child.kill().await;
            }
            result
        }
        Err(_) => {
            let _ = child.kill().await;
            Err("用量同步超时，保留最近成功结果".into())
        }
    };
    if result.is_err()
        && args.first().is_some_and(|s| s == "antigravity")
        && child.try_wait().ok().flatten().is_some()
    {
        if let Some(pid) = pid {
            cleanup_owned_sync_lock(&antigravity_cache(base), pid, started);
        }
    }
    result
}
pub(super) fn antigravity_cache(base: &Path) -> PathBuf {
    std::env::var_os("TOKSCALE_CONFIG_DIR")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| base.join("AppData/Roaming"))
                .join("tokscale")
        })
        .join("antigravity-cache")
}
pub(super) async fn cancelled_run(
    child: &mut tokio::process::Child,
    command: &[String],
    base: &Path,
    pid: Option<u32>,
    started: i64,
) -> Result<Value, String> {
    let _ = child.kill().await;
    if command.first().is_some_and(|value| value == "antigravity")
        && child.try_wait().ok().flatten().is_some()
    {
        if let Some(pid) = pid {
            cleanup_owned_sync_lock(&antigravity_cache(base), pid, started);
        }
    }
    Err("Agent 名单已更新，已停止旧采集".into())
}
pub(super) fn cleanup_owned_sync_lock(cache: &Path, pid: u32, started: i64) {
    // Only reclaim a record belonging to the child we just reaped. Unknown/crashed external locks remain untouched.
    let os_path = cache.join("sync.os.lock");
    let path = cache.join("sync.lock");
    let Ok(meta) = std::fs::symlink_metadata(&path) else {
        return;
    };
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 128 {
        return;
    }
    let Ok(os_lock) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(os_path)
    else {
        return;
    };
    if os_lock.try_lock().is_err() {
        return;
    }
    let Ok(record) = std::fs::read_to_string(&path) else {
        return;
    };
    let parts: Vec<_> = record.split_whitespace().collect();
    if parts.len() != 2 || parts[0].parse::<u32>().ok() != Some(pid) {
        return;
    }
    let Some(time) = parts[1].parse::<i64>().ok() else {
        return;
    };
    if time < started - 1 || time > chrono::Utc::now().timestamp() + 1 {
        return;
    }
    if std::fs::read_to_string(&path).ok().as_deref() == Some(&record) {
        let _ = std::fs::remove_file(path);
    }
}
pub(super) fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| (*s).into()).collect()
}
