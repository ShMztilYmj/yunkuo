//! 指纹浏览器默认收藏。
//!
//! 引擎始终以 `--user-data-dir=<slot>/profile --profile-directory=Default` 启动，
//! 收藏栏即该资料目录下的 `Default/Bookmarks`。启动前补齐一份 Chromium 结构的
//! 书签文件：文件缺失时整体生成，已存在时只追加尚未收藏的站点，用户自己的条目
//! 原样保留。写入后删去 `checksum`，交给引擎在下次保存时重算。
use anyhow::Result;
use serde_json::{Value, json};
use std::{collections::HashSet, path::Path};

/// 预置在收藏栏的站点；数组顺序即收藏栏顺序。
pub(crate) const DEFAULTS: &[(&str, &str)] = &[
    ("IPPure 纯净度", "https://ippure.com/"),
    ("IP 质量检测", "https://ip.net.coffee/"),
    ("Gmail", "https://mail.google.com/"),
    ("Claude", "https://claude.ai/"),
    ("ChatGPT", "https://chatgpt.com/"),
    ("Google 搜索", "https://www.google.com/"),
];

/// Chromium 内部的微秒时间戳（自 1601-01-01 起）。
const STAMP: &str = "13402000000000000";
const BAR_ID: i64 = 1;
const OTHER_ID: i64 = 2;
const SYNCED_ID: i64 = 3;

fn guid(seed: i64) -> String {
    format!("00000000-0000-4000-8000-{seed:012x}")
}

fn url_node(id: i64, name: &str, url: &str) -> Value {
    json!({
        "date_added": STAMP,
        "date_last_used": "0",
        "guid": guid(id),
        "id": id.to_string(),
        "name": name,
        "type": "url",
        "url": url,
    })
}

fn folder(id: i64, name: &str, children: Vec<Value>) -> Value {
    json!({
        "children": children,
        "date_added": STAMP,
        "date_last_used": "0",
        "date_modified": STAMP,
        "guid": guid(id),
        "id": id.to_string(),
        "name": name,
        "type": "folder",
    })
}

fn document(bar: Vec<Value>) -> Value {
    json!({
        "roots": {
            "bookmark_bar": folder(BAR_ID, "Bookmarks bar", bar),
            "other": folder(OTHER_ID, "Other bookmarks", Vec::new()),
            "synced": folder(SYNCED_ID, "Mobile bookmarks", Vec::new()),
        },
        "version": 1,
    })
}

/// 去重口径：忽略大小写与结尾斜杠，`https://a.com` 与 `https://a.com/` 视为同一站点。
fn key(url: &str) -> String {
    url.trim().trim_end_matches('/').to_ascii_lowercase()
}

/// 统计已有站点与最大节点 id，避免与引擎分配的 id 冲突。
fn walk(value: &Value, urls: &mut HashSet<String>, max_id: &mut i64) {
    match value {
        Value::Object(map) => {
            if let Some(id) = map
                .get("id")
                .and_then(Value::as_str)
                .and_then(|raw| raw.parse::<i64>().ok())
            {
                *max_id = (*max_id).max(id);
            }
            if let Some(url) = map.get("url").and_then(Value::as_str) {
                urls.insert(key(url));
            }
            for child in map.values() {
                walk(child, urls, max_id);
            }
        }
        Value::Array(items) => {
            for item in items {
                walk(item, urls, max_id);
            }
        }
        _ => (),
    }
}

/// 让收藏栏在所有标签页可见；仅在该资料还没有偏好文件时写入，不改动既有偏好。
fn show_bar(profile: &Path) -> Result<()> {
    let file = profile.join("Default").join("Preferences");
    if file.try_exists()? {
        return Ok(());
    }
    crate::platform::files::safe(&file)?;
    std::fs::create_dir_all(file.parent().unwrap_or(profile))?;
    let prefs = json!({"bookmark_bar": {"show_on_all_tabs": true}});
    crate::config::atomic_write(&file, &serde_json::to_vec(&prefs)?)?;
    Ok(())
}

/// 幂等：同一资料反复调用只会在缺少站点时补齐。
pub(crate) fn ensure(profile: &Path) -> Result<()> {
    let file = profile.join("Default").join("Bookmarks");
    crate::platform::files::safe(&file)?;
    let mut doc = match std::fs::read(&file) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) if value.get("roots").is_some_and(Value::is_object) => value,
            // 结构不认识时不动原文件，交给引擎自己修复。
            _ => return Ok(()),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => document(Vec::new()),
        Err(error) => return Err(error.into()),
    };

    let (mut known, mut max_id) = (HashSet::new(), 0);
    walk(&doc, &mut known, &mut max_id);

    let mut added = false;
    {
        let Some(roots) = doc.get_mut("roots").and_then(Value::as_object_mut) else {
            return Ok(());
        };
        let bar = roots
            .entry("bookmark_bar")
            .or_insert_with(|| folder(BAR_ID, "Bookmarks bar", Vec::new()));
        let Some(bar) = bar.as_object_mut() else {
            return Ok(());
        };
        let children = bar
            .entry("children")
            .or_insert_with(|| Value::Array(Vec::new()));
        let Value::Array(children) = children else {
            return Ok(());
        };
        for (name, url) in DEFAULTS {
            if !known.insert(key(url)) {
                continue;
            }
            max_id += 1;
            children.push(url_node(max_id, name, url));
            added = true;
        }
        if !added {
            return Ok(());
        }
        bar.insert("date_modified".into(), Value::String(STAMP.into()));
    }

    if let Some(map) = doc.as_object_mut() {
        // 留着旧校验和会被引擎判成损坏文件，删掉由它在下次保存时重算。
        map.remove("checksum");
    }
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::config::atomic_write(&file, &serde_json::to_vec(&doc)?)?;
    show_bar(profile)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        // 落在 crate 的 target 下：系统临时目录在受控环境里可能不可写。
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/bookmark-tests")
            .join(format!("{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn read(dir: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(dir.join("Default/Bookmarks")).unwrap()).unwrap()
    }

    #[test]
    fn seeds_every_default_site_once() {
        let dir = scratch("seed");
        ensure(&dir).unwrap();
        ensure(&dir).unwrap();
        let doc = read(&dir);
        let bar = doc["roots"]["bookmark_bar"]["children"].as_array().unwrap();
        assert_eq!(bar.len(), DEFAULTS.len());
        for (name, url) in DEFAULTS {
            assert!(
                bar.iter()
                    .any(|node| node["url"] == *url && node["name"] == *name)
            );
        }
        assert!(doc.get("checksum").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeps_existing_bookmarks_and_ids() {
        let dir = scratch("merge");
        let existing = document(vec![url_node(11, "自建", "https://example.com/page")]);
        std::fs::create_dir_all(dir.join("Default")).unwrap();
        std::fs::write(
            dir.join("Default/Bookmarks"),
            serde_json::to_vec(&existing).unwrap(),
        )
        .unwrap();
        ensure(&dir).unwrap();
        let doc = read(&dir);
        let bar = doc["roots"]["bookmark_bar"]["children"].as_array().unwrap();
        assert_eq!(bar.len(), DEFAULTS.len() + 1);
        assert!(
            bar.iter()
                .any(|node| node["url"] == "https://example.com/page")
        );
        // 新节点沿用已有最大 id 之后，不能与既有节点重号。
        let mut ids: Vec<_> = bar
            .iter()
            .map(|node| node["id"].as_str().unwrap())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), DEFAULTS.len() + 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn leaves_unreadable_file_alone() {
        let dir = scratch("keep");
        std::fs::create_dir_all(dir.join("Default")).unwrap();
        std::fs::write(dir.join("Default/Bookmarks"), b"not json").unwrap();
        ensure(&dir).unwrap();
        assert_eq!(
            std::fs::read(dir.join("Default/Bookmarks")).unwrap(),
            b"not json"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
