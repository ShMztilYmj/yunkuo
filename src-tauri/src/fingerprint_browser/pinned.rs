//! 把随包扩展固定到工具栏。
//!
//! 引擎用 `--load-extension=<资源目录>/extensions/webrtc-control` 装载解包扩展。
//! 解包扩展默认以路径推导 ID：换个安装目录（免安装包每次构建换目录、换盘、换机器）
//! 就是另一个 ID，之前固定到工具栏的按钮随之消失。清单里的 `key` 把 ID 钉成由公钥
//! 推导的固定值，这里再把这个 ID 写进资料的 `Default/Preferences`：
//! `extensions.pinned_extensions` 是引擎判断“图标是否留在工具栏”的唯一依据。
//!
//! 写入发生在引擎启动之前，所以第一次打开资料就能看到按钮；已固定时不动文件，其余
//! 偏好原样保留。换了安装目录留下的历史 ID 不再对应任何扩展，顺手从列表里清掉。
use anyhow::{Context, Result};
use base64::Engine as _;
use serde_json::{Value, json};
use std::path::Path;

/// 随包扩展的目录名，也用来认领同一扩展在旧路径下登记过的 ID。
const NAME: &str = "webrtc-control";

/// 由清单里的 `key` 得到固定 ID；清单没有 `key` 时返回错误，绝不猜。
pub(crate) fn id(extension: &Path) -> Result<String> {
    let manifest = extension.join("manifest.json");
    let text = std::fs::read_to_string(&manifest)
        .with_context(|| format!("读取扩展清单失败：{}", manifest.display()))?;
    let manifest: Value = serde_json::from_str(&text).context("扩展清单不是合法 JSON")?;
    let key = manifest
        .get("key")
        .and_then(Value::as_str)
        .filter(|key| !key.is_empty())
        .context("扩展清单缺少 key，无法得到固定 ID")?;
    let der = base64::engine::general_purpose::STANDARD
        .decode(key)
        .context("扩展清单的 key 不是合法 base64")?;
    Ok(from_public_key(&der))
}

/// 与引擎一致：公钥 SHA-256 的前 16 字节，每个半字节映射成 a–p。
pub(crate) fn from_public_key(der: &[u8]) -> String {
    const ALPHABET: &[u8; 16] = b"abcdefghijklmnop";
    ring::digest::digest(&ring::digest::SHA256, der).as_ref()[..16]
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .map(|nibble| ALPHABET[usize::from(nibble)] as char)
        .collect()
}

/// 同一扩展在旧路径下登记过的 ID：换了安装目录或补上 `key` 之后，这些 ID 不再对应
/// 任何扩展，留在偏好里只会越积越多。只读 `Secure Preferences`，不写它（写入会被
/// 引擎的校验和判成损坏）。
fn retired(secure: &Path, current: &str) -> Vec<String> {
    let Ok(bytes) = std::fs::read(secure) else {
        return Vec::new();
    };
    let Ok(doc) = serde_json::from_slice::<Value>(&bytes) else {
        return Vec::new();
    };
    let Some(settings) = doc
        .get("extensions")
        .and_then(|extensions| extensions.get("settings"))
        .and_then(Value::as_object)
    else {
        return Vec::new();
    };
    settings
        .iter()
        .filter(|(id, entry)| {
            id.as_str() != current && entry.get("path").and_then(Value::as_str).is_some_and(ours)
        })
        .map(|(id, _)| id.clone())
        .collect()
}

/// 解包扩展的注册路径带 `\\?\` 前缀，按结尾的目录名认领即可。
fn ours(path: &str) -> bool {
    path.trim_end_matches(['\\', '/'])
        .rsplit(['\\', '/'])
        .next()
        == Some(NAME)
}

/// 幂等：已经固定时不动文件；文件缺失就新建，结构不认识就原样留给引擎。
pub(crate) fn ensure(profile: &Path, id: &str) -> Result<()> {
    let file = profile.join("Default").join("Preferences");
    crate::platform::files::safe(&file)?;
    let mut doc = match std::fs::read(&file) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) if value.is_object() => value,
            _ => return Ok(()),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(error) => return Err(error.into()),
    };

    let retired = retired(&profile.join("Default").join("Secure Preferences"), id);
    let Some(root) = doc.as_object_mut() else {
        return Ok(());
    };
    let extensions = root.entry("extensions").or_insert_with(|| json!({}));
    let Some(extensions) = extensions.as_object_mut() else {
        return Ok(());
    };
    let pinned = extensions
        .entry("pinned_extensions")
        .or_insert_with(|| json!([]));
    let Some(pinned) = pinned.as_array_mut() else {
        return Ok(());
    };
    let before = pinned.clone();
    pinned.retain(|value| {
        !matches!(value.as_str(), Some(other) if retired.iter().any(|stale| stale == other))
    });
    if !pinned.iter().any(|value| value.as_str() == Some(id)) {
        pinned.push(json!(id));
    }
    if *pinned == before {
        return Ok(());
    }

    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::config::atomic_write(&file, &serde_json::to_vec(&doc)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 随包清单里写死的公钥，推导出的 ID 必须与它一致；ID 变了，用户已经固定好的
    /// 按钮就会重新缩回扩展菜单里。
    const SHIPPED: &str = "ppglokdnpgebfkcdhmllbkchncplpjme";

    fn scratch(tag: &str) -> std::path::PathBuf {
        // 落在 crate 的 target 下：系统临时目录在受控环境里可能不可写。
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/pinned-tests")
            .join(format!("{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn shipped() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/extensions/webrtc-control")
    }

    fn read(profile: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(profile.join("Default/Preferences")).unwrap())
            .unwrap()
    }

    #[test]
    fn shipped_manifest_derives_stable_id() {
        let id = id(&shipped()).unwrap();
        assert_eq!(id, SHIPPED);
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| ('a'..='p').contains(&c)));
    }

    #[test]
    fn pins_new_profile_and_keeps_other_preferences() {
        let dir = scratch("pin");
        std::fs::create_dir_all(dir.join("Default")).unwrap();
        std::fs::write(
            dir.join("Default/Preferences"),
            br#"{"bookmark_bar":{"show_on_all_tabs":true},"profile":{"name":"fixture"}}"#,
        )
        .unwrap();
        ensure(&dir, SHIPPED).unwrap();
        let doc = read(&dir);
        assert_eq!(doc["extensions"]["pinned_extensions"][0], SHIPPED);
        assert_eq!(doc["bookmark_bar"]["show_on_all_tabs"], true);
        assert_eq!(doc["profile"]["name"], "fixture");

        // 再跑一次不能再写文件，也不能把 ID 追加两遍。
        let before = std::fs::read(dir.join("Default/Preferences")).unwrap();
        ensure(&dir, SHIPPED).unwrap();
        assert_eq!(
            std::fs::read(dir.join("Default/Preferences")).unwrap(),
            before
        );
        assert_eq!(
            doc["extensions"]["pinned_extensions"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn creates_preferences_when_missing() {
        let dir = scratch("fresh");
        ensure(&dir, SHIPPED).unwrap();
        assert_eq!(read(&dir)["extensions"]["pinned_extensions"][0], SHIPPED);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn drops_ids_registered_under_the_old_path_only() {
        let dir = scratch("retire");
        std::fs::create_dir_all(dir.join("Default")).unwrap();
        let old = "ciefhpkdnlojkpmacbdjljbpdiehgidi";
        let other = "eepeadgljpkkjpbfecfkijnnliikglpl";
        let settings = json!({
            "extensions": {"settings": {
                old: {"path": "\\\\?\\C:\\legacy\\resources\\extensions\\webrtc-control", "location": 8},
                SHIPPED: {"path": "C:\\fixture\\resources\\extensions\\webrtc-control", "location": 8},
                other: {"path": "C:\\other-extension", "location": 8}
            }}
        });
        std::fs::write(
            dir.join("Default/Secure Preferences"),
            serde_json::to_vec(&settings).unwrap(),
        )
        .unwrap();
        std::fs::write(
            dir.join("Default/Preferences"),
            serde_json::to_vec(&json!({"extensions": {"pinned_extensions": [old, other]}}))
                .unwrap(),
        )
        .unwrap();

        ensure(&dir, SHIPPED).unwrap();
        let pinned = read(&dir)["extensions"]["pinned_extensions"].clone();
        let pinned: Vec<&str> = pinned
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(pinned, vec![other, SHIPPED]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn leaves_unreadable_preferences_alone() {
        let dir = scratch("keep");
        std::fs::create_dir_all(dir.join("Default")).unwrap();
        std::fs::write(dir.join("Default/Secure Preferences"), b"not json").unwrap();
        std::fs::write(dir.join("Default/Preferences"), b"not json").unwrap();
        ensure(&dir, SHIPPED).unwrap();
        assert_eq!(
            std::fs::read(dir.join("Default/Preferences")).unwrap(),
            b"not json"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
