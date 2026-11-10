//! 安装器卸载前清理。由 NSIS 卸载钩子在删除程序文件之前调用；不初始化 Tauri/WebView，
//! 主程序界面无法打开时也能运行。顺序与“解除保护”一致：先结束安装目录内的残留组件，
//! 再关闭指向本软件的系统代理、还原默认浏览器，最后经管理员授权停止服务、清除网络保护规则。
//! 各步骤互不依赖，任何一步失败都写入结果文件并返回非零，由安装器询问是否继续卸载。
use crate::config::{AppConfig, FileStore};
use crate::proxy::sysproxy::Sysproxy;
use anyhow::{Result, anyhow};
use std::path::{Path, PathBuf};

/// 与 tauri.conf.json 的 identifier 一致；Tauri 的 app_data_dir 即 %APPDATA%\<identifier>。
const IDENTIFIER: &str = "com.yunkuo.desktop";
const SERVICE_KEY: &str = r"SYSTEM\CurrentControlSet\Services\YunkuoService";
/// 改名前注册的旧服务；卸载新版时一并清理。
const LEGACY_SERVICE_KEY: &str = r"SYSTEM\CurrentControlSet\Services\ClaudeOneClickService";
/// 安装器的 MessageBox 有长度上限，结果只保留开头部分。
const REPORT_CHARS: usize = 400;

pub fn run(report: Option<PathBuf>) -> i32 {
    let failures = cleanup();
    if let Some(path) = report {
        let _ = write_report(&path, &failures.join("；"));
    }
    if failures.is_empty() { 0 } else { 1 }
}

fn cleanup() -> Vec<String> {
    let Some(install_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    else {
        return vec!["无法定位安装目录".into()];
    };
    let mut failures = Vec::new();
    if let Err(error) = stop_components(&install_dir) {
        failures.push(format!("{error:#}"));
    }
    restore_user_settings(&mut failures);
    if let Err(error) = remove_service(&install_dir) {
        failures.push(format!("{error:#}"));
    }
    failures
}

/// 结束主程序及从安装目录启动的内核、浏览器等组件，避免它们在清理期间重写系统设置或占用待删除文件。
fn stop_components(install_dir: &Path) -> Result<()> {
    use sysinfo::{ProcessRefreshKind, RefreshKind, System, UpdateKind};
    let prefix = format!(
        "{}\\",
        install_dir
            .to_string_lossy()
            .trim_end_matches('\\')
            .to_lowercase()
    );
    let own = sysinfo::get_current_pid().ok();
    let system = System::new_with_specifics(
        RefreshKind::nothing()
            .with_processes(ProcessRefreshKind::nothing().with_exe(UpdateKind::Always)),
    );
    // 升级时选择“先卸载旧版本”，卸载程序会在安装目录内原地运行，不能结束调用本清理的卸载程序。
    let launcher = own
        .and_then(|pid| system.process(pid))
        .and_then(|process| process.parent());
    let mut remaining = Vec::new();
    for (pid, process) in system.processes() {
        let Some(exe) = process.exe() else { continue };
        let exe = exe.to_string_lossy().to_lowercase();
        if Some(*pid) == own
            || Some(*pid) == launcher
            || !exe.starts_with(&prefix)
            || exe.ends_with("\\uninstall.exe")
        {
            continue;
        }
        if process.kill_and_wait().is_err() {
            remaining.push(process.name().to_string_lossy().into_owned());
        }
    }
    if remaining.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(
            "以下组件仍在运行，未能结束：{}",
            remaining.join("、")
        ))
    }
}

/// 关闭仍指向本软件端口的系统代理（其他软件设置的代理不动），还原默认浏览器并注销浏览器入口。
fn restore_user_settings(failures: &mut Vec<String>) {
    let directory = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .filter(|base| base.is_absolute())
        .map(|base| base.join(IDENTIFIER));
    let lease_exists = directory
        .as_ref()
        .is_some_and(|path| path.join("proxy-lease.json").is_file());
    if let Some(directory) = &directory {
        if let Err(error) = crate::proxy::sysproxy::release_saved(directory) {
            failures.push(format!("原系统代理恢复失败：{error:#}"));
        }
    }
    let store = directory.map(|base| FileStore::new(base.join("config.yaml")));
    let config = match store.as_ref().map(|store| store.load::<AppConfig>()) {
        Some(Ok(config)) => Some(config),
        Some(Err(error)) => {
            failures.push(format!("配置无法读取，请手动确认系统代理已关闭：{error:#}"));
            None
        }
        None => {
            failures.push("无法定位用户配置目录，请手动确认系统代理已关闭".into());
            None
        }
    };
    if let Some(config) = &config {
        // A lease already restored the exact original values. The port-only
        // fallback is exclusively for installations predating that journal.
        if !lease_exists {
            if let Err(error) = disable_app_proxy(config.mixed_port) {
                failures.push(format!("系统代理关闭失败：{error:#}"));
            }
        }
    }
    match crate::fingerprint_browser::defaultbrowser::release_for_uninstall(
        config
            .as_ref()
            .and_then(|c| c.default_browser_backup.as_ref()),
    ) {
        Ok(()) => {
            // 清掉已用过的备份，避免升级安装后的新版本把旧值再写回一次；写失败不影响本次还原结果。
            if let (Some(store), Some(mut config)) = (store, config) {
                if config.default_browser_backup.take().is_some() {
                    let _ = store.save(&config);
                }
            }
            if let Err(error) = crate::fingerprint_browser::defaultbrowser::unregister() {
                failures.push(format!("浏览器入口注销失败：{error:#}"));
            }
        }
        Err(error) => failures.push(format!(
            "默认浏览器还原失败，请在 Windows 设置中重新选择：{error:#}"
        )),
    }
}

fn disable_app_proxy(port: u16) -> Result<()> {
    // 读不到或格式不是单一地址时，说明不是本软件写入的代理。
    let Ok(current) = crate::proxy::sysproxy::get_sysproxy() else {
        return Ok(());
    };
    if points_to_app(&current, port) {
        Sysproxy {
            enable: false,
            ..current
        }
        .set_system_proxy()?;
    }
    Ok(())
}

fn points_to_app(proxy: &Sysproxy, port: u16) -> bool {
    proxy.enable && proxy.host == "127.0.0.1" && proxy.port == port
}

/// 服务已注册或服务目录仍在时，经管理员授权停止并删除服务、清除网络保护规则和服务目录。
fn remove_service(install_dir: &Path) -> Result<()> {
    let legacy = registered_under(LEGACY_SERVICE_KEY);
    if !service_registered() && !legacy && !Path::new(crate::service_protocol::PROGRAM_DIR).exists()
    {
        return Ok(());
    }
    let owner = crate::proxy::service_install::current_user_sid()?;
    let script = crate::proxy::service_install::removal_script(
        &install_dir.join("yunkuo-service.exe"),
        &owner,
    )?;
    crate::proxy::service_install::run_elevated(&script)
        .map_err(|_| anyhow!("安全服务或网络保护规则未能移除（可能取消了管理员授权）"))
}

fn service_registered() -> bool {
    registered_under(SERVICE_KEY)
}

fn registered_under(key: &str) -> bool {
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{
        HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RegCloseKey, RegOpenKeyExW,
    };
    use windows::core::PCWSTR;
    let path: Vec<u16> = key.encode_utf16().chain(Some(0)).collect();
    let mut key = HKEY::default();
    let code = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(path.as_ptr()),
            None,
            KEY_READ,
            &mut key,
        )
    };
    if code == ERROR_SUCCESS {
        unsafe {
            let _ = RegCloseKey(key);
        }
    }
    // 读取被拒绝等情况按已注册处理，交给管理员帮助进程确认。
    code != ERROR_FILE_NOT_FOUND
}

/// 安装器用 FileReadUTF16LE 读取首行，因此写成带 BOM 的单行 UTF-16LE。
fn write_report(path: &Path, message: &str) -> std::io::Result<()> {
    let line: String = message
        .chars()
        .map(|c| if c == '\r' || c == '\n' { ' ' } else { c })
        .take(REPORT_CHARS)
        .collect();
    let bytes: Vec<u8> = std::iter::once(0xFEFF)
        .chain(line.encode_utf16())
        .flat_map(u16::to_le_bytes)
        .collect();
    std::fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_apps_own_loopback_proxy_is_turned_off() {
        let proxy = |enable, host: &str, port| Sysproxy {
            enable,
            host: host.into(),
            port,
            bypass: String::new(),
        };
        assert!(points_to_app(&proxy(true, "127.0.0.1", 7897), 7897));
        assert!(
            !points_to_app(&proxy(true, "127.0.0.1", 7890), 7897),
            "其他软件的本机端口不动"
        );
        assert!(
            !points_to_app(&proxy(true, "10.0.0.2", 7897), 7897),
            "企业代理不动"
        );
        assert!(!points_to_app(&proxy(false, "127.0.0.1", 7897), 7897));
    }

    #[test]
    fn report_is_a_single_bom_prefixed_utf16_line() {
        let dir = crate::test_support::scratch_root()
            .join(format!("yunkuo-uninstall-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("result.txt");
        write_report(
            &path,
            &format!("第一行\r\n第二行{}", "长".repeat(REPORT_CHARS)),
        )
        .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(units[0], 0xFEFF);
        let text = String::from_utf16(&units[1..]).unwrap();
        assert!(text.starts_with("第一行  第二行") && !text.contains('\n'));
        assert_eq!(text.chars().count(), REPORT_CHARS);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
