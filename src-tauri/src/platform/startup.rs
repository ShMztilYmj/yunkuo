//! WebView2 必须能写入用户数据目录；目录存在不代表当前进程具有写入权限。
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

pub(crate) struct WebviewProfile {
    pub directory: PathBuf,
    pub fallback: bool,
}

fn check_writable(directory: &Path) -> io::Result<()> {
    // WebView2 实际使用 EBWebView 子目录；它可能具有不同于父目录的权限。
    for folder in [directory.to_path_buf(), directory.join("EBWebView")] {
        fs::create_dir_all(&folder)?;
        let probe = folder.join(format!(".yunkuo-write-probe-{}", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&probe)?;
            file.write_all(b"WebView2 startup check")
        })();
        let removed = fs::remove_file(&probe);
        result?;
        removed?;
    }
    Ok(())
}

fn select_profile(
    explicit: Option<PathBuf>,
    default: Option<PathBuf>,
    fallback: Option<PathBuf>,
    mut prepare: impl FnMut(&Path) -> io::Result<()>,
) -> Result<WebviewProfile, String> {
    if let Some(directory) = explicit {
        prepare(&directory).map_err(|error| {
            format!(
                "指定的 WebView2 数据目录无法写入：\n{}\n\n{error}\n\n请修正 WEBVIEW2_USER_DATA_FOLDER 或移除该环境变量后重试。",
                directory.display()
            )
        })?;
        return Ok(WebviewProfile {
            directory,
            fallback: false,
        });
    }
    let mut failures = Vec::new();
    for (directory, is_fallback) in default
        .into_iter()
        .map(|directory| (directory, false))
        .chain(fallback.into_iter().map(|directory| (directory, true)))
    {
        match prepare(&directory) {
            Ok(()) => {
                return Ok(WebviewProfile {
                    directory,
                    fallback: is_fallback,
                });
            }
            Err(error) => failures.push(format!("{}：{error}", directory.display())),
        }
    }
    Err(format!(
        "WebView2 数据目录无法写入，已停止启动以避免白屏。\n\n{}\n\n请从资源管理器启动软件，或将 WEBVIEW2_USER_DATA_FOLDER 指向当前启动环境允许写入的目录。",
        failures.join("\n")
    ))
}

pub(crate) fn prepare_webview() -> Result<WebviewProfile, String> {
    let explicit = std::env::var_os("WEBVIEW2_USER_DATA_FOLDER")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let default = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|root| root.join("Yunkuo/webview"));
    let fallback = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|root| root.join(".yunkuo/webview")));
    let selected = select_profile(explicit, default, fallback, check_writable)?;
    // SAFETY: 仅在正式入口、Tauri 和所有后台线程创建之前调用。
    unsafe { std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", &selected.directory) };
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_but_unwritable_default_uses_writable_fallback() {
        let default = PathBuf::from("default-profile");
        let fallback = PathBuf::from("portable-profile");
        let mut checked = Vec::new();
        let selected = select_profile(
            None,
            Some(default.clone()),
            Some(fallback.clone()),
            |path| {
                checked.push(path.to_path_buf());
                if path == default {
                    Err(io::ErrorKind::PermissionDenied.into())
                } else {
                    Ok(())
                }
            },
        )
        .unwrap();
        assert_eq!(checked, vec![default, fallback.clone()]);
        assert_eq!(selected.directory, fallback);
        assert!(selected.fallback);
    }

    #[test]
    fn writable_default_preserves_the_existing_profile() {
        let default = PathBuf::from("default-profile");
        let selected = select_profile(
            None,
            Some(default.clone()),
            Some("fallback".into()),
            |path| {
                assert_eq!(path, default);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(selected.directory, default);
        assert!(!selected.fallback);
    }

    #[test]
    fn explicit_profile_failure_is_reported_without_changing_profiles() {
        let error = select_profile(
            Some("explicit-profile".into()),
            Some("default".into()),
            Some("fallback".into()),
            |path| {
                assert_eq!(path, Path::new("explicit-profile"));
                Err(io::ErrorKind::PermissionDenied.into())
            },
        )
        .err()
        .unwrap();
        assert!(error.contains("WEBVIEW2_USER_DATA_FOLDER"));
        assert!(error.contains("explicit-profile"));
    }

    #[test]
    fn no_writable_profile_returns_an_actionable_error() {
        let error = select_profile(
            None,
            Some("default".into()),
            Some("fallback".into()),
            |_| Err(io::ErrorKind::PermissionDenied.into()),
        )
        .err()
        .unwrap();
        assert!(error.contains("已停止启动"));
        assert!(error.contains("default"));
        assert!(error.contains("fallback"));
    }

    #[test]
    fn writable_check_verifies_the_browser_subdirectory_and_cleans_probes() {
        let fixture = crate::test_support::FixtureDir::new();
        check_writable(fixture.path()).unwrap();
        assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 1);
        assert_eq!(
            fs::read_dir(fixture.path().join("EBWebView"))
                .unwrap()
                .count(),
            0
        );
        fs::remove_dir(fixture.path().join("EBWebView")).unwrap();
        fs::write(fixture.path().join("EBWebView"), b"blocked by a file").unwrap();
        assert!(check_writable(fixture.path()).is_err());
    }
}
