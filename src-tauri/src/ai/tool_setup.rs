//! 工具入口与用户级配置的只读摘要。不读取认证文件，不向前端返回密钥。
use crate::config::AppConfig;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tauri::{AppHandle, Manager, WebviewWindow};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSummary {
    pub key: String,
    pub model: Option<String>,
    pub source: String,
    pub endpoint: Option<String>,
    pub note: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSetup {
    key: String,
    installed: Option<bool>,
    running: Option<bool>,
    configured: bool,
    shortcut_valid: bool,
    candidates: Vec<String>,
    connection: ConnectionSummary,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    tools: Vec<ToolSetup>,
    checked_at: i64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inspection {
    pub candidates: Vec<String>,
    pub version: Option<String>,
    pub note: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connectivity {
    pub reachable: bool,
    pub endpoint: String,
    pub status: Option<u16>,
    pub note: String,
}
/// 自动定位结果：`path` 只给出本机已存在的文件，前端保存前仍会再次校验。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Autodetect {
    pub path: Option<String>,
    pub candidates: Vec<String>,
    pub note: String,
}

fn allowed(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("此操作仅用于主界面的工具管理".into())
    }
}
fn supported(tool: &str) -> Result<()> {
    if !crate::config::AI_TOOL_KEYS.contains(&tool) {
        bail!("未知工具");
    }
    Ok(())
}
fn short(value: Option<&str>) -> Option<String> {
    value
        .filter(|v| !v.is_empty())
        .map(|v| v.chars().filter(|c| !c.is_control()).take(160).collect())
}
/// 用户在工具里自己的配置位置：`CLAUDE_CONFIG_DIR` / `CODEX_HOME` 优先。
fn user_config_path(tool: &str) -> Result<PathBuf> {
    let home = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .context("无法确认当前用户目录")?;
    let (variable, fallback, file) = match tool {
        "claude" => ("CLAUDE_CONFIG_DIR", ".claude", "settings.json"),
        "codex" | "chatgpt" => ("CODEX_HOME", ".codex", "config.toml"),
        _ => bail!("未知的工具"),
    };
    let directory = std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(fallback));
    let path = directory.join(file);
    crate::platform::secure_storage::safe(&path)?;
    Ok(path)
}
fn user_config(tool: &str) -> Result<Option<Vec<u8>>> {
    let path = user_config_path(tool)?;
    match std::fs::metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
        Ok(meta) => {
            if !meta.is_file() || meta.len() > 2 * 1024 * 1024 {
                bail!("用户配置过大或不是文件");
            }
            crate::platform::secure_storage::safe(&path)?;
            Ok(Some(std::fs::read(path)?))
        }
    }
}

// 仅保留地址，移除 userinfo/query/fragment；摘要和探测均不携带认证数据。
fn clean_endpoint(value: &str) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(value).context("接口地址格式无效")?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("接口地址必须为 HTTP 或 HTTPS");
    }
    if !url.username().is_empty() || url.password().is_some() || url.query().is_some() {
        bail!("接口地址含认证或查询参数，请在工具中单独配置认证");
    }
    url.set_fragment(None);
    Ok(url)
}
fn parse_connection(tool: &str, bytes: Option<&[u8]>) -> Result<ConnectionSummary> {
    let (model, base) = if tool == "claude" {
        let value: Value = bytes
            .map(serde_json::from_slice)
            .transpose()?
            .unwrap_or_else(|| serde_json::json!({}));
        if [
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
            "CLAUDE_CODE_USE_FOUNDRY",
        ]
        .iter()
        .any(|key| value["env"][key].as_str() == Some("1"))
        {
            bail!("云平台连接请在工具中确认");
        }
        (
            short(
                value["model"]
                    .as_str()
                    .or(value["env"]["ANTHROPIC_MODEL"].as_str()),
            ),
            value["env"]["ANTHROPIC_BASE_URL"]
                .as_str()
                .map(str::to_string),
        )
    } else {
        let value =
            std::str::from_utf8(bytes.unwrap_or_default())?.parse::<toml_edit::DocumentMut>()?;
        let profile = value
            .get("profile")
            .and_then(|v| v.as_str())
            .and_then(|name| value.get("profiles").and_then(|v| v.get(name)));
        let model = profile
            .and_then(|p| p.get("model"))
            .or_else(|| value.get("model"))
            .and_then(|v| v.as_str());
        let provider = profile
            .and_then(|p| p.get("model_provider"))
            .or_else(|| value.get("model_provider"))
            .and_then(|v| v.as_str())
            .unwrap_or("openai");
        let base = value
            .get("model_providers")
            .and_then(|v| v.get(provider))
            .and_then(|v| v.get("base_url"))
            .and_then(|v| v.as_str());
        if provider != "openai" && base.is_none() {
            bail!("自定义服务商未提供可确认的接口地址");
        }
        (short(model), base.map(str::to_string))
    };
    let url = base.as_deref().map(clean_endpoint).transpose()?;
    let source = if url.is_some() { "external" } else { "native" };
    Ok(ConnectionSummary {
        key: tool.into(),
        model,
        source: source.into(),
        endpoint: url.map(|v| v.origin().ascii_serialization()),
        note: if tool == "chatgpt" {
            "仅 Codex 用户级配置参考；快捷方式可能指向其他入口。项目配置、环境变量与启动参数可能覆盖，登录需在工具中确认。"
        } else {
            "用户级配置参考；项目配置、环境变量与启动参数可能覆盖，账号登录需在工具中确认。"
        }
        .into(),
    })
}
pub(crate) fn connection(tool: &str) -> ConnectionSummary {
    user_config(tool)
        .and_then(|bytes| parse_connection(tool, bytes.as_deref()))
        .unwrap_or_else(|_| ConnectionSummary {
            key: tool.into(),
            model: None,
            source: "unknown".into(),
            endpoint: None,
            note: "用户配置无法读取或格式无效；未猜测模型与连接方式。".into(),
        })
}
fn valid_shortcut(path: &str) -> bool {
    let path = Path::new(path);
    path.is_absolute()
        && !path.to_string_lossy().starts_with(r"\\")
        && path.is_file()
        && path.extension().and_then(|v| v.to_str()).is_some_and(|v| {
            ["exe", "cmd", "bat", "lnk"]
                .iter()
                .any(|ext| v.eq_ignore_ascii_case(ext))
        })
}
/// npm 的 `.cmd`/`.bat` 只是转发脚本，不是原生入口。
fn shim_script(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|ext| {
            ["cmd", "bat"]
                .iter()
                .any(|value| ext.eq_ignore_ascii_case(value))
        })
}
/// 只有“现有入口是 npm 转发脚本、本机又存在原生 EXE”时才换；手动选的 .lnk/.exe 一律保留。
fn upgrades_to_native(current: &str, best: &str) -> bool {
    extension_rank(best) == 0 && shim_script(current)
}
fn extension_rank(path: &str) -> u8 {
    match Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("exe") => 0,
        Some("cmd") => 1,
        Some("bat") => 2,
        Some("lnk") => 3,
        _ => 4,
    }
}
/// 自动挑选最直接的已安装入口：原生 EXE 优先于 npm 的 cmd/bat 转发脚本，
/// 同类候选取路径最短的一个，避免把深层缓存里的临时副本当成安装位置。
fn preferred_candidate(candidates: &[String]) -> Option<String> {
    candidates
        .iter()
        .filter(|path| valid_shortcut(path))
        .min_by(|left, right| {
            extension_rank(left)
                .cmp(&extension_rank(right))
                .then_with(|| left.len().cmp(&right.len()))
                .then_with(|| left.cmp(right))
        })
        .cloned()
}
fn snapshot(config: &AppConfig) -> Snapshot {
    let inventory = crate::ai::agent_tools::read(config).ok();
    let tools = config
        .tool_cards()
        .into_iter()
        .map(|key| {
            let process = inventory
                .as_ref()
                .and_then(|values| values.iter().find(|tool| tool.key == key));
            let shortcut = config.tool_shortcut(&key).unwrap_or_default().trim();
            let valid = valid_shortcut(shortcut);
            let connection = if ["claude", "chatgpt"].contains(&key.as_str()) {
                connection(&key)
            } else {
                ConnectionSummary {
                    key: key.clone(),
                    model: None,
                    source: "tool".into(),
                    endpoint: None,
                    note: "模型、连接与登录由工具自身管理。".into(),
                }
            };
            ToolSetup {
                candidates: crate::ai::agent_tools::launch_candidates(&key),
                installed: process.map(|p| p.installed || (valid && key != "chatgpt")),
                running: process.map(|p| p.online),
                configured: !shortcut.is_empty(),
                shortcut_valid: valid,
                key,
                connection,
            }
        })
        .collect();
    Snapshot {
        tools,
        checked_at: chrono::Utc::now().timestamp(),
    }
}
#[tauri::command]
pub async fn tool_setup_status(app: AppHandle, window: WebviewWindow) -> Result<Snapshot, String> {
    allowed(&window)?;
    let config = app.state::<crate::config::ConfigMutex>().lock().clone();
    tokio::task::spawn_blocking(move || snapshot(&config))
        .await
        .map_err(|_| "工具配置读取未完成".into())
}
#[tauri::command]
pub async fn tool_setup_inspect(
    app: AppHandle,
    window: WebviewWindow,
    tool: String,
) -> Result<Inspection, String> {
    allowed(&window)?;
    supported(&tool).map_err(|e| e.to_string())?;
    let config = app.state::<crate::config::ConfigMutex>().lock().clone();
    tokio::task::spawn_blocking(move || -> Result<Inspection, String> {
        let candidates = crate::ai::agent_tools::selectable_candidates(&tool);
        let path = config.tool_shortcut(&tool).filter(|p| valid_shortcut(p));
        let version = path.filter(|p| Path::new(p).extension().is_some_and(|v| v.eq_ignore_ascii_case("exe"))).and_then(|path| {
            crate::platform::files::shell("$ErrorActionPreference='Stop'; (Get-Item -LiteralPath $env:COC_TOOL_FILE).VersionInfo.FileVersion", &[("COC_TOOL_FILE", path)]).ok()
                .and_then(|value| short(Some(value.trim())))
        });
        Ok(Inspection { candidates, note: if version.is_some() { "读取 EXE 文件版本，未执行工具。" } else { "快捷方式或 CLI 脚本未提供 EXE 版本；未执行安装或启动命令。" }.into(), version })
    }).await.map_err(|_| "安装检查未完成".to_string())?
}
/// 自动检测安装位置：优先保留仍然有效的现有入口，否则在 PATH、npm 包内、安装目录与
/// 开始菜单里选一个可直接执行的文件。只返回路径，不启动、不改配置，保存仍走原有补丁命令。
/// 唯一例外：现有入口只是 npm 的 `.cmd`/`.bat` 转发脚本而本机有原生 EXE 时改用 EXE。
#[tauri::command]
pub async fn tool_setup_autodetect(
    app: AppHandle,
    window: WebviewWindow,
    tool: String,
) -> Result<Autodetect, String> {
    allowed(&window)?;
    supported(&tool).map_err(|e| e.to_string())?;
    let config = app.state::<crate::config::ConfigMutex>().lock().clone();
    tokio::task::spawn_blocking(move || {
        let mut candidates = crate::ai::agent_tools::autodetect_candidates(&tool);
        // 用户已经选好的入口仍然有效时不覆盖：自动检测只修复空配置与失效路径。
        let current = config
            .tool_shortcut(&tool)
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .filter(|path| crate::ai::agent_tools::configured_install(&tool, Path::new(path)));
        if let Some(current) = current {
            // npm 全局安装只在顶层留转发脚本，原生 EXE 在包内：换成它才算选对入口。
            let native =
                preferred_candidate(&candidates).filter(|path| upgrades_to_native(current, path));
            if let Some(native) = native {
                candidates.truncate(12);
                return Autodetect {
                    path: Some(native),
                    candidates,
                    note: "当前入口是 npm 转发脚本，已改用包内的原生 EXE；如需保留脚本请手动选择。"
                        .into(),
                };
            }
            candidates.truncate(12);
            return Autodetect {
                path: Some(current.to_owned()),
                candidates,
                note: "当前启动入口仍然有效，已保留原有选择；如需更换请手动选择。".into(),
            };
        }
        // 挑选时使用全部候选（例如只在开始菜单里留下的 .lnk），展示仍限制为 12 条。
        let path = preferred_candidate(&candidates);
        candidates.truncate(12);
        Autodetect {
            note: if path.is_some() {
                "已按本机安装位置自动选中启动入口，保存后即可用于启动。".into()
            } else {
                "未在本机 PATH、npm 包、安装目录或开始菜单找到可执行入口，请手动选择快捷方式。"
                    .into()
            },
            path,
            candidates,
        }
    })
    .await
    .map_err(|_| "自动检测未完成".to_string())
}
fn probe_target(tool: &str) -> Result<(reqwest::Url, bool)> {
    if !["claude", "chatgpt"].contains(&tool) {
        bail!("此工具的服务配置由其自身管理，暂不推测连接地址，请在工具中测试");
    }
    let summary = connection(tool);
    if summary.source == "unknown" {
        bail!("无法确认工具连接配置，请先检查配置文件或在工具中确认服务商");
    }
    let (base, custom_endpoint) = if let Some(endpoint) = summary.endpoint {
        (endpoint, true)
    } else {
        (
            (if tool == "claude" {
                "https://claude.ai"
            } else {
                "https://chatgpt.com"
            })
            .to_string(),
            false,
        )
    };
    Ok((clean_endpoint(&base)?, custom_endpoint))
}
pub(crate) async fn test_connection(tool: &str, config: &AppConfig) -> Result<Connectivity> {
    supported(tool)?;
    let (url, custom) = probe_target(tool)?;
    let local = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    let builder = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none());
    let client = if local {
        builder
    } else {
        builder.proxy(reqwest::Proxy::all(format!(
            "http://127.0.0.1:{}",
            config.mixed_port
        ))?)
    }
    .build()?;
    Ok(probe_service(&client, url, custom).await)
}
async fn probe_service(client: &reqwest::Client, url: reqwest::Url, custom: bool) -> Connectivity {
    let endpoint = url.origin().ascii_serialization();
    let response = client.get(url).timeout(Duration::from_secs(8)).send().await;
    match response {
        Ok(response) => {
            let status = response.status();
            // API 根路径可返回 404/405；这只是地址探测，不是推理或权限验证。
            let reachable = status.is_success()
                || status.is_redirection()
                || (custom && matches!(status.as_u16(), 401 | 404 | 405));
            Connectivity {
                reachable,
                endpoint,
                status: Some(status.as_u16()),
                note: if reachable {
                    "地址可达；未携带密钥、未调用模型，不代表登录有效、额度充足或模型可用。".into()
                } else {
                    format!(
                        "服务返回 HTTP {}，请检查服务地址或代理线路。",
                        status.as_u16()
                    )
                },
            }
        }
        Err(_) => Connectivity {
            reachable: false,
            endpoint,
            status: None,
            note: "连接超时或失败，请检查代理线路及服务地址；未回退直连。".into(),
        },
    }
}
#[tauri::command]
pub async fn tool_setup_test(
    app: AppHandle,
    window: WebviewWindow,
    tool: String,
) -> Result<Connectivity, String> {
    allowed(&window)?;
    let config = app.state::<crate::config::ConfigMutex>().lock().clone();
    if crate::proxy::core::probe_controller(&config).await.is_err() {
        return Err("代理内核未运行，请先启动代理再测试连接".into());
    }
    test_connection(&tool, &config)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn summaries_distinguish_native_and_custom_endpoints() {
        let native = parse_connection(
            "claude",
            Some(br#"{"model":"native","env":{"ANTHROPIC_AUTH_TOKEN":"secret"}}"#),
        )
        .unwrap();
        assert_eq!(native.source, "native");
        assert!(native.endpoint.is_none());
        assert!(!serde_json::to_string(&native).unwrap().contains("secret"));
        let custom = parse_connection(
            "claude",
            Some(br#"{"model":"new","env":{"ANTHROPIC_BASE_URL":"https://api.example/v1","ANTHROPIC_AUTH_TOKEN":"secret"}}"#),
        )
        .unwrap();
        assert_eq!(custom.source, "external");
        assert_eq!(custom.endpoint.as_deref(), Some("https://api.example"));
        assert!(!serde_json::to_string(&custom).unwrap().contains("secret"));
        let codex = parse_connection(
            "chatgpt",
            Some(b"model='default'\nprofile='work'\n[profiles.work]\nmodel='project'\n"),
        )
        .unwrap();
        assert_eq!(codex.model.as_deref(), Some("project"));
        assert_eq!(parse_connection("chatgpt", None).unwrap().source, "native");
    }
    #[test]
    fn endpoint_validation_never_exposes_or_sends_embedded_credentials() {
        for url in [
            "file:///secret",
            "https://key:password@api.example",
            "https://api.example?key=secret",
        ] {
            assert!(clean_endpoint(url).is_err());
        }
        assert_eq!(
            clean_endpoint("https://api.example/v1#section")
                .unwrap()
                .origin()
                .ascii_serialization(),
            "https://api.example"
        );
        assert!(parse_connection("claude", Some(b"broken")).is_err());
        assert!(parse_connection("chatgpt", Some(b"model_provider='unregistered'")).is_err());
        assert!(
            parse_connection(
                "claude",
                Some(br#"{"env":{"CLAUDE_CODE_USE_BEDROCK":"1"}}"#)
            )
            .is_err()
        );
    }
    #[tokio::test]
    async fn service_probe_handles_api_roots_without_inference_or_credentials() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        for (status, custom, expected) in [
            (200, false, true),
            (401, true, true),
            (404, true, true),
            (405, true, true),
            (401, false, false),
            (403, true, false),
            (500, true, false),
            (302, true, true),
        ] {
            let server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = server.local_addr().unwrap();
            let task = tokio::spawn(async move {
                let (mut stream, _) = server.accept().await.unwrap();
                let mut buffer = [0; 8192];
                let count = stream.read(&mut buffer).await.unwrap();
                let request = String::from_utf8_lossy(&buffer[..count]).to_lowercase();
                assert!(request.starts_with("get / http/1.1"));
                assert!(!request.contains("authorization:") && !request.contains("cookie:"));
                stream.write_all(format!("HTTP/1.1 {status} Test\r\nLocation: http://127.0.0.1:1/must-not-follow\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
            });
            let result = probe_service(
                &client,
                format!("http://{address}").parse().unwrap(),
                custom,
            )
            .await;
            assert_eq!(result.reachable, expected, "HTTP {status}, custom={custom}");
            assert_eq!(result.status, Some(status));
            task.await.unwrap();
        }
    }

    #[test]
    fn autodetect_prefers_native_entries_and_ignores_missing_files() {
        let fixture = crate::test_support::FixtureDir::new();
        for name in [
            "autodetect.exe",
            "autodetect.cmd",
            "autodetect.bat",
            "autodetect.lnk",
        ] {
            std::fs::write(fixture.path().join(name), b"fixture").unwrap();
        }
        let path = |name: &str| fixture.path().join(name).to_string_lossy().into_owned();
        let candidates = vec![
            path("autodetect.bat"),
            path("missing.exe"),
            path("autodetect.cmd"),
            path("autodetect.exe"),
        ];
        assert_eq!(
            preferred_candidate(&candidates).as_deref(),
            Some(path("autodetect.exe").as_str())
        );
        assert_eq!(
            preferred_candidate(&candidates[0..2]).as_deref(),
            Some(path("autodetect.bat").as_str())
        );
        assert_eq!(preferred_candidate(&[path("missing.exe")]), None);
        assert_eq!(
            preferred_candidate(&[path("autodetect.cmd"), path("autodetect.lnk")]).as_deref(),
            Some(path("autodetect.cmd").as_str())
        );
    }

    #[test]
    fn npm_shim_yields_to_native_exe_but_manual_choices_stay() {
        let exe = r"C:\Users\t\AppData\Roaming\npm\node_modules\@openai\codex\vendor\x86_64-pc-windows-msvc\bin\codex.exe";
        let cmd = r"C:\Users\t\AppData\Roaming\npm\codex.cmd";
        let lnk = r"C:\Users\t\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Codex.lnk";
        // npm 转发脚本让位给包内的原生 EXE。
        assert!(upgrades_to_native(cmd, exe));
        assert!(upgrades_to_native(r"C:\npm\agent.BAT", exe));
        // 手动选的快捷方式、已经是 EXE、以及没有原生 EXE 时都不动。
        assert!(!upgrades_to_native(lnk, exe));
        assert!(!upgrades_to_native(exe, exe));
        assert!(!upgrades_to_native(cmd, r"C:\npm\agent.bat"));
        assert!(shim_script(cmd) && !shim_script(exe) && !shim_script(lnk));
    }

    #[test]
    fn autodetect_result_serializes_camel_case_for_the_client() {
        let value = serde_json::to_value(Autodetect {
            path: None,
            candidates: vec![r"C:\tools\agent.exe".into()],
            note: "fixture".into(),
        })
        .unwrap();
        assert!(value["path"].is_null());
        assert_eq!(value["candidates"][0], r"C:\tools\agent.exe");
        assert_eq!(value["note"], "fixture");
    }
}
