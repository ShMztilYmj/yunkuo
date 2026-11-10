//! Codex 认证快照：当前 Windows 用户 DPAPI 加密；有效性检查与原子激活。
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Serialize;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};
use zeroize::Zeroizing;

#[cfg(test)]
use crate::platform::secure_storage::private_file;
use crate::platform::secure_storage::{crypt, safe, write_auth};
#[cfg(test)]
use windows::core::PCWSTR;

const LIMIT: u64 = 1024 * 1024;
const MAGIC: &[u8] = b"COCSNAP1";
static OPERATION: Mutex<()> = Mutex::new(());

pub fn ensure_codex_closed() -> Result<()> {
    use std::os::windows::process::CommandExt;
    let output = std::process::Command::new(
        std::path::PathBuf::from(
            std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()),
        )
        .join("System32/tasklist.exe"),
    )
    .creation_flags(0x08000000)
    .args(["/FI", "IMAGENAME eq Codex.exe", "/FO", "CSV", "/NH"])
    .output()
    .context("无法确认 Codex 是否已退出，已停止切换")?;
    if !output.status.success() {
        bail!("无法确认 Codex 是否已退出，已停止切换");
    }
    if String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.to_ascii_lowercase().starts_with("\"codex.exe\","))
    {
        bail!("请先关闭所有 Codex 窗口和 CLI 进程，再切换本地账号；当前认证文件未修改");
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct Account {
    pub which: String,
    pub email: Option<String>,
    pub label: String,
    pub legacy: bool,
}

fn read(path: &Path) -> Result<Zeroizing<Vec<u8>>> {
    use std::io::Read;
    safe(path)?;
    let file = fs::File::open(path).context("无法读取账号文件")?;
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        bail!("账号文件超过 1 MiB，未修改登录状态");
    }
    Ok(bytes)
}

fn metadata(bytes: &[u8]) -> Result<Account> {
    let value: Value =
        serde_json::from_slice(bytes).context("账号文件不是有效 JSON，未修改登录状态")?;
    fn valid_string(v: Option<&Value>) -> Option<&str> {
        v.and_then(Value::as_str).filter(|v| !v.trim().is_empty())
    }
    let (email, identity) = if let Some(key) = valid_string(value.get("OPENAI_API_KEY")) {
        (None, format!("api:{key}"))
    } else {
        let tokens = value.get("tokens").context("账号缺少 tokens 或 API Key")?;
        let id_token = valid_string(tokens.get("id_token")).context("账号缺少 id_token")?;
        valid_string(tokens.get("access_token")).context("账号缺少 access_token")?;
        valid_string(tokens.get("refresh_token")).context("账号缺少 refresh_token")?;
        let payload = id_token.split('.').nth(1).context("id_token 结构无效")?;
        let claims: Value = serde_json::from_slice(
            &URL_SAFE_NO_PAD
                .decode(payload.trim_end_matches('='))
                .context("id_token 结构无效")?,
        )
        .context("id_token 内容无效")?;
        let email = valid_string(claims.get("email")).map(str::to_owned);
        let identity = valid_string(tokens.get("account_id"))
            .or_else(|| valid_string(claims.get("sub")))
            .or(email.as_deref())
            .context("账号缺少可识别的账户标识")?
            .to_owned();
        (email, identity)
    };
    let which = format!(
        "{}.coc",
        crate::platform::digest::sha256_lower(identity.as_bytes())
    );
    Ok(Account {
        which,
        label: email.clone().unwrap_or_else(|| {
            if valid_string(value.get("OPENAI_API_KEY")).is_some() {
                "API Key 账号".into()
            } else {
                "未命名 Codex 账号".into()
            }
        }),
        email,
        legacy: false,
    })
}

fn migration_name(meta: &Account, bytes: &[u8]) -> String {
    let tag = crate::platform::digest::hex_lower(&crate::platform::digest::sha256(bytes)[..8]);
    format!("{}.{}.coc", meta.which.trim_end_matches(".coc"), tag)
}
fn snapshot_identifier(which: &str) -> bool {
    if which.len() > 85 {
        return false;
    }
    let pieces: Vec<_> = which.split('.').collect();
    let hex = |value: &str, size: usize| {
        value.len() == size && value.bytes().all(|b| b.is_ascii_hexdigit())
    };
    match pieces.as_slice() {
        [id, "coc"] => hex(id, 64),
        [id, tag, "coc"] => hex(id, 64) && hex(tag, 16),
        _ => false,
    }
}

pub struct Store {
    root: PathBuf,
}
impl Store {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    fn directory(&self) -> PathBuf {
        self.root.join("accounts-secure")
    }
    fn write_snapshot(&self, bytes: &[u8], name: Option<&str>) -> Result<Account> {
        let meta = metadata(bytes)?;
        let path = self.directory().join(name.unwrap_or(&meta.which));
        safe(&path)?;
        let protected = crypt(bytes, true)?;
        let mut saved = MAGIC.to_vec();
        saved.extend_from_slice(&protected);
        if saved.len() as u64 > LIMIT {
            bail!("加密后的账号快照超过大小上限，未写入快照");
        }
        crate::config::atomic_write(&path, &saved)?;
        if self.open(&path)?.as_slice() != bytes {
            bail!("加密快照读回不一致，已停止");
        }
        Ok(meta)
    }
    fn open(&self, path: &Path) -> Result<Zeroizing<Vec<u8>>> {
        let bytes = read(path)?;
        if !bytes.starts_with(MAGIC) {
            bail!("不是有效的加密账号快照");
        }
        let plain = crypt(&bytes[MAGIC.len()..], false)?;
        metadata(&plain)?;
        Ok(plain)
    }
    pub fn save(&self) -> Result<Account> {
        let _operation = OPERATION
            .lock()
            .map_err(|_| anyhow::anyhow!("账号操作锁不可用"))?;
        self.write_snapshot(&read(&self.root.join("auth.json"))?, None)
    }
    pub fn activate(&self, which: &str) -> Result<()> {
        let _operation = OPERATION
            .lock()
            .map_err(|_| anyhow::anyhow!("账号操作锁不可用"))?;
        if !snapshot_identifier(which) {
            bail!("请先加密迁移旧版快照，再选择有效的加密账号");
        }
        let replacement = self.open(&self.directory().join(which))?;
        let meta = metadata(&replacement)?;
        if meta.which != which && migration_name(&meta, &replacement) != which {
            bail!("快照与账号标识不一致，未修改登录状态");
        }
        let auth = self.root.join("auth.json");
        safe(&auth)?;
        let previous = if auth.exists() {
            let current = read(&auth)?;
            // 当前文件无效时保留原件供诊断，不让恢复备份被无效数据覆盖。
            if metadata(&current).is_ok() {
                self.write_snapshot(&current, None)?;
                self.write_snapshot(&current, Some("last-active.coc"))?;
            } else {
                bail!("当前认证文件无效，请先修复或通过官方客户端重新登录；未覆盖原文件");
            }
            Some(current)
        } else {
            None
        };
        // ReplaceFileW 保留已有认证文件的 DACL；失败后检查原件，必要时回滚。
        if let Err(failure) = write_auth(&auth, &replacement) {
            if let Some(previous) = previous {
                if read(&auth).ok().as_deref().map(|v| v.as_slice()) != Some(previous.as_slice()) {
                    write_auth(&auth, &previous)
                        .context("激活和回滚均未完成，加密恢复快照仍保留，请关闭 Codex 后重试")?;
                }
            }
            return Err(failure.context("账号激活未完成；原登录文件已保留或恢复"));
        }
        Ok(())
    }
    pub fn list(&self) -> Result<Vec<Account>> {
        let _operation = OPERATION
            .lock()
            .map_err(|_| anyhow::anyhow!("账号操作锁不可用"))?;
        let mut list = Vec::new();
        for (directory, legacy) in [
            (self.directory(), false),
            (self.root.join("accounts"), true),
        ] {
            safe(&directory)?;
            if !directory.exists() {
                continue;
            }
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                let path = entry.path();
                if !entry.file_type()?.is_file() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if (!legacy && (!name.ends_with(".coc") || name == "last-active.coc"))
                    || (legacy && !name.ends_with(".json"))
                {
                    continue;
                }
                let bytes = if legacy {
                    read(&path)?
                } else {
                    self.open(&path)?
                };
                let mut meta = metadata(&bytes)
                    .with_context(|| format!("账号快照 {name} 无效，未修改登录状态"))?;
                if !legacy && meta.which != name {
                    if migration_name(&meta, &bytes) != name {
                        bail!("加密快照标识不一致，当前账号未修改");
                    }
                    meta.label = format!(
                        "{} · 迁移副本 {}",
                        meta.label,
                        name.split('.').nth(1).unwrap_or_default()
                    );
                }
                meta.which = name;
                meta.legacy = legacy;
                list.push(meta);
            }
        }
        list.sort_by(|a, b| a.label.cmp(&b.label));
        Ok(list)
    }
    pub fn migrate(&self) -> Result<usize> {
        let _operation = OPERATION
            .lock()
            .map_err(|_| anyhow::anyhow!("账号操作锁不可用"))?;
        let directory = self.root.join("accounts");
        safe(&directory)?;
        if !directory.exists() {
            return Ok(0);
        }
        let mut sources = Vec::new();
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_file() && path.extension().is_some_and(|e| e == "json") {
                let bytes = read(&path)?;
                metadata(&bytes)?;
                sources.push((path, bytes));
            }
        }
        // 先检查全部来源；冲突的凭据版本也必须各自加密保留。
        for (_, bytes) in &sources {
            let meta = metadata(bytes)?;
            let target = self.directory().join(&meta.which);
            if target.exists() {
                let existing = self.open(&target)?;
                if metadata(&existing)?.which != meta.which {
                    bail!("已有加密快照标识不一致，旧明文已保留");
                }
                if existing.as_slice() != bytes.as_slice() {
                    self.write_snapshot(bytes, Some(&migration_name(&meta, bytes)))?;
                }
            } else {
                self.write_snapshot(bytes, None)?;
            }
        }
        for (path, bytes) in &sources {
            if read(path)?.as_slice() != bytes.as_slice() {
                bail!("迁移期间旧快照发生变化，加密副本已保存，变动的源文件保留，请重试");
            }
            fs::remove_file(path)
                .context("加密副本已保存，但旧明文删除失败，请检查文件权限后重试")?;
        }

        Ok(sources.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn auth(key: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"OPENAI_API_KEY":key})).unwrap()
    }
    #[test]
    fn snapshots_are_encrypted_and_refreshable() {
        let fixture = crate::test_support::FixtureDir::new();
        let store = Store::new(fixture.path().into());
        fs::write(fixture.path().join("auth.json"), auth("fixture-secret-A")).unwrap();
        let saved = store.save().unwrap();
        let bytes = fs::read(store.directory().join(&saved.which)).unwrap();
        assert!(!bytes.windows(16).any(|v| v == b"fixture-secret-A"));
        assert_eq!(store.list().unwrap().len(), 1);
        assert_eq!(store.save().unwrap().which, saved.which);
    }
    #[test]
    fn invalid_or_tampered_snapshot_never_overwrites_current_auth() {
        let fixture = crate::test_support::FixtureDir::new();
        let store = Store::new(fixture.path().into());
        let auth_path = fixture.path().join("auth.json");
        fs::write(&auth_path, auth("first")).unwrap();
        let saved = store.save().unwrap();
        fs::write(&auth_path, auth("current")).unwrap();
        let snapshot = store.directory().join(&saved.which);
        let mut bytes = fs::read(&snapshot).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        fs::write(snapshot, bytes).unwrap();
        assert!(store.activate(&saved.which).is_err());
        assert_eq!(fs::read(&auth_path).unwrap(), auth("current"));
        assert!(store.activate("../auth.json").is_err());
        assert!(metadata(b"{}").is_err());
        assert!(metadata(b"{bad").is_err());
    }
    #[test]
    fn atomic_activation_keeps_encrypted_rollback_and_migrates_legacy() {
        let fixture = crate::test_support::FixtureDir::new();
        let store = Store::new(fixture.path().into());
        let path = fixture.path().join("auth.json");
        fs::create_dir(fixture.path().join("accounts")).unwrap();
        fs::write(fixture.path().join("accounts/old.json"), auth("old")).unwrap();
        assert!(store.list().unwrap()[0].legacy);
        assert_eq!(store.migrate().unwrap(), 1);
        assert!(!fixture.path().join("accounts/old.json").exists());
        let target = store.list().unwrap().remove(0);
        fs::write(&path, auth("current")).unwrap();
        store.activate(&target.which).unwrap();
        assert_eq!(fs::read(&path).unwrap(), auth("old"));
        assert_eq!(
            store
                .open(&store.directory().join("last-active.coc"))
                .unwrap()
                .as_slice(),
            auth("current")
        );
    }
    #[test]
    fn invalid_legacy_input_does_not_remove_sources() {
        let fixture = crate::test_support::FixtureDir::new();
        let store = Store::new(fixture.path().into());
        fs::create_dir(fixture.path().join("accounts")).unwrap();
        fs::write(fixture.path().join("accounts/broken.json"), b"{}").unwrap();
        assert!(store.migrate().is_err());
        assert!(fixture.path().join("accounts/broken.json").exists());
    }
    #[test]
    fn concurrent_activation_always_leaves_a_complete_auth_file() {
        let fixture = crate::test_support::FixtureDir::new();
        let store = Store::new(fixture.path().into());
        let path = fixture.path().join("auth.json");
        fs::write(&path, auth("a")).unwrap();
        let a = store.save().unwrap().which;
        fs::write(&path, auth("b")).unwrap();
        let b = store.save().unwrap().which;
        std::thread::scope(|scope| {
            for i in 0..8 {
                let root = fixture.path().to_owned();
                let which = if i % 2 == 0 { a.clone() } else { b.clone() };
                scope.spawn(move || Store::new(root).activate(&which).unwrap());
            }
        });
        let final_auth = fs::read(path).unwrap();
        assert!(final_auth == auth("a") || final_auth == auth("b"));
    }
    #[test]
    fn failed_replace_keeps_current_auth_and_recovery_snapshot() {
        let fixture = crate::test_support::FixtureDir::new();
        let store = Store::new(fixture.path().into());
        let path = fixture.path().join("auth.json");
        fs::write(&path, auth("target")).unwrap();
        let target = store.save().unwrap();
        fs::write(&path, auth("current")).unwrap();
        let original_permissions = fs::metadata(&path).unwrap().permissions();
        let mut readonly = original_permissions.clone();
        readonly.set_readonly(true);
        fs::set_permissions(&path, readonly).unwrap();
        let result = store.activate(&target.which);
        fs::set_permissions(&path, original_permissions).unwrap();
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), auth("current"));
        assert!(store.directory().join("last-active.coc").exists());
    }
    #[test]
    fn activation_preserves_existing_file_acl() {
        let fixture = crate::test_support::FixtureDir::new();
        let store = Store::new(fixture.path().into());
        let path = fixture.path().join("auth.json");
        fs::write(&path, auth("target")).unwrap();
        let target = store.save().unwrap();
        fs::write(&path, auth("current")).unwrap();
        private_file(&path).unwrap();
        let acl = || {
            use std::os::windows::ffi::OsStrExt;
            use windows::Win32::Security::{
                DACL_SECURITY_INFORMATION, GetFileSecurityW, GetSecurityDescriptorControl,
                GetSecurityDescriptorDacl, PSECURITY_DESCRIPTOR, SE_DACL_PROTECTED,
            };
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let mut needed = 0;
            unsafe {
                let _ = GetFileSecurityW(
                    PCWSTR(wide.as_ptr()),
                    DACL_SECURITY_INFORMATION.0,
                    None,
                    0,
                    &mut needed,
                );
                assert!(needed > 0);
                let mut bytes = vec![0u8; needed as usize];
                GetFileSecurityW(
                    PCWSTR(wide.as_ptr()),
                    DACL_SECURITY_INFORMATION.0,
                    Some(PSECURITY_DESCRIPTOR(bytes.as_mut_ptr().cast())),
                    needed,
                    &mut needed,
                )
                .ok()
                .unwrap();
                let descriptor = PSECURITY_DESCRIPTOR(bytes.as_mut_ptr().cast());
                let mut control = 0;
                let mut revision = 0;
                GetSecurityDescriptorControl(descriptor, &mut control, &mut revision).unwrap();
                let mut present = windows::core::BOOL::default();
                let mut defaulted = windows::core::BOOL::default();
                let mut dacl = std::ptr::null_mut();
                GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted)
                    .unwrap();
                assert!(present.as_bool() && !dacl.is_null());
                (
                    control & SE_DACL_PROTECTED.0,
                    std::slice::from_raw_parts(dacl.cast::<u8>(), (*dacl).AclSize as usize)
                        .to_vec(),
                )
            }
        };
        let before = acl();
        store.activate(&target.which).unwrap();
        assert!(acl() == before, "认证文件的访问权限不应因替换而改变");
    }
    #[test]
    fn migration_preserves_conflicting_versions_and_old_last_active_snapshot() {
        fn oauth(access: &str) -> Vec<u8> {
            let claims = URL_SAFE_NO_PAD
                .encode(br#"{"email":"fixture@example.invalid","sub":"fixture-account"}"#);
            serde_json::to_vec(&serde_json::json!({"tokens":{"id_token":format!("e30.{claims}.fixture"),"access_token":access,"refresh_token":"fixture-refresh","account_id":"fixture-account"}})).unwrap()
        }
        let fixture = crate::test_support::FixtureDir::new();
        let store = Store::new(fixture.path().into());
        let auth_path = fixture.path().join("auth.json");
        let current = oauth("current");
        let legacy = oauth("legacy");
        let old_backup = oauth("old-backup");
        fs::write(&auth_path, &current).unwrap();
        let primary = store.save().unwrap();
        store
            .write_snapshot(&current, Some("last-active.coc"))
            .unwrap();
        fs::create_dir(fixture.path().join("accounts")).unwrap();
        fs::write(fixture.path().join("accounts/legacy.json"), &legacy).unwrap();
        fs::write(
            fixture.path().join("accounts/last_active.bak.json"),
            &old_backup,
        )
        .unwrap();
        assert_eq!(store.migrate().unwrap(), 2);
        assert_eq!(store.list().unwrap().len(), 3);
        assert_eq!(
            store
                .open(&store.directory().join(&primary.which))
                .unwrap()
                .as_slice(),
            current
        );
        assert_eq!(
            store
                .open(&store.directory().join("last-active.coc"))
                .unwrap()
                .as_slice(),
            current
        );
        assert_eq!(fs::read(&auth_path).unwrap(), current);
        let migrated = migration_name(&metadata(&legacy).unwrap(), &legacy);
        store.activate(&migrated).unwrap();
        assert_eq!(fs::read(&auth_path).unwrap(), legacy);
        let backup = migration_name(&metadata(&old_backup).unwrap(), &old_backup);
        assert!(store.open(&store.directory().join(backup)).is_ok());
    }
    #[test]
    fn malformed_oauth_and_unicode_identifiers_are_rejected() {
        assert!(safe(Path::new("auth.json")).is_err());
        assert!(metadata(br#"{"tokens":{"access_token":"fixture"}}"#).is_err());
        let fixture = crate::test_support::FixtureDir::new();
        let store = Store::new(fixture.path().into());
        assert!(store.activate("legacy.json").is_err());
        assert!(
            store
                .activate(&format!("{}中.coc", "a".repeat(61)))
                .is_err()
        );
    }
}
