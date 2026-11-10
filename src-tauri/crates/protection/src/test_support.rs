use std::path::{Path, PathBuf};

const PREFIX: &str = "claude-regression-";

/// 可写的测试根目录。
///
/// 系统临时目录在受限 ACL 的 CI、企业终端或沙箱里可能禁止创建子目录，
/// 因此实际探测一次；探测失败就退回到 workspace 的 `target/test-fixtures`。
pub fn scratch_root() -> PathBuf {
    let candidates = [
        std::env::temp_dir(),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/test-fixtures"),
    ];
    for root in candidates {
        if std::fs::create_dir_all(&root).is_err() {
            continue;
        }
        // 只在根目录"存在"不算数：必须真的能建出子目录。
        let probe = root.join(format!(".probe-{}", uuid::Uuid::new_v4()));
        if std::fs::create_dir(&probe).is_ok() {
            let _ = std::fs::remove_dir(&probe);
            return root;
        }
    }
    std::env::temp_dir()
}

/// 一次性测试目录，Drop 时自行清理。
pub struct FixtureDir {
    root: PathBuf,
    path: PathBuf,
}

impl FixtureDir {
    pub fn new() -> Self {
        let name = format!("{PREFIX}{}", uuid::Uuid::new_v4());
        let root = scratch_root();
        let path = root.join(&name);
        std::fs::create_dir(&path)
            .unwrap_or_else(|error| panic!("测试临时目录不可写：{}（{error}）", path.display()));
        Self { root, path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for FixtureDir {
    fn drop(&mut self) {
        let named = self
            .path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(PREFIX));
        let contained = match (self.path.canonicalize(), self.root.canonicalize()) {
            (Ok(path), Ok(root)) => path.starts_with(root),
            _ => false,
        };
        if named && contained {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
