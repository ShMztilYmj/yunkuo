//! 上次检测摘要：只服务展示，绝不参与启动门控。
//!
//! 与 `WebScanState` 的分工：门控凭据是内存态、300 秒 TTL、绑定运行代次的；
//! 这里落盘的只有「上次差什么」，结构上没有 `finished` / `native` /
//! `webResults`，因此不可能被 `computeGate` 读成一次新的放行。
//!
//! 失效判定不用 `EnvironmentBasis`：它含进程内代次，重启必然不等。改用可跨
//! 重启比较的配置语义摘要，配合前端现场读到的系统语言/时区与时间做提示。

use super::report::config_digest_hex;
use crate::config::FileStore;
use crate::platform::files::environment_root;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

const VERSION: u32 = 1;
const FILE: &str = "last-scan.json";
const MAX_FAILURES: usize = 20;
const MAX_FAILURE_CHARS: usize = 160;
const MAX_LANGUAGES: usize = 8;
const MAX_LANGUAGE_CHARS: usize = 40;

/// 当时的系统事实：只用来发现「系统改过了」，不当作当前状态展示。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScanSystem {
    /// Windows 时区键名，与 `NativeDetect.currentTz` 同一表示
    pub timezone: String,
    pub languages: Vec<String>,
    pub culture: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LastScan {
    version: u32,
    at: i64,
    score: i32,
    failures: Vec<String>,
    system: ScanSystem,
    config_digest: String,
}

/// 返回前端的视图：只读展示字段，外加「配置是否已变」。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastScanView {
    pub at: i64,
    pub score: i32,
    pub failures: Vec<String>,
    pub system: ScanSystem,
    pub config_changed: bool,
}

fn path(app: &AppHandle) -> Result<PathBuf, String> {
    environment_root(app)
        .map(|root| root.join(FILE))
        .map_err(|error| format!("{error:#}"))
}

/// 读不到、格式坏了、版本不认识、评分越界，一律当作「没有历史」。
fn read(path: &Path) -> Option<LastScan> {
    let last: LastScan = FileStore::new(path.to_path_buf()).load().ok()?;
    (last.version == VERSION && (0..=100).contains(&last.score)).then_some(last)
}

fn write(path: &Path, last: &LastScan) -> Result<(), String> {
    FileStore::new(path.to_path_buf())
        .save(last)
        .map_err(|error| format!("{error:#}"))
}

fn truncate(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_string();
    }
    trimmed.chars().take(limit).collect::<String>() + "…"
}

fn clean_failures(items: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in items {
        let text = truncate(&item, MAX_FAILURE_CHARS);
        if text.is_empty() || !seen.insert(text.clone()) {
            continue;
        }
        out.push(text);
        if out.len() == MAX_FAILURES {
            break;
        }
    }
    out
}

fn clean_system(system: ScanSystem) -> ScanSystem {
    let mut seen = HashSet::new();
    let languages = system
        .languages
        .iter()
        .map(|language| truncate(language, MAX_LANGUAGE_CHARS))
        .filter(|language| !language.is_empty() && seen.insert(language.clone()))
        .take(MAX_LANGUAGES)
        .collect();
    ScanSystem {
        timezone: truncate(&system.timezone, MAX_LANGUAGE_CHARS),
        languages,
        culture: truncate(&system.culture, MAX_LANGUAGE_CHARS),
    }
}

#[tauri::command]
pub async fn get_last_scan(app: AppHandle) -> Result<Option<LastScanView>, String> {
    let digest = {
        let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
        config_digest_hex(&cfg)
    };
    let last =
        tauri::async_runtime::spawn_blocking(move || path(&app).ok().and_then(|file| read(&file)))
            .await
            .map_err(|error| error.to_string())?;
    Ok(last.map(|last| LastScanView {
        at: last.at,
        score: last.score,
        failures: last.failures,
        system: last.system,
        config_changed: last.config_digest != digest,
    }))
}

#[tauri::command]
pub async fn save_last_scan(
    app: AppHandle,
    score: i32,
    failures: Vec<String>,
    system: ScanSystem,
) -> Result<(), String> {
    if !(0..=100).contains(&score) {
        return Err("环境评分必须在 0–100 之间，请重新检测".into());
    }
    let digest = {
        let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
        config_digest_hex(&cfg)
    };
    let last = LastScan {
        version: VERSION,
        at: chrono::Utc::now().timestamp_millis(),
        score,
        failures: clean_failures(failures),
        system: clean_system(system),
        config_digest: digest,
    };
    tauri::async_runtime::spawn_blocking(move || {
        let file = path(&app)?;
        write(&file, &last)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> LastScan {
        LastScan {
            version: VERSION,
            at: 1_700_000_000_000,
            score: 95,
            failures: vec!["TUN 未实际开启".into()],
            system: ScanSystem {
                timezone: "Central Standard Time".into(),
                languages: vec!["en-US".into()],
                culture: "en-US".into(),
            },
            config_digest: "0123456789abcdef".into(),
        }
    }

    #[test]
    fn summary_round_trips_and_rejects_foreign_versions() {
        let fixture = crate::test_support::FixtureDir::new();
        let file = fixture.path().join(FILE);
        let last = sample();
        write(&file, &last).unwrap();
        assert_eq!(read(&file).expect("摘要应能读回"), last);

        let mut unknown = last.clone();
        unknown.version = VERSION + 1;
        write(&file, &unknown).unwrap();
        assert!(read(&file).is_none(), "版本不认识时按没有历史处理");

        let mut out_of_range = last.clone();
        out_of_range.score = 101;
        write(&file, &out_of_range).unwrap();
        assert!(read(&file).is_none(), "越界评分不能当作历史展示");
    }

    #[test]
    fn missing_file_is_not_an_error() {
        let fixture = crate::test_support::FixtureDir::new();
        assert!(read(&fixture.path().join(FILE)).is_none());
    }

    #[test]
    fn failures_and_system_facts_are_bounded() {
        let long = "x".repeat(400);
        let cleaned = clean_failures(vec![long.clone(), long, "   ".into(), "内核未运行".into()]);
        assert_eq!(cleaned.len(), 2, "空白项丢弃、重复项合并");
        assert_eq!(cleaned[0].chars().count(), MAX_FAILURE_CHARS + 1);

        let many: Vec<String> = (0..40).map(|index| format!("项 {index}")).collect();
        assert_eq!(clean_failures(many).len(), MAX_FAILURES);

        let system = clean_system(ScanSystem {
            timezone: "  Central Standard Time  ".into(),
            culture: " en-US ".into(),
            languages: (0..12).map(|index| format!("lang-{index}")).collect(),
        });
        assert_eq!(system.timezone, "Central Standard Time");
        assert_eq!(system.culture, "en-US");
        assert_eq!(system.languages.len(), MAX_LANGUAGES);
    }
}
