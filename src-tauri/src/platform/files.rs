use anyhow::{Context, Result, bail};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs,
    io::Read,
    os::windows::{fs::MetadataExt, process::CommandExt},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// 保留既有物理路径；身份资料与浏览器 profile 仍分别由各自业务拥有。
pub(crate) fn environment_root(app: &tauri::AppHandle) -> Result<std::path::PathBuf> {
    use tauri::Manager;
    Ok(app.path().app_data_dir()?.join("environment"))
}

pub(crate) fn error(error: impl std::fmt::Display) -> String {
    format!("{error}")
}

/// 拒绝包含 junction/symlink 的目录链，避免跨越受管理目录。
pub fn safe(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) if meta.file_attributes() & 0x400 != 0 => {
                bail!("路径含重解析点，操作已停止：{}", path.display())
            }
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
/// 读取不超过 `limit` 字节的文件。
///
/// 第三参数是**完整**的用户可见文案（不是"读什么"）：各调用点的超限提示都带有自己的
/// 数值、处置指引或安全承诺，不能由统一模板拼出来。
pub fn read_bounded(path: &Path, limit: usize, message: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= limit, "{message}");
    Ok(bytes)
}
pub fn read<T: DeserializeOwned + Default>(path: &Path) -> Result<T> {
    safe(path)?;
    match fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).context("环境数据损坏，原文件已保留")?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e.into()),
    }
}
pub fn write<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    safe(path)?;
    crate::config::atomic_write(path, &serde_json::to_vec_pretty(value)?)
}
pub fn shell(script: &str, env: &[(&str, &str)]) -> Result<String> {
    let executable = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "C:\\Windows".into())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let mut child = Command::new(executable)
        .creation_flags(0x08000000)
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        // 从 PowerShell 7 启动时，不能把其模块目录带给 Windows PowerShell 5。
        .env_remove("PSModulePath")
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let output = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut bytes = vec![];
        output
            .take(1_048_577)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                bail!("Windows 操作未完成；请关闭相关程序后重试，或检查系统权限。")
            }
            break;
        }
        if start.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Windows 操作超时；部分设置可能已生效，请刷新状态，必要时使用备份恢复。")
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    let bytes = reader
        .join()
        .map_err(|_| anyhow::anyhow!("Windows 操作输出中断"))??;
    if bytes.len() > 1_048_576 {
        bail!("Windows 操作输出超过限制")
    }
    Ok(String::from_utf8(bytes)?
        .trim_start_matches('\u{feff}')
        .trim()
        .into())
}
/// 私密根目录仅给予当前用户访问权；不调整其父目录权限。
pub fn private_dir(path: &Path) -> Result<()> {
    safe(path)?;
    fs::create_dir_all(path)?;
    shell(
        r#"$ErrorActionPreference='Stop'; $p=$env:COC_PRIVATE; $sid=[Security.Principal.WindowsIdentity]::GetCurrent().User; $acl=New-Object Security.AccessControl.DirectorySecurity; $acl.SetOwner($sid); $acl.SetAccessRuleProtection($true,$false); $rule=New-Object Security.AccessControl.FileSystemAccessRule($sid,'FullControl','ContainerInherit,ObjectInherit','None','Allow'); $acl.AddAccessRule($rule); Set-Acl -LiteralPath $p -AclObject $acl"#,
        &[("COC_PRIVATE", &path.to_string_lossy())],
    )?;
    Ok(())
}
