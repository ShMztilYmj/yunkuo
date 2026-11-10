//! HUD 本机工具识别：只返回安装/进程状态，不发送进程路径、参数或会话正文。
use crate::config::{AI_TOOL_KEYS as KEYS, AppConfig};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    pub key: String,
    pub installed: bool,
    pub online: bool,
}

struct Inventory {
    system: System,
    at: Option<Instant>,
    shortcuts: Vec<String>,
    tools: Vec<ToolStatus>,
}
static INVENTORY: Lazy<Mutex<Inventory>> = Lazy::new(|| {
    Mutex::new(Inventory {
        system: System::new(),
        at: None,
        shortcuts: Vec::new(),
        tools: Vec::new(),
    })
});

fn normalized(value: &str) -> String {
    value.replace('\\', "/").to_ascii_lowercase()
}
fn executable_key(name: &str) -> Option<&'static str> {
    let normalized = name.to_ascii_lowercase();
    crate::ai::agent_roster::key_for_alias(normalized.trim_end_matches(".exe"))
}
fn interpreter(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().trim_end_matches(".exe"),
        "node" | "nodejs" | "bun" | "python" | "python3" | "pythonw" | "uv" | "uvx"
    )
}

fn process_key(name: &str, args: &[String]) -> Option<&'static str> {
    if let Some(key) = executable_key(name) {
        return Some(key);
    }
    if !interpreter(name) {
        return None;
    }
    // 只检查解释器入口，不能因提示词/脚本参数提到某个工具而误报在线。
    if args
        .iter()
        .skip(1)
        .take(2)
        .any(|arg| matches!(arg.as_str(), "-e" | "--eval" | "-p" | "--print" | "-c"))
    {
        return None;
    }
    let entry = normalized(args.iter().skip(1).find(|arg| !arg.starts_with('-'))?);
    if entry == "kimi_cli" || entry.starts_with("kimi_cli.") {
        return Some("kimi");
    }
    // npm 包路径特征同样来自名单：带尾斜杠的要求脚本扩展名，不带的要求固定入口。
    for (marker, key) in crate::ai::agent_roster::npm_markers() {
        let matched = if marker.ends_with('/') {
            entry.contains(marker)
                && [".js", ".mjs", ".cjs", ".py"]
                    .iter()
                    .any(|suffix| entry.ends_with(suffix))
        } else {
            entry.ends_with(marker) || entry.ends_with(&format!("{marker}.js"))
        };
        if matched {
            return Some(key);
        }
    }
    None
}

fn identity(name: &str, executable: Option<&Path>, args: &[String]) -> Option<&'static str> {
    let key = process_key(name, args)?;
    if key == "claude" {
        let path = executable
            .map(|path| normalized(&path.to_string_lossy()))
            .unwrap_or_default();
        // Claude 桌面端的 Electron 主进程与 CLI 同名，桌面目录不能作为 Code 的证据。
        if [
            "/programs/claude/",
            "/program files/claude/",
            "/program files (x86)/claude/",
            "/anthropicclaude/",
            "/packages/claude_",
            "/windowsapps/anthropicpbc.claude",
            "/claude/app-",
        ]
        .iter()
        .any(|marker| path.contains(marker))
        {
            return None;
        }
    }
    Some(key)
}

fn shortcut_target(path: &Path) -> Option<PathBuf> {
    if !local_file(path) {
        return None;
    }
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"))
    {
        return Some(path.to_owned());
    }
    use windows::{
        Win32::{
            System::Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize, IPersistFile, STGM_READ,
            },
            UI::Shell::{IShellLinkW, ShellLink},
        },
        core::{Interface, PCWSTR},
    };
    unsafe {
        // 不调用 Resolve，避免网络探测、交互对话框和执行快捷方式。
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let result = (|| {
            let link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
            let persist: IPersistFile = link.cast().ok()?;
            let filename: Vec<u16> = path
                .as_os_str()
                .to_string_lossy()
                .encode_utf16()
                .chain(Some(0))
                .collect();
            persist.Load(PCWSTR(filename.as_ptr()), STGM_READ).ok()?;
            let mut target = [0u16; 32768];
            link.GetPath(&mut target, std::ptr::null_mut(), 0).ok()?;
            let length = target
                .iter()
                .position(|ch| *ch == 0)
                .unwrap_or(target.len());
            if length == 0 {
                return None;
            }
            Some(PathBuf::from(String::from_utf16_lossy(&target[..length])))
        })();
        if initialized.is_ok() {
            CoUninitialize();
        }
        result
    }
}
pub(crate) fn configured_install(key: &str, path: &Path) -> bool {
    let Some(target) = shortcut_target(path) else {
        return false;
    };
    if !local_file(&target) {
        return false;
    }
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    if key == "claude"
        && executable_key(&name) == Some("claude")
        && identity(&name, Some(&target), &[]).is_none()
    {
        return false;
    }
    true
}

fn command_names(key: &str) -> &'static [String] {
    crate::ai::agent_roster::install_names(key)
}
fn local_file(path: &Path) -> bool {
    !path.as_os_str().to_string_lossy().starts_with(r"\\") && path.is_file()
}
fn search_directories() -> Vec<PathBuf> {
    let mut directories: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|value| {
            std::env::split_paths(&value)
                .filter(|path| path.is_absolute())
                .collect()
        })
        .unwrap_or_default();
    // 通用包管理器目录与逐工具的安装目录分开：前者不属于任何一款 Agent。
    for (variable, relative) in [
        ("APPDATA", "npm"),
        ("USERPROFILE", ".local/bin"),
        ("USERPROFILE", ".bun/bin"),
    ] {
        if let Some(base) = std::env::var_os(variable) {
            directories.push(PathBuf::from(base).join(relative));
        }
    }
    for (variable, relative) in crate::ai::agent_roster::install_dirs() {
        if let Some(base) = std::env::var_os(variable) {
            directories.push(PathBuf::from(base).join(relative));
        }
    }
    directories.sort();
    directories.dedup();
    directories
}
/// 开始菜单与桌面：安装器常常只留下 .lnk，而不会把目录写进 PATH。
fn shortcut_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for (variable, relative) in [
        ("APPDATA", "Microsoft/Windows/Start Menu/Programs"),
        ("ProgramData", "Microsoft/Windows/Start Menu/Programs"),
        ("USERPROFILE", "Desktop"),
        ("PUBLIC", "Desktop"),
    ] {
        if let Some(base) = std::env::var_os(variable) {
            let path = PathBuf::from(base).join(relative);
            if path.is_dir() {
                roots.push(path);
            }
        }
    }
    roots.sort();
    roots.dedup();
    roots
}
fn shortcut_matches(key: &str, path: &Path) -> bool {
    let stem = path
        .file_stem()
        .map(|value| value.to_string_lossy().trim().to_ascii_lowercase())
        .unwrap_or_default();
    if stem.is_empty()
        || !command_names(key)
            .iter()
            .any(|name| name.to_ascii_lowercase() == stem)
    {
        return false;
    }
    let Some(target) = shortcut_target(path) else {
        return false;
    };
    if !local_file(&target) {
        return false;
    }
    // Claude 桌面端与 CLI 同名：指向桌面应用的快捷方式不能当作 Claude Code。
    !(key == "claude"
        && target
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
        && identity("claude.exe", Some(&target), &[]).is_none())
}
/// 限制深度与计数：开始菜单里无关的快捷方式很多，检测不应全量解析。
fn collect_shortcuts(
    directory: &Path,
    key: &str,
    depth: u8,
    budget: &mut usize,
    paths: &mut Vec<String>,
) {
    if depth == 0 || *budget == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        if *budget == 0 {
            return;
        }
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            collect_shortcuts(&path, key, depth - 1, budget, paths);
        } else if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
        {
            *budget = (*budget).saturating_sub(1);
            if shortcut_matches(key, &path) {
                paths.push(path.to_string_lossy().into_owned());
            }
        }
    }
}
/// PATH、npm 与安装目录里能直接执行的入口；用于状态轮询与手动选择，不扫开始菜单。
fn executable_candidates(key: &str) -> Vec<String> {
    let names = command_names(key);
    let mut paths = Vec::new();
    for directory in search_directories() {
        for name in names {
            for extension in ["exe", "cmd", "bat"] {
                let path = directory.join(format!("{name}.{extension}"));
                if !local_file(&path) {
                    continue;
                }
                // Claude 桌面端与 CLI 同名：Programs/Claude 下的 EXE 不能作为启动入口。
                if key == "claude"
                    && extension == "exe"
                    && identity("claude.exe", Some(&path), &[]).is_none()
                {
                    continue;
                }
                paths.push(path.to_string_lossy().into_owned());
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}
pub(crate) fn launch_candidates(key: &str) -> Vec<String> {
    let mut paths = executable_candidates(key);
    paths.truncate(12);
    paths
}
/// 在包目录里找与命令同名的 EXE（`codex.exe`），`rg.exe` 一类随包工具不算入口。
fn collect_named_binaries(
    directory: &Path,
    names: &[String],
    depth: u8,
    budget: &mut usize,
    paths: &mut Vec<String>,
) {
    if depth == 0 || *budget == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        if *budget == 0 {
            return;
        }
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            *budget = (*budget).saturating_sub(1);
            collect_named_binaries(&path, names, depth - 1, budget, paths);
        } else if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
            && path.file_stem().is_some_and(|stem| {
                names
                    .iter()
                    .any(|name| stem.eq_ignore_ascii_case(std::ffi::OsStr::new(name)))
            })
            && local_file(&path)
        {
            paths.push(path.to_string_lossy().into_owned());
        }
    }
}
/// npm 全局安装的 CLI 只在顶层留 `.cmd`/`.ps1` 转发脚本，真正的原生 EXE 藏在包内
/// `vendor/<target>/bin` 里；只沿名单里的包特征做有界深度扫描，不遍历整个 node_modules。
fn package_binaries(key: &str, directories: &[PathBuf]) -> Vec<String> {
    let names = command_names(key);
    let markers: Vec<&str> = crate::ai::agent_roster::npm_markers()
        .into_iter()
        .filter(|(_, owner)| *owner == key)
        .map(|(marker, _)| marker.trim_matches('/'))
        .filter(|marker| !marker.is_empty())
        .collect();
    if markers.is_empty() {
        return Vec::new();
    }
    let mut paths = Vec::new();
    let mut budget = 400usize;
    for directory in directories {
        for marker in &markers {
            let root = marker
                .split('/')
                .fold(directory.join("node_modules"), |root, part| root.join(part));
            if root.is_dir() {
                // 包根 → node_modules → @scope → 平台包 → vendor → target → bin，共 7 层。
                collect_named_binaries(&root, names, 8, &mut budget, &mut paths);
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}
/// 手动选择列表：在可执行入口之外补上 npm 包内的原生 EXE；只在用户点击检测时调用。
pub(crate) fn selectable_candidates(key: &str) -> Vec<String> {
    let mut paths = executable_candidates(key);
    paths.extend(package_binaries(key, &search_directories()));
    paths.sort();
    paths.dedup();
    paths.truncate(16);
    paths
}
/// 自动定位在可执行入口之外再扫 npm 包内 EXE、开始菜单与桌面；只在用户点击时调用，避免轮询付代价。
pub(crate) fn autodetect_candidates(key: &str) -> Vec<String> {
    let mut paths = executable_candidates(key);
    paths.extend(package_binaries(key, &search_directories()));
    let mut budget = 400usize;
    for root in shortcut_roots() {
        collect_shortcuts(&root, key, 4, &mut budget, &mut paths);
    }
    paths.sort();
    paths.dedup();
    paths
}
fn installed_on_path(key: &str, directories: &[PathBuf]) -> bool {
    let names = command_names(key);
    directories.iter().any(|directory| {
        names.iter().any(|name| {
            ["exe", "cmd", "bat", "ps1"].iter().any(|extension| {
                let path = directory.join(format!("{name}.{extension}"));
                local_file(&path)
                    && !(key == "claude"
                        && extension == &"exe"
                        && identity("claude.exe", Some(&path), &[]).is_none())
            })
        })
    })
}
fn configured_shortcut<'a>(key: &str, config: &'a AppConfig) -> Option<&'a str> {
    // ChatGPT 的启动入口不能当作 Codex 的安装证据。
    if key == "chatgpt" {
        None
    } else {
        config.tool_shortcut(key)
    }
}

/// 卡片是 HUD 名单来源；安装证据和在线状态仍保留各自真实含义。
pub fn for_cards(tools: Vec<ToolStatus>, config: &AppConfig) -> Vec<ToolStatus> {
    config
        .tool_cards()
        .into_iter()
        .map(|key| {
            tools
                .iter()
                .find(|tool| tool.key == key)
                .cloned()
                .unwrap_or(ToolStatus {
                    key,
                    installed: false,
                    online: false,
                })
        })
        .collect()
}

pub fn read(config: &AppConfig) -> Result<Vec<ToolStatus>, String> {
    let mut inventory = INVENTORY.lock();
    let keys = config.tool_cards();
    let shortcuts: Vec<_> = keys
        .iter()
        .map(|key| {
            format!(
                "{key}:{}",
                configured_shortcut(key, config).unwrap_or_default()
            )
        })
        .collect();
    if inventory.shortcuts == shortcuts
        && inventory
            .at
            .is_some_and(|at| at.elapsed() < Duration::from_secs(5))
    {
        return Ok(inventory.tools.clone());
    }
    let system = &mut inventory.system;
    let count = system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
    );
    if count == 0 {
        return Err("本机进程列表暂时不可用".into());
    }
    // 大部分进程只读名称；只对 Node/Python 等解释器读取命令入口，不采集环境变量。
    let interpreters: Vec<_> = system
        .processes()
        .iter()
        .filter(|(_, process)| interpreter(&process.name().to_string_lossy()))
        .map(|(pid, _)| *pid)
        .collect();
    if !interpreters.is_empty() {
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&interpreters),
            false,
            ProcessRefreshKind::nothing().with_cmd(UpdateKind::OnlyIfNotSet),
        );
    }
    let mut online = std::collections::HashSet::new();
    for process in system.processes().values() {
        let args: Vec<String> = if interpreter(&process.name().to_string_lossy()) {
            process
                .cmd()
                .iter()
                .take(8)
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect()
        } else {
            Vec::new()
        };
        if let Some(key) = identity(&process.name().to_string_lossy(), process.exe(), &args) {
            online.insert(key);
        }
    }
    let directories = search_directories();
    let tools: Vec<_> = keys
        .iter()
        .map(|key| {
            let running = online.contains(key.as_str());
            let configured = configured_shortcut(key, config).is_some_and(|path| {
                !path.trim().is_empty() && configured_install(key, Path::new(path.trim()))
            });
            ToolStatus {
                key: key.clone(),
                installed: running || configured || installed_on_path(key, &directories),
                online: running,
            }
        })
        .collect();
    inventory.tools = tools.clone();
    inventory.shortcuts = shortcuts;
    inventory.at = Some(Instant::now());
    Ok(tools)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }
    #[test]
    fn hud_uses_card_order_and_preserves_real_process_evidence() {
        let mut config = AppConfig::default();
        config.ai_tool_cards = args(&["kimi", "claude", "kimi"]);
        let tools = vec![
            ToolStatus {
                key: "grok".into(),
                installed: true,
                online: true,
            },
            ToolStatus {
                key: "kimi".into(),
                installed: true,
                online: true,
            },
            ToolStatus {
                key: "claude".into(),
                installed: false,
                online: false,
            },
        ];
        let selected = for_cards(tools, &config);
        assert_eq!(
            selected
                .iter()
                .map(|tool| tool.key.as_str())
                .collect::<Vec<_>>(),
            ["claude", "chatgpt", "kimi"]
        );
        assert!(!selected[0].installed && !selected[0].online);
        assert!(!selected[1].installed && !selected[1].online);
        assert!(selected[2].online);
        config.ai_tool_cards.clear();
        assert_eq!(for_cards(selected, &config).len(), 2);
    }
    #[test]
    fn online_requires_exact_executable_or_interpreter_entry() {
        assert_eq!(process_key("Claude.exe", &[]), Some("claude"));
        assert_eq!(process_key("third-party-notetool.exe", &[]), None);
        assert_eq!(
            process_key(
                "node.exe",
                &args(&["node", "C:/npm/node_modules/@openai/codex/bin/codex.js"])
            ),
            Some("chatgpt")
        );
        assert_eq!(
            process_key("python.exe", &args(&["python", "-m", "kimi_cli"])),
            Some("kimi")
        );
        assert_eq!(
            process_key(
                "node.exe",
                &args(&[
                    "node",
                    "unrelated.js",
                    "C:/npm/node_modules/@openai/codex/bin/codex.js"
                ])
            ),
            None
        );
        assert_eq!(
            process_key(
                "node.exe",
                &args(&[
                    "node",
                    "-e",
                    "C:/npm/node_modules/@openai/codex/bin/codex.js"
                ])
            ),
            None
        );
    }
    #[test]
    fn installation_needs_existing_executable_not_old_usage_directory() {
        let directory =
            crate::test_support::scratch_root().join(format!("hud-tools-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(directory.join(".claude")).unwrap();
        assert!(!installed_on_path("claude", &[directory.clone()]));
        std::fs::write(directory.join("claude.cmd"), "fixture").unwrap();
        assert!(installed_on_path("claude", &[directory.clone()]));
        assert!(!installed_on_path("chatgpt", &[directory.clone()]));
        std::fs::write(directory.join("dsh.cmd"), "fixture").unwrap();
        std::fs::write(directory.join("zcode.exe"), "fixture").unwrap();
        std::fs::write(directory.join("cursor.exe"), "fixture").unwrap();
        std::fs::write(directory.join("opencode.exe"), "fixture").unwrap();
        for key in ["deepseek", "glm", "cursor", "opencode"] {
            assert!(installed_on_path(key, &[directory.clone()]));
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn named_agents_require_their_own_entry_not_model_names_or_arguments() {
        for (name, key) in [
            ("Cursor.exe", "cursor"),
            ("opencode.exe", "opencode"),
            ("ZCode.exe", "glm"),
            ("dsh.exe", "deepseek"),
            ("DeepSeek Harness.exe", "deepseek"),
            ("grok-build.exe", "grok"),
        ] {
            assert_eq!(process_key(name, &[]), Some(key));
        }
        assert_eq!(process_key("glm.exe", &[]), None);
        assert_eq!(process_key("deepseek.exe", &[]), None);
        let entry = "C:/npm/node_modules/@deepseek-ai/dsh/lib/bin.js";
        assert_eq!(
            process_key("node.exe", &args(&["node", entry, "web"])),
            Some("deepseek")
        );
        assert_eq!(
            process_key("node.exe", &args(&["node", "unrelated.js", entry])),
            None
        );
        assert_eq!(
            process_key("node.exe", &args(&["node", "--eval", entry])),
            None
        );
        assert_eq!(
            process_key(
                "node.exe",
                &args(&["node", "C:/npm/node_modules/opencode-ai/bin/opencode"])
            ),
            Some("opencode")
        );
        assert_eq!(
            process_key("node.exe", &args(&["node", "notes/deepseek-harness.js"])),
            None
        );
    }
    #[test]
    fn desktop_claude_and_stale_shortcut_are_not_code_installation_evidence() {
        assert_eq!(
            identity(
                "Claude.exe",
                Some(Path::new(
                    "C:/Users/test/AppData/Local/Programs/Claude/Claude.exe"
                )),
                &[]
            ),
            None
        );
        assert_eq!(
            identity(
                "claude.exe",
                Some(Path::new("C:/Users/test/.local/bin/claude.exe")),
                &[]
            ),
            Some("claude")
        );
        assert_eq!(
            identity(
                "third-party-notetool.exe",
                Some(Path::new("C:/apps/third-party-notetool.exe")),
                &[]
            ),
            None
        );
        let fixture = crate::test_support::FixtureDir::new();
        assert!(!configured_install(
            "claude",
            &fixture.path().join("missing.lnk")
        ));
        std::fs::write(fixture.path().join("invalid.lnk"), b"invalid").unwrap();
        assert!(!configured_install(
            "claude",
            &fixture.path().join("invalid.lnk")
        ));
    }
    #[test]
    fn start_menu_entry_must_match_the_agent_and_point_to_a_local_file() {
        let fixture = crate::test_support::FixtureDir::new();
        let claude = fixture.path().join("Claude.lnk");
        let unrelated = fixture.path().join("Notes.lnk");
        std::fs::write(&claude, b"invalid shortcut").unwrap();
        std::fs::write(&unrelated, b"invalid shortcut").unwrap();
        // 名称不匹配的快捷方式在解析目标前就被排除。
        assert!(!shortcut_matches("claude", &unrelated));
        // 名称匹配但目标无法解析的快捷方式同样不可用。
        assert!(!shortcut_matches("claude", &claude));
        assert!(!shortcut_matches("chatgpt", &claude));
    }
    #[test]
    fn npm_wrapped_cli_resolves_the_native_exe_inside_the_package() {
        let fixture = crate::test_support::FixtureDir::new();
        let npm = fixture.path().join("npm");
        let bin = npm
            .join("node_modules")
            .join("@openai")
            .join("codex")
            .join("node_modules")
            .join("@openai")
            .join("codex-win32-x64")
            .join("vendor")
            .join("x86_64-pc-windows-msvc")
            .join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(npm.join("codex.cmd"), b"fixture").unwrap();
        std::fs::write(bin.join("codex.exe"), b"fixture").unwrap();
        // 随包工具与同名非 EXE 文件都不算启动入口。
        std::fs::write(bin.join("rg.exe"), b"fixture").unwrap();
        std::fs::write(bin.join("codex"), b"fixture").unwrap();
        assert_eq!(
            package_binaries("chatgpt", std::slice::from_ref(&npm)),
            vec![bin.join("codex.exe").to_string_lossy().into_owned()]
        );
        // 包没装时不会凭空造出候选。
        assert!(package_binaries("claude", std::slice::from_ref(&npm)).is_empty());
    }
    #[test]
    #[ignore = "读取本机安装位置；用于确认自动定位能发现已安装的 CLI"]
    fn local_autodetect_candidates_are_absolute_files() {
        for key in ["claude", "chatgpt"] {
            for path in autodetect_candidates(key) {
                println!("{key}: {path}");
                assert!(Path::new(&path).is_absolute());
                assert!(Path::new(&path).is_file());
            }
        }
    }
    #[test]
    fn native_inventory_follows_added_cards_and_invalidates_the_warm_cache() {
        let tools = read(&AppConfig::default()).unwrap();
        assert_eq!(
            tools
                .iter()
                .map(|tool| tool.key.as_str())
                .collect::<Vec<_>>(),
            ["claude", "chatgpt"]
        );
        for tool in &tools {
            assert!(!tool.online || tool.installed);
            println!(
                "{}: installed={}, online={}",
                tool.key, tool.installed, tool.online
            );
        }
        let json = serde_json::to_value(tools).unwrap();
        assert!(
            json.as_array()
                .unwrap()
                .iter()
                .all(|tool| tool.as_object().unwrap().len() == 3)
        );
        let mut config = AppConfig::default();
        config.ai_tool_cards.push("kimi".into());
        assert_eq!(
            read(&config)
                .unwrap()
                .iter()
                .map(|tool| tool.key.as_str())
                .collect::<Vec<_>>(),
            ["claude", "chatgpt", "kimi"]
        );
        assert_eq!(read(&AppConfig::default()).unwrap().len(), 2);
    }
}
/// 已启用的九款工具，按名单顺序；退役卡片不在其中。
pub fn order() -> Vec<&'static str> {
    crate::ai::agent_roster::all()
        .iter()
        .filter(|agent| !agent.retired)
        .map(|agent| agent.key.as_str())
        .collect()
}
/// The supported catalog is broader than the set the user has added.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MonitoringScope(Vec<String>);
impl MonitoringScope {
    pub(crate) fn from_cards(cards: &[String]) -> Self {
        Self(
            ["claude", "chatgpt"]
                .into_iter()
                .chain(cards.iter().map(String::as_str))
                .filter(|key| KEYS.contains(key))
                .fold(Vec::new(), |mut keys, key| {
                    if !keys.iter().any(|existing| existing == key) {
                        keys.push(key.to_string());
                    }
                    keys
                }),
        )
    }
    pub(crate) fn current(app: &tauri::AppHandle) -> Self {
        use tauri::Manager;
        Self::from_cards(
            &app.state::<crate::config::ConfigMutex>()
                .lock()
                .ai_tool_cards,
        )
    }
    pub(crate) fn contains(&self, key: &str) -> bool {
        self.0.iter().any(|value| value == key)
    }
    pub(crate) fn keys(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }
}
pub fn label(key: &str) -> &'static str {
    crate::ai::agent_roster::get(key).map_or("Agent", |agent| agent.name.as_str())
}
