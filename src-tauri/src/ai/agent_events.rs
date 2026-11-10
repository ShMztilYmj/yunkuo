//! 官方 lifecycle hooks 的本机连接。写入观察脚本、保留原有 hooks，备份后原子合并。
//! 事件文件只含白名单元数据；不监听端口，不执行日志中的命令或 transcript 路径。
use crate::ai::aimon::{AiUsageItem, activity::AiSession};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const MARKER: &str = "-YunKuoTool";
/// 支持官方 hooks 的工具来自名单，不再在这里维护。
fn hook_tools() -> Vec<&'static str> {
    crate::ai::agent_roster::hook_keys()
}
fn is_hook_tool(key: &str) -> bool {
    crate::ai::agent_roster::hook_config(key).is_some()
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationStatus {
    key: String,
    connected: bool,
    method: String,
    note: String,
}
fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default())
}
fn root() -> PathBuf {
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("YunKuo/agent-monitor")
}
fn config_path(base: &Path, key: &str) -> PathBuf {
    base.join(crate::ai::agent_roster::hook_config(key).unwrap_or("unsupported"))
}
fn config_value(path: &Path) -> Result<Value, String> {
    match std::fs::read(path) {
        Ok(bytes) if bytes.len() <= 4 * 1024 * 1024 => serde_json::from_slice(&bytes)
            .map_err(|_| "原有 hook 配置不是有效 JSON，已保持原文件；请在工具内检查配置".into()),
        Ok(_) => Err("hook 配置过大，已保持原文件".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(_) => Err("无法读取现有 hook 配置，已保持原文件".into()),
    }
}
fn owned(value: &Value) -> bool {
    match value {
        Value::String(s) => s.contains(MARKER) && s.contains("agent-event.ps1"),
        Value::Array(items) => items.iter().any(owned),
        Value::Object(items) => items.values().any(owned),
        _ => false,
    }
}
fn remove_owned(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items.iter_mut() {
                remove_owned(item);
            }
            items.retain(|item| {
                !item.get("command").is_some_and(owned)
                    && !item
                        .get("hooks")
                        .is_some_and(|hooks| hooks.as_array().is_some_and(Vec::is_empty))
            });
        }
        Value::Object(items) => {
            for item in items.values_mut() {
                remove_owned(item);
            }
        }
        _ => {}
    }
}
fn command(script: &Path, key: &str, event: &str) -> String {
    // 路径来自当前用户的应用数据目录，不能插入换行或命令引号。
    let path = script.to_string_lossy();
    format!(
        "powershell.exe -NoProfile -NonInteractive -WindowStyle Hidden -File \"{path}\" -YunKuoTool {key} -YunKuoEvent {event}"
    )
}
fn merge_config(mut value: Value, key: &str, script: &Path, enable: bool) -> Result<Value, String> {
    if !value.is_object() {
        return Err("原有 hook 配置结构无法识别，已保持原文件".into());
    }
    if enable && value["enabled"] == false {
        return Err("该工具已关闭全局 hooks，已尊重原设置；请先在工具中启用 hooks".into());
    }
    if key == "antigravity" {
        let name = "yun-kuo-agent-monitor";
        if value.get(name).is_some() && !owned(&value[name]) {
            return Err("存在同名自定义 hook，已保持原配置".into());
        }
        value.as_object_mut().unwrap().remove(name);
        if enable {
            let mut hooks = serde_json::Map::new();
            for event in ["SessionStart", "PreInvocation", "Stop"] {
                hooks.insert(
                    event.into(),
                    json!([{"type":"command","command":command(script,key,event),"timeout":3}]),
                );
            }
            hooks.insert("PreToolUse".into(),json!([{"matcher":".*","hooks":[{"type":"command","command":command(script,key,"PreToolUse"),"timeout":3}]}]));
            value[name] = Value::Object(hooks);
        }
    } else {
        if value.get("hooks").is_some_and(|hooks| !hooks.is_object()) {
            return Err("现有 hooks 不是事件字典，已保持原配置".into());
        }
        if value.get("hooks").is_none() {
            value["hooks"] = json!({});
        }
        remove_owned(&mut value["hooks"]);
        if enable {
            let events: &[&str] = if key == "cursor" {
                &[
                    "sessionStart",
                    "beforeSubmitPrompt",
                    "preToolUse",
                    "stop",
                    "sessionEnd",
                ]
            } else {
                &[
                    "SessionStart",
                    "UserPromptSubmit",
                    "PreToolUse",
                    "Stop",
                    "StopCancelled",
                    "StopFailure",
                    "SessionEnd",
                ]
            };
            for event in events {
                if value["hooks"].get(*event).is_none() {
                    value["hooks"][*event] = json!([]);
                }
                let Some(items) = value["hooks"][*event].as_array_mut() else {
                    return Err("现有 hook 事件不是数组，已保持原配置".into());
                };
                let cmd = command(script, key, event);
                items.push(if key == "cursor" {
                    json!({"command":cmd,"timeout":3})
                } else {
                    json!({"hooks":[{"type":"command","command":cmd,"timeout":3}]})
                });
            }
            if key == "cursor" && value.get("version").is_none() {
                value["version"] = 1.into();
            }
        }
    }
    Ok(value)
}
fn connected_at(base: &Path, directory: &Path, key: &str) -> bool {
    directory.join("agent-event.ps1").is_file()
        && config_value(&config_path(base, key)).is_ok_and(|value| owned(&value))
}
fn install_at(base: &Path, directory: &Path, key: &str, enable: bool) -> Result<(), String> {
    if !is_hook_tool(key) {
        return Err("此工具通过只读日志或数据库自动接入，无需写入 hooks".into());
    }
    let script = directory.join("agent-event.ps1");
    if script
        .to_string_lossy()
        .chars()
        .any(|c| matches!(c, '"' | '\n' | '\r' | '%' | '`' | '$'))
    {
        return Err("当前用户路径不适合生成 shell hook，请更换本地数据目录".into());
    }
    let path = config_path(base, key);
    let original = std::fs::read(&path).ok();
    let value = config_value(&path)?;
    let updated = merge_config(value, key, &script, enable)?;
    let bytes =
        serde_json::to_vec_pretty(&updated).map_err(|_| "无法生成 hook 配置".to_string())?;
    std::fs::create_dir_all(directory).map_err(|_| "无法创建本地状态目录".to_string())?;
    if enable {
        // Windows PowerShell 5.1 依靠 BOM 识别 UTF-8，避免中文元数据乱码。
        let mut content = vec![0xEF, 0xBB, 0xBF];
        content.extend_from_slice(include_bytes!("../agent-event.ps1"));
        crate::config::atomic_write(&script, &content)
            .map_err(|_| "无法保存状态采集脚本".to_string())?;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| "无法创建工具配置目录".to_string())?;
    }
    if std::fs::read(&path).ok() != original {
        return Err("工具配置在连接期间发生变化，已保留新文件，请重试".into());
    }
    if path.is_file() {
        let previous = std::fs::read(&path).map_err(|_| "无法备份原有 hooks".to_string())?;
        if previous == bytes {
            return Ok(());
        }
        // 每次变更保留精确恢复副本，停用仅移除本连接，不覆盖用户随后新增的设置。
        let backup = path.with_file_name(format!(
            "{}.yun-kuo-{}.bak",
            path.file_name().unwrap_or_default().to_string_lossy(),
            uuid::Uuid::new_v4()
        ));
        crate::config::atomic_write(&backup, &previous)
            .map_err(|_| "原有 hooks 备份失败，已保持原文件".to_string())?;
    }
    crate::config::atomic_write(&path, &bytes)
        .map_err(|_| "hook 配置写入失败，可用同目录备份恢复".to_string())
}
#[tauri::command]
pub fn agent_integration_status(app: tauri::AppHandle) -> Vec<IntegrationStatus> {
    let scope = crate::ai::agent_tools::MonitoringScope::current(&app);
    crate::ai::agent_tools::order()
        .iter()
        .filter(|key| scope.contains(key))
        .map(|key| {
            let hooks = is_hook_tool(key);
            let connected = !hooks || connected_at(&home(), &root(), key);
            IntegrationStatus {
                key: (*key).into(),
                connected,
                method: if hooks { "hooks" } else { "local" }.into(),
                note: if hooks {
                    if connected {
                        "状态连接已启用，新会话生效；Token 由统一用量引擎独立读取"
                    } else {
                        "启用官方 hooks 后读取状态；原有配置会先备份并保留"
                    }
                } else {
                    "自动只读本地会话或数据库；未发现记录时显示暂无数据"
                }
                .into(),
            }
        })
        .collect()
}
#[tauri::command]
pub fn agent_integration_connect(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    key: String,
    enabled: bool,
) -> Result<Vec<IntegrationStatus>, String> {
    if window.label() != "main" {
        return Err("请在主界面设置 Agent 连接".into());
    }
    if !crate::ai::agent_tools::MonitoringScope::current(&app).contains(&key) {
        return Err("请先添加该 Agent".into());
    }
    install_at(&home(), &root(), &key, enabled)?;
    Ok(agent_integration_status(app))
}
pub(crate) fn set_scope(scope: &crate::ai::agent_tools::MonitoringScope) -> Result<(), String> {
    let directory = root();
    std::fs::create_dir_all(&directory).map_err(|_| "无法创建 Agent 监测目录")?;
    // Refresh only our script; user-owned hook definitions are left intact.
    let script = [
        b"\xef\xbb\xbf".as_slice(),
        include_bytes!("../agent-event.ps1").as_slice(),
    ]
    .concat();
    crate::config::atomic_write(&directory.join("agent-event.ps1"), &script)
        .map_err(|_| "无法更新状态连接脚本")?;
    let keys: Vec<_> = scope.keys().collect();
    crate::config::atomic_write(
        &directory.join("scope.json"),
        &serde_json::to_vec(&keys).map_err(|_| "无法生成监测名单")?,
    )
    .map_err(|_| "无法保存 Agent 监测名单".into())
}
pub fn install_all() -> Result<(), String> {
    let mut errors = Vec::new();
    for key in hook_tools() {
        if let Err(error) = install_at(&home(), &root(), key, true) {
            errors.push(format!("{}：{error}", crate::ai::agent_tools::label(key)));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}
pub fn merge(items: &mut [AiUsageItem]) {
    merge_at(items, &root().join("events"));
    for item in items.iter_mut().filter(|item| is_hook_tool(&item.key)) {
        if !connected_at(&home(), &root(), &item.key) {
            for session in &mut item.recent_sessions {
                if matches!(session.state.as_str(), "running" | "asking") {
                    session.state = "unknown".into();
                }
            }
        }
    }
}
fn merge_at(items: &mut [AiUsageItem], directory: &Path) {
    let now = chrono::Utc::now().timestamp();
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.take(20000).flatten() {
        let path = entry.path();
        if !items.iter().any(|item| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(&format!("{}-", item.key))
        }) {
            continue;
        }
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        if entry
            .file_type()
            .is_ok_and(|kind| !kind.is_file() || kind.is_symlink())
        {
            continue;
        }
        if entry.metadata().map_or(true, |meta| meta.len() > 16 * 1024) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let key = value["tool"].as_str().unwrap_or_default();
        if !is_hook_tool(key)
            || !entry
                .file_name()
                .to_string_lossy()
                .starts_with(&format!("{key}-"))
        {
            continue;
        }
        let Some(item) = items.iter_mut().find(|item| item.key == key) else {
            continue;
        };
        let time = value["timestamp"].as_i64().unwrap_or(0) / 1000;
        if time < now - 30 * 86400 || time > now + 60 {
            continue;
        }
        let session = value["session"].as_str().unwrap_or_default();
        if session.is_empty() || session.len() > 256 {
            continue;
        }
        let id = format!("{key}:{session}");
        let mut state = value["state"].as_str().unwrap_or("unknown");
        if !matches!(
            state,
            "running" | "asking" | "completed" | "ended" | "unknown"
        ) {
            continue;
        }
        if matches!(state, "running" | "asking") && now - time >= 1800 {
            state = "unknown";
        }
        let completed = if state == "completed" {
            value["completedAt"]
                .as_i64()
                .map(|value| value / 1000)
                .filter(|completed| *completed > 0 && *completed <= time)
        } else {
            None
        };
        if let Some(old) = item
            .recent_sessions
            .iter_mut()
            .find(|record| record.id == id)
        {
            if time >= old.last_activity_at {
                old.state = state.into();
                old.last_activity_at = time;
                old.completed_at = completed;
            }
        } else {
            item.recent_sessions.push(AiSession {
                id,
                tool: key.into(),
                project: value["project"]
                    .as_str()
                    .unwrap_or("本地会话")
                    .chars()
                    .take(80)
                    .collect(),
                model: value["model"]
                    .as_str()
                    .map(|s| s.chars().take(160).collect()),
                state: state.into(),
                last_activity_at: time,
                completed_at: completed,
                total: 0,
                usage_available: Some(false),
            });
        }
        item.session_available = true;
        item.last_activity_ts = Some(item.last_activity_ts.unwrap_or(0).max(time));
        if !item.usage_available {
            item.note = "已连接官方生命周期事件；本次尚未读取到可核验用量，显示 —".into();
        }
    }
    for item in items {
        item.recent_sessions
            .sort_by_key(|record| std::cmp::Reverse(record.last_activity_at));
        item.recent_sessions.truncate(100);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn install_is_idempotent_preserves_guards_and_disconnect_preserves_new_hooks() {
        let fixture = crate::test_support::FixtureDir::new();
        let directory = fixture.path().join("monitor");
        for key in hook_tools() {
            let path = config_path(fixture.path(), key);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                &path,
                if key == "antigravity" {
                    r#"{"guard":{"Stop":[{"command":"my-guard"}]}}"#
                } else {
                    r#"{"hooks":{"stop":[{"command":"my-guard"}]}}"#
                },
            )
            .unwrap();
            install_at(fixture.path(), &directory, key, true).unwrap();
            let first = std::fs::read(&path).unwrap();
            install_at(fixture.path(), &directory, key, true).unwrap();
            assert_eq!(first, std::fs::read(&path).unwrap());
            assert!(connected_at(fixture.path(), &directory, key));
            assert!(String::from_utf8(first).unwrap().contains("my-guard"));
            install_at(fixture.path(), &directory, key, false).unwrap();
            assert!(!owned(&config_value(&path).unwrap()));
            assert!(std::fs::read_to_string(path).unwrap().contains("my-guard"));
        }
    }
    #[test]
    fn only_status_records_keep_their_source_and_never_fabricate_token_usage() {
        let fixture = crate::test_support::FixtureDir::new();
        let now = chrono::Utc::now().timestamp_millis();
        let mut items: Vec<_> = hook_tools()
            .into_iter()
            .map(|key| AiUsageItem {
                key: key.into(),
                ..Default::default()
            })
            .collect();
        for key in hook_tools() {
            std::fs::write(fixture.path().join(format!("{key}-fixture.json")),json!({"version":1,"tool":key,"session":"same","project":"fixture","model":"claude","timestamp":now,"state":"running"}).to_string()).unwrap();
        }
        merge_at(&mut items, fixture.path());
        for item in items {
            assert!(!item.usage_available);
            assert!(item.session_available);
            assert_eq!(item.recent_sessions[0].tool, item.key);
            assert_eq!(item.recent_sessions[0].state, "running");
            assert_eq!(item.all.total, 0);
        }
    }
    #[test]
    fn invalid_configuration_is_never_replaced() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = config_path(fixture.path(), "cursor");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "invalid").unwrap();
        assert!(
            install_at(
                fixture.path(),
                &fixture.path().join("monitor"),
                "cursor",
                true
            )
            .is_err()
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), "invalid");
    }
    #[test]
    fn removing_connector_keeps_user_hook_in_same_group_and_disabled_policy() {
        let script = Path::new("C:/local/agent-event.ps1");
        let mut value = json!({"hooks":{"Stop":[{"hooks":[{"command":command(script,"grok","Stop")},{"command":"user-guard"}]}]}});
        remove_owned(&mut value);
        assert_eq!(
            value["hooks"]["Stop"][0]["hooks"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            value["hooks"]["Stop"][0]["hooks"][0]["command"],
            "user-guard"
        );
        assert!(merge_config(json!({"enabled":false}), "cursor", script, true).is_err());
        assert!(merge_config(json!({"enabled":false}), "cursor", script, false).is_ok());
    }
}
