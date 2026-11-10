//! 当前 Windows 用户的受保护存储机制。
//! 不拥有业务资料；DPAPI、重解析点检查、私有 ACL 与原子替换沿用原有行为。
use anyhow::{Context, Result, anyhow, bail};
use std::{fs, os::windows::process::CommandExt, path::Path};
use windows::{
    Win32::{
        Foundation::{HLOCAL, LocalFree},
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
    },
    core::PCWSTR,
};
use zeroize::Zeroizing;

pub(crate) fn safe(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        bail!("需要绝对路径，未访问账号或配置文件");
    }
    use std::os::windows::fs::MetadataExt;
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_attributes() & 0x400 != 0 => {
                bail!("账号路径含链接或重解析点，已停止")
            }
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

pub(crate) fn crypt(bytes: &[u8], encrypt: bool) -> Result<Zeroizing<Vec<u8>>> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len().try_into()?,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // 不使用 LOCAL_MACHINE：只能由当前 Windows 用户解密，不产生明文主密钥。
    unsafe {
        if encrypt {
            CryptProtectData(
                &input,
                PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
        .context("快照无法验证或不属于当前 Windows 用户；当前账号保持不变")?;
        if output.pbData.is_null() || output.cbData == 0 {
            if !output.pbData.is_null() {
                let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
            }
            bail!("解密结果为空，未修改当前数据");
        }
        let buffer = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
        let result = Zeroizing::new(buffer.to_vec());
        buffer.fill(0);
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
        Ok(result)
    }
}

pub(crate) fn private_file(path: &Path) -> Result<()> {
    private_acl(path, false)
}

pub(crate) fn private_directory(path: &Path) -> Result<()> {
    private_acl(path, true)
}

fn private_acl(path: &Path, directory: bool) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Security::{
        Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
        DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        SetFileSecurityW,
    };
    let sid = current_user_sid()?;
    let flags = if directory { "OICI" } else { "" };
    let sddl: Vec<u16> = format!("D:P(A;{flags};FA;;;SY)(A;{flags};FA;;;BA)(A;{flags};FA;;;{sid})")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let target: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            1,
            &mut descriptor,
            None,
        )?;
        let result = SetFileSecurityW(
            PCWSTR(target.as_ptr()),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        )
        .ok();
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        result?;
    }
    Ok(())
}

pub(crate) fn write_auth(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::{io::Write, os::windows::ffi::OsStrExt};
    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_WRITE_THROUGH, MoveFileExW, REPLACE_FILE_FLAGS, ReplaceFileW,
    };
    safe(path)?;
    let parent = path.parent().context("认证路径无效")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".auth-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        // 临时文件尚为空时先收紧权限；明文从未写入继承宽权限的暂存文件。
        private_file(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        let from: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            if path.exists() {
                ReplaceFileW(
                    PCWSTR(to.as_ptr()),
                    PCWSTR(from.as_ptr()),
                    PCWSTR::null(),
                    REPLACE_FILE_FLAGS(0),
                    None,
                    None,
                )?;
            } else {
                MoveFileExW(
                    PCWSTR(from.as_ptr()),
                    PCWSTR(to.as_ptr()),
                    MOVEFILE_WRITE_THROUGH,
                )?;
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn current_user_sid() -> Result<String> {
    let system = std::path::PathBuf::from(
        std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()),
    );
    let output = std::process::Command::new(system.join("System32/whoami.exe"))
        .args(["/user", "/fo", "csv", "/nh"])
        .creation_flags(0x0800_0000)
        .output()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let sid = text
        .trim()
        .split(',')
        .next_back()
        .unwrap_or("")
        .trim_matches('"')
        .to_string();
    if !valid_sid(&sid) {
        return Err(anyhow!("无法确认当前 Windows 用户"));
    }
    Ok(sid)
}

fn valid_sid(sid: &str) -> bool {
    sid.starts_with("S-1-")
        && sid
            .bytes()
            .all(|c| c.is_ascii_digit() || c == b'S' || c == b'-')
}
