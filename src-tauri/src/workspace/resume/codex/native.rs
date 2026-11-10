//! Read-only Windows Restart Manager inventory. Never acquires Codex's writer lock.
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use windows::{
    Win32::{
        Foundation::{CloseHandle, FILETIME},
        System::{
            RestartManager::*,
            Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
        },
    },
    core::{PCWSTR, PWSTR},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    pub created: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Desktop {
    pub process: Process,
    pub exe: PathBuf,
    pub size: u64,
    pub modified: u128,
}

pub fn users(files: &[PathBuf]) -> Result<Vec<Process>> {
    if files.is_empty() {
        bail!("没有可核验的 Codex 资源");
    }
    struct Session(u32);
    impl Drop for Session {
        fn drop(&mut self) {
            unsafe {
                let _ = RmEndSession(self.0);
            }
        }
    }
    let mut session = 0;
    let mut key = [0u16; 33];
    unsafe {
        if RmStartSession(&mut session, None, PWSTR(key.as_mut_ptr())).0 != 0 {
            bail!("会话加载探测不可用");
        }
    }
    let guard = Session(session);
    let names: Vec<Vec<u16>> = files
        .iter()
        .map(|p| p.to_string_lossy().encode_utf16().chain(Some(0)).collect())
        .collect();
    let ptrs: Vec<_> = names.iter().map(|n| PCWSTR(n.as_ptr())).collect();
    unsafe {
        if RmRegisterResources(guard.0, Some(&ptrs), None, None).0 != 0 {
            bail!("会话资源探测失败");
        }
        let (mut needed, mut count, mut reasons) = (0, 0, 0);
        let mut rc = RmGetList(guard.0, &mut needed, &mut count, None, &mut reasons);
        if rc.0 == 0 {
            return Ok(Vec::new());
        }
        for _ in 0..3 {
            if rc.0 != 234 || needed > 1024 {
                bail!("无法核验会话持有者");
            }
            let mut list = vec![RM_PROCESS_INFO::default(); needed as usize];
            count = needed;
            rc = RmGetList(
                guard.0,
                &mut needed,
                &mut count,
                Some(list.as_mut_ptr()),
                &mut reasons,
            );
            if rc.0 == 0 {
                return Ok(list
                    .into_iter()
                    .take(count as usize)
                    .map(|p| Process {
                        pid: p.Process.dwProcessId,
                        created: stamp(p.Process.ProcessStartTime),
                    })
                    .collect());
            }
        }
    }
    bail!("会话持有者正在变化，请稍后重试")
}
fn stamp(t: FILETIME) -> u64 {
    ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64
}
fn process(pid: u32) -> Result<Process> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)?;
        let (mut created, mut exit, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        let result = GetProcessTimes(handle, &mut created, &mut exit, &mut kernel, &mut user);
        let _ = CloseHandle(handle);
        result?;
        Ok(Process {
            pid,
            created: stamp(created),
        })
    }
}
fn official_engine(path: &Path) -> bool {
    let Some(root) =
        std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("OpenAI/Codex/bin"))
    else {
        return false;
    };
    let Ok(path) = path.canonicalize() else {
        return false;
    };
    let Ok(root) = root.canonicalize() else {
        return false;
    };
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let parts: Vec<_> = relative.components().collect();
    parts.len() == 2 && parts[1].as_os_str().eq_ignore_ascii_case("codex.exe") && {
        let hash = parts[0].as_os_str().to_string_lossy();
        (8..=64).contains(&hash.len()) && hash.chars().all(|c| c.is_ascii_hexdigit())
    }
}
fn official_app(path: &Path) -> bool {
    let root = std::env::var_os("ProgramW6432")
        .or_else(|| std::env::var_os("ProgramFiles"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:/Program Files"))
        .join("WindowsApps");
    let p = path.to_string_lossy().to_lowercase();
    let base = root.to_string_lossy().to_lowercase();
    p.starts_with(&format!("{base}\\openai.codex_"))
        && p.ends_with("__2p2nqsd0c76g0\\app\\chatgpt.exe")
}
pub fn desktop(home: &Path) -> Result<Desktop> {
    let system =
        sysinfo::System::new_with_specifics(sysinfo::RefreshKind::nothing().with_processes(
            sysinfo::ProcessRefreshKind::nothing().with_exe(sysinfo::UpdateKind::Always),
        ));
    let apps: Vec<_> = system
        .processes()
        .values()
        .filter(|p| p.exe().is_some_and(official_app))
        .collect();
    let mains: Vec<_> = apps
        .iter()
        .filter(|p| {
            p.parent()
                .is_none_or(|id| !apps.iter().any(|a| a.pid() == id))
        })
        .collect();
    if mains.len() != 1 {
        bail!("等待 Codex 桌面端启动，或关闭重复的桌面实例");
    }
    let engines: Vec<_> = system
        .processes()
        .values()
        .filter(|p| p.parent() == Some(mains[0].pid()) && p.exe().is_some_and(official_engine))
        .collect();
    let files: Vec<_> = ["state", "queue"]
        .iter()
        .filter_map(|kind| super::source::database(home, kind).ok())
        .collect();
    let held = users(&files)?;
    let servers: Vec<_> = engines
        .iter()
        .filter(|p| held.iter().any(|h| h.pid == p.pid().as_u32()))
        .collect();
    if servers.len() != 1 {
        bail!("无法唯一确定持有本机数据的官方 Codex 引擎");
    }
    let server = servers[0];
    let identity = process(server.pid().as_u32())?;
    if !held.contains(&identity) {
        bail!("Codex 引擎已变化，请重新核验");
    }
    let exe = server
        .exe()
        .context("无法读取 Codex 引擎位置")?
        .canonicalize()?;
    let meta = std::fs::metadata(&exe)?;
    Ok(Desktop {
        process: identity,
        exe,
        size: meta.len(),
        modified: meta
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos(),
    })
}
pub fn loaded(home: &Path, id: &str, desktop: &Desktop) -> Result<()> {
    if !super::model::valid_id(id) {
        bail!("无效会话标识");
    }
    let file = home.join("thread-writer-locks").join(format!("{id}.lock"));
    if !file.exists() {
        bail!("等待在 Codex 中打开此会话");
    }
    let holders = users(&[file])?;
    if holders.len() == 1
        && holders[0] == desktop.process
        && process(desktop.process.pid)? == desktop.process
    {
        return Ok(());
    }
    if holders.is_empty() {
        bail!("等待在 Codex 中打开此会话");
    }
    bail!("会话由其他实例使用，或持有者暂时无法核验")
}
