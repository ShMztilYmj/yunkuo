//! 有界、脱敏故障日志。panic payload 和用户路径永不写入；日志失败不得引起二次崩溃。
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

const LIMIT: u64 = 128 * 1024;
static JOURNAL: OnceLock<Arc<Journal>> = OnceLock::new();

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Panic,
    StartupFailed,
    Render,
    Rejection,
    Script,
    UncleanExit,
}

pub struct Journal {
    root: PathBuf,
    component: &'static str,
    writing: Mutex<()>,
}

fn safe(path: &Path) -> io::Result<()> {
    use std::os::windows::fs::MetadataExt;
    if !path.is_absolute() {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) if meta.file_attributes() & 0x400 != 0 => {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            Ok(_) => (),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

impl Journal {
    pub fn new(root: PathBuf, component: &'static str) -> Self {
        Self {
            root,
            component,
            writing: Mutex::new(()),
        }
    }
    pub fn record(&self, kind: Kind, location: Option<(&str, u32)>) -> io::Result<()> {
        // panic 可能发生在其他日志正在写时；宁可丢一条，也不死锁。
        let Some(_writing) = self.writing.try_lock() else {
            return Ok(());
        };
        let path = self.root.join("faults.jsonl");
        let previous = self.root.join("faults-previous.jsonl");
        safe(&path)?;
        safe(&previous)?;
        fs::create_dir_all(&self.root)?;
        if fs::metadata(&path).is_ok_and(|meta| meta.len() >= LIMIT) {
            if previous.exists() {
                fs::remove_file(&previous)?;
            }
            fs::rename(&path, previous)?;
        }
        let source = location.and_then(|(file, line)| {
            let name = file.rsplit(['/', '\\']).next()?;
            (name.len() <= 80
                && name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)))
            .then(|| serde_json::json!({"file":name,"line":line}))
        });
        let value = serde_json::json!({"schema":1,"at":chrono::Utc::now().to_rfc3339(),"version":env!("CARGO_PKG_VERSION"),"component":self.component,"kind":kind,"source":source});
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        serde_json::to_writer(&mut file, &value)?;
        file.write_all(b"\n")?;
        file.sync_data()
    }
    fn start(&self) -> io::Result<bool> {
        let marker = self.root.join("session-active");
        safe(&marker)?;
        fs::create_dir_all(&self.root)?;
        let unclean = marker.exists();
        fs::write(marker, b"1")?;
        if unclean {
            self.record(Kind::UncleanExit, None)?;
        }
        Ok(unclean)
    }
    fn finish(&self) {
        let marker = self.root.join("session-active");
        if safe(&marker).is_ok() {
            let _ = fs::remove_file(marker);
        }
    }
}

pub fn install(root: PathBuf, component: &'static str) {
    // 排障开关：只有显式设置 COC_PANIC_DETAIL 才把 panic 文本落盘，
    // 默认仍然只记位置，避免把用户路径和内部错误原文写进日志。
    let detail = std::env::var_os("COC_PANIC_DETAIL").map(|_| root.clone());
    let journal = Arc::new(Journal::new(root, component));
    if JOURNAL.set(journal.clone()).is_err() {
        return;
    }
    std::panic::set_hook(Box::new(move |info| {
        let source = info
            .location()
            .map(|location| (location.file(), location.line()));
        let _ = journal.record(Kind::Panic, source);
        if let Some(root) = &detail {
            let text = info
                .payload()
                .downcast_ref::<&str>()
                .map(|text| (*text).to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "<非字符串 panic>".into());
            let _ = fs::write(root.join("last-panic.txt"), text.as_bytes());
        }
    }));
}
pub fn start() -> bool {
    JOURNAL
        .get()
        .is_some_and(|journal| journal.start().unwrap_or(false))
}
pub fn record(kind: Kind) {
    if let Some(journal) = JOURNAL.get() {
        let _ = journal.record(kind, None);
    }
}
pub fn finish() {
    if let Some(journal) = JOURNAL.get() {
        journal.finish();
    }
}
pub fn app_root() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|path| path.join("Yunkuo/diagnostics"))
}

/// 发行包故障注入仅操作调用者提供的临时目录，先于正常 setup 入口。
pub fn panic_probe(root: PathBuf) -> ! {
    use windows::Win32::System::Diagnostics::Debug::{
        SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SetErrorMode,
    };
    unsafe {
        SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX);
    }
    install(root, "self-test");
    start();
    panic!("fixture-secret-must-not-be-recorded");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_records_preserve_only_safe_metadata() {
        let fixture = crate::test_support::FixtureDir::new();
        let journal = Journal::new(fixture.path().to_path_buf(), "test");
        for _ in 0..1200 {
            journal
                .record(Kind::Render, Some(("C:/private/account/fixture.rs", 42)))
                .unwrap();
        }
        for name in ["faults.jsonl", "faults-previous.jsonl"] {
            let bytes = fs::read(fixture.path().join(name)).unwrap();
            assert!(bytes.len() < LIMIT as usize + 1024);
            let text = String::from_utf8(bytes).unwrap();
            assert!(!text.contains("private"));
            for line in text.lines() {
                let value: serde_json::Value = serde_json::from_str(line).unwrap();
                assert_eq!(value["source"]["file"], "fixture.rs");
                assert_eq!(value["kind"], "render");
            }
        }
    }
    #[test]
    fn journal_failure_and_busy_writer_do_not_panic() {
        let fixture = crate::test_support::FixtureDir::new();
        let blocked = fixture.path().join("file");
        fs::write(&blocked, b"fixture").unwrap();
        assert!(
            Journal::new(blocked.join("dir"), "test")
                .record(Kind::Panic, None)
                .is_err()
        );
        let journal = Journal::new(fixture.path().to_path_buf(), "test");
        let _guard = journal.writing.lock();
        assert!(journal.record(Kind::Panic, None).is_ok());
    }
    #[test]
    fn unclean_marker_is_not_a_claim_of_known_crash() {
        let fixture = crate::test_support::FixtureDir::new();
        let journal = Journal::new(fixture.path().to_path_buf(), "test");
        assert!(!journal.start().unwrap());
        assert!(journal.start().unwrap());
        journal.finish();
        assert!(!journal.start().unwrap());
        journal.finish();
    }
    #[test]
    fn panic_child() {
        if let Some(root) = std::env::var_os("COC_FAULT_TEST_ROOT") {
            install(root.into(), "test");
            panic!("fixture-secret-must-not-be-recorded");
        }
    }
    #[test]
    fn panic_hook_writes_in_isolated_process() {
        use std::os::windows::process::CommandExt;
        let fixture = crate::test_support::FixtureDir::new();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "faults::tests::panic_child", "--nocapture"])
            .env("COC_FAULT_TEST_ROOT", fixture.path())
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(!result.status.success());
        let text = fs::read_to_string(fixture.path().join("faults.jsonl")).unwrap();
        assert!(text.contains("panic"));
        assert!(!text.contains("fixture-secret"));
    }
}
