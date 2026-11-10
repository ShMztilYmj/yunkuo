//! 保留 Windows 代理原始字符串，兼容空设置、域名和按协议配置的代理。
use anyhow::{Result, anyhow, bail};
use windows::{
    Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA},
        System::Registry::*,
    },
    core::{PCWSTR, w},
};
const KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings");
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn text(name: PCWSTR) -> Result<String> {
    // 两次读取之间其他软件可能改值，大小变化时只重试读取，不以空值覆盖。
    for _ in 0..3 {
        let mut size = 0;
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                KEY,
                name,
                RRF_RT_REG_SZ,
                None,
                None,
                Some(&mut size),
            )
        };
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(String::new());
        }
        result.ok()?;
        if size > 128 * 1024 {
            bail!("系统代理字符串过大，未改动设置");
        }
        let mut buffer = vec![0u16; (size as usize).div_ceil(2).max(1)];
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                KEY,
                name,
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if result == ERROR_MORE_DATA {
            continue;
        }
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(String::new());
        }
        result.ok()?;
        let length = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
        return Ok(String::from_utf16_lossy(&buffer[..length]));
    }
    bail!("系统代理被其他软件持续修改，请稍后重试")
}
pub(super) fn read() -> Result<(bool, String, String)> {
    let mut enabled = 0u32;
    let mut size = 4;
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            KEY,
            w!("ProxyEnable"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut enabled as *mut u32).cast()),
            Some(&mut size),
        )
    };
    if result != ERROR_FILE_NOT_FOUND {
        result.ok()?;
    }
    Ok((
        enabled != 0,
        text(w!("ProxyServer"))?,
        text(w!("ProxyOverride"))?,
    ))
}
pub(super) fn write(enabled: bool, server: &str, bypass: &str) -> Result<()> {
    // 禁用时先关闭接管；启用时最后打开，避免中间状态指向半写入的端口。
    if !enabled {
        write_enabled(false)?;
    }
    for (key, value) in [(w!("ProxyServer"), server), (w!("ProxyOverride"), bypass)] {
        let value = wide(value);
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                KEY,
                key,
                REG_SZ.0,
                Some(value.as_ptr().cast()),
                (value.len() * 2) as u32,
            )
        }
        .ok()?;
    }
    if enabled {
        write_enabled(true)?;
    }
    // WinINet 通知仅刷新系统代理缓存，不发起网络请求。
    #[link(name = "wininet")]
    unsafe extern "system" {
        fn InternetSetOptionW(
            handle: *mut core::ffi::c_void,
            option: u32,
            buffer: *const core::ffi::c_void,
            length: u32,
        ) -> i32;
    }
    unsafe {
        if InternetSetOptionW(std::ptr::null_mut(), 39, std::ptr::null(), 0) == 0 {
            return Err(anyhow!("系统代理已写入，但设置变更通知失败"));
        }
        if InternetSetOptionW(std::ptr::null_mut(), 37, std::ptr::null(), 0) == 0 {
            return Err(anyhow!("系统代理已写入，但缓存刷新失败"));
        }
    }
    Ok(())
}
fn write_enabled(enabled: bool) -> Result<()> {
    let enabled = u32::from(enabled);
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            KEY,
            w!("ProxyEnable"),
            REG_DWORD.0,
            Some((&enabled as *const u32).cast()),
            4,
        )
    }
    .ok()?;
    Ok(())
}
