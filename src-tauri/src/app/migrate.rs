//! 旧品牌（Claude One Click / claude-one-click）到「云阔」的一次性迁移。
//!
//! 三处历史标识会随着改名失效，必须在新版本首次运行时接续，否则老用户升级后
//! 等于全新安装，或者系统里留下一条指向旧路径、仍在抢 TUN 的服务：
//! 1. 应用数据目录 `%APPDATA%\com.claudeoneclick.app` → `%APPDATA%\com.yunkuo.desktop`；
//! 2. 开机启动项 `Claude一键配置` → `云阔`（服务端在桌面端 setup 里处理）；
//! 3. 系统服务 `ClaudeOneClickService` / `C:\ProgramData\ClaudeOneClick`
//!    → `YunkuoService` / `C:\ProgramData\Yunkuo`（安装服务时处理）。
//!
//! 迁移只做一次，且**不删旧数据**：重命名成功即自然消失，失败就原样保留旧目录，
//! 让用户仍能用旧版本取回自己的配置。

/// 旧版本的 Tauri 应用标识，等价于 `%APPDATA%\<此处>`。
pub(crate) const LEGACY_IDENTIFIER: &str = "com.claudeoneclick.app";
/// 与 tauri.conf.json 的 identifier 保持一致。
pub(crate) const IDENTIFIER: &str = "com.yunkuo.desktop";
/// 旧版本写入的开机启动项名（任务管理器「启动」里显示的名字）。
const LEGACY_AUTOSTART_NAME: &str = "Claude一键配置";

fn appdata() -> Option<std::path::PathBuf> {
    std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
}

/// 把旧标识的数据目录整体接到新标识上。
///
/// 返回迁移结果说明，仅供日志；任何失败都不阻断启动（界面会按新目录重新初始化）。
pub(crate) fn adopt_legacy_data_dir() -> Option<String> {
    let base = appdata()?;
    let legacy = base.join(LEGACY_IDENTIFIER);
    let current = base.join(IDENTIFIER);
    if !legacy.is_dir() || current.exists() {
        return None;
    }
    match std::fs::rename(&legacy, &current) {
        Ok(()) => Some(format!(
            "已把旧数据目录 {} 迁移到 {}",
            legacy.display(),
            current.display()
        )),
        Err(error) => {
            // 重命名失败（常见原因：旧版本仍在运行占着 cache.db）时不动旧目录，
            // 复制一份配置让用户至少保留设置。
            let copied = copy_dir(&legacy, &current).is_ok();
            Some(if copied {
                format!(
                    "旧数据目录无法重命名（{error}），已复制到 {}；旧目录保留未删除",
                    current.display()
                )
            } else {
                format!("旧数据目录迁移失败（{error}），本次按新目录初始化")
            })
        }
    }
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if !target.exists() {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// 删除旧版开机启动项。
///
/// 调用点在 Tauri 的 autostart 插件接管之前：插件只会读写新名字，
/// 旧值留着就会每次开机拉起一个已经不在的路径，任务管理器里也还挂着旧品牌。
pub(crate) fn drop_legacy_autostart() {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, RegCloseKey, RegDeleteValueW, RegOpenKeyExW,
    };
    use windows::core::PCWSTR;
    let key_path: Vec<u16> = "Software\\Microsoft\\Windows\\CurrentVersion\\Run"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let name: Vec<u16> = LEGACY_AUTOSTART_NAME
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut key = HKEY::default();
    unsafe {
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key_path.as_ptr()),
            None,
            KEY_SET_VALUE,
            &mut key,
        )
        .is_err()
        {
            return;
        }
        let _ = RegDeleteValueW(key, PCWSTR(name.as_ptr()));
        let _ = RegCloseKey(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_differ_and_are_well_formed() {
        assert_ne!(IDENTIFIER, LEGACY_IDENTIFIER);
        // 不能以 `.app` 结尾：Tauri 会把它当成 macOS 应用包扩展名并给出警告。
        assert!(IDENTIFIER.starts_with("com.") && IDENTIFIER.ends_with(".desktop"));
    }

    #[test]
    fn copies_a_missing_target_once() {
        let root = crate::test_support::scratch_root()
            .join(format!("yunkuo-migrate-{}", uuid::Uuid::new_v4()));
        let from = root.join("old");
        let to = root.join("new");
        std::fs::create_dir_all(from.join("profiles")).unwrap();
        std::fs::write(from.join("config.yaml"), "port: 7897").unwrap();
        std::fs::write(from.join("profiles").join("a.yaml"), "x").unwrap();
        copy_dir(&from, &to).unwrap();
        assert_eq!(
            std::fs::read_to_string(to.join("config.yaml")).unwrap(),
            "port: 7897"
        );
        assert!(to.join("profiles").join("a.yaml").is_file());
        // 目标已存在时不覆盖，避免把新版本的写入冲掉。
        std::fs::write(to.join("config.yaml"), "port: 7898").unwrap();
        copy_dir(&from, &to).unwrap();
        assert_eq!(
            std::fs::read_to_string(to.join("config.yaml")).unwrap(),
            "port: 7898"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
