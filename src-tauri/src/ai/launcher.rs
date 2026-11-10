use anyhow::{Result, anyhow};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::PCWSTR;

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 通过快捷方式（.lnk）或可执行文件启动 Claude
pub fn launch_shortcut(path: &str) -> Result<()> {
    let path = path.trim();
    if path.is_empty() {
        return Err(anyhow!("尚未配置 Claude 启动快捷方式，请到设置页选择"));
    }
    if !std::path::Path::new(path).exists() {
        return Err(anyhow!("快捷方式不存在: {path}"));
    }
    let file = to_wide(path);
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR::null(),
            PCWSTR(file.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW 返回值 > 32 表示成功
    if result.0 as isize <= 32 {
        return Err(anyhow!(
            "启动失败（ShellExecute 返回 {}），请确认快捷方式有效",
            result.0 as isize
        ));
    }
    Ok(())
}
