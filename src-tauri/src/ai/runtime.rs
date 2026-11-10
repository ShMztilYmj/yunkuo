//! Native executable discovery for the same tool catalog shown on the tool page.
//! Desktop launch shortcuts are never interpreted as a protocol-capable CLI.
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub fn supported(id: &str) -> bool {
    crate::ai::agent_roster::is_runtime(id)
}
pub fn external(id: &str) -> bool {
    crate::ai::agent_roster::is_external(id)
}
pub fn label(id: &str) -> &str {
    match crate::ai::agent_roster::label_for_runtime(id) {
        Some(name) => name,
        None => id,
    }
}
fn local(path: &Path) -> bool {
    path.is_absolute() && !path.to_string_lossy().starts_with(r"\\") && path.is_file()
}
fn directories() -> Vec<PathBuf> {
    let mut paths = std::env::var_os("PATH")
        .map(|v| {
            std::env::split_paths(&v)
                .filter(|p| p.is_absolute())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for (variable, relative) in [
        ("USERPROFILE", ".local/bin"),
        ("USERPROFILE", ".cursor/bin"),
        ("USERPROFILE", ".kimi-code/bin"),
        ("USERPROFILE", ".bun/bin"),
        ("APPDATA", "npm"),
        ("LOCALAPPDATA", "Programs/ZCode"),
        ("LOCALAPPDATA", "Programs/Antigravity"),
    ] {
        if let Some(base) = std::env::var_os(variable) {
            paths.push(PathBuf::from(base).join(relative));
        }
    }
    paths
}
fn find(names: &[&str]) -> Option<PathBuf> {
    directories()
        .iter()
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|p| local(p))
}
fn node() -> Result<PathBuf> {
    find(&["node.exe"]).context("请安装 Node.js 或选择原生可执行文件")
}
fn entry(path: &Path, id: &str) -> Result<Executable> {
    if !local(path) {
        bail!("请选择本机存在的运行时绝对路径");
    }
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let filename = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    // ZCode desktop ships its CLI bundle beside its Electron executable.
    if id == "glm" && extension == "exe" {
        let bundle = path.parent().unwrap().join("resources/glm/zcode.cjs");
        if bundle.is_file() {
            return Ok(Executable {
                program: path.to_owned(),
                prefix: vec![bundle.to_string_lossy().into_owned()],
            });
        }
    }
    if id == "cursor" && filename == "cursor.exe" {
        bail!("请选择 Cursor 的 agent.exe/CLI 入口，Cursor.exe 是编辑器入口");
    }
    if id == "antigravity" && filename != "agy_acp_server.exe" {
        bail!("请选择 agy_acp_server.exe；Antigravity 桌面入口不提供 ACP stdio");
    }
    match extension.as_str() {
        "exe" => Ok(Executable {
            program: path.to_owned(),
            prefix: vec![],
        }),
        "js" | "mjs" | "cjs" => Ok(Executable {
            program: node()?,
            prefix: vec![path.to_string_lossy().into_owned()],
        }),
        _ => bail!("运行时入口需为 .exe 或 Node .js/.mjs/.cjs；启动快捷方式不能作为协议入口"),
    }
}
pub fn discover(id: &str) -> Result<Executable> {
    if !external(id) {
        bail!("未知外部运行时");
    }
    let names: &[&str] = match id {
        "cursor" => &["agent.exe", "cursor-agent.exe"],
        "antigravity" => &["agy_acp_server.exe"],
        "kimi" => &["kimi.exe", "kimi-code.exe"],
        "grok" => &["grok.exe", "grok-build.exe"],
        "glm" => &["zcode.exe", "zcode-cli.exe", "ZCode.exe"],
        "deepseek" => &["dsh.exe"],
        _ => &[],
    };
    if let Some(path) = find(names) {
        return entry(&path, id);
    }
    if id == "deepseek" {
        let package = PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default())
            .join("npm/node_modules/@deepseek-ai/dsh");
        if let Ok(bytes) = std::fs::read(package.join("package.json")) {
            if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                if let Some(bin) = value["bin"]["dsh"].as_str().or(value["bin"].as_str()) {
                    let target = package.join(bin).canonicalize()?;
                    if target.starts_with(package.canonicalize()?) {
                        return entry(&target, id);
                    }
                }
            }
        }
    }
    bail!(
        "未找到 {} 原生协议入口，请先安装、登录并在协作运行时中选择入口",
        label(id)
    )
}
/// 按给定的协作入口表解析；入口表来自应用配置，测试里来自 teams/runtime-paths.json。
pub fn resolve_with(paths: &BTreeMap<String, String>, id: &str) -> Result<Executable> {
    if !supported(id) {
        bail!("未知协作运行时");
    }
    if let Some(path) = paths.get(id) {
        return entry(Path::new(path), id);
    }
    resolve(id)
}
/// 校验并写入一条协作入口；空路径表示恢复自动发现。
pub fn apply(paths: &mut BTreeMap<String, String>, id: &str, path: &str) -> Result<()> {
    if !supported(id) {
        bail!("未知协作运行时");
    }
    if path.is_empty() {
        paths.remove(id);
    } else {
        entry(Path::new(path), id)?;
        paths.insert(id.into(), path.into());
    }
    Ok(())
}
/// 旧版存档位置：仅在测试或没有 AppHandle 时使用。
pub(crate) fn read_paths(root: &Path) -> Result<BTreeMap<String, String>> {
    let file = root.join("runtime-paths.json");
    if !file.exists() {
        return Ok(BTreeMap::new());
    }
    crate::platform::secure_storage::safe(&file)?;
    if std::fs::metadata(&file)?.len() > 32 * 1024 {
        bail!("运行时路径配置过大");
    }
    Ok(serde_json::from_slice(&std::fs::read(file)?)?)
}
pub(crate) fn write_paths(root: &Path, paths: &BTreeMap<String, String>) -> Result<()> {
    crate::platform::secure_storage::write_auth(
        &root.join("runtime-paths.json"),
        &serde_json::to_vec(paths)?,
    )
}
/// 旧版存档路径的便捷入口，生产环境改用 `Engine::resolve_runtime`；测试沿用它。
#[cfg_attr(not(test), allow(dead_code))]
pub fn resolve_legacy(root: &Path, id: &str) -> Result<Executable> {
    resolve_with(&read_paths(root).unwrap_or_default(), id)
}
/// 旧版存档路径的写入入口，生产环境改用 `Engine::save_tool_paths`；测试沿用它。
#[cfg_attr(not(test), allow(dead_code))]
pub fn configure(root: &Path, id: &str, path: &str) -> Result<()> {
    let mut paths = read_paths(root)?;
    apply(&mut paths, id, path)?;
    write_paths(root, &paths)
}

#[derive(Clone)]
pub struct Executable {
    pub program: PathBuf,
    pub prefix: Vec<String>,
}
fn path_exe(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|p| p.join(format!("{name}.exe")))
        .find(|p| p.is_file())
}
pub fn resolve(name: &str) -> Result<Executable> {
    if external(name) {
        return discover(name);
    }
    if let Some(p) = path_exe(name) {
        return Ok(Executable {
            program: p,
            prefix: vec![],
        });
    }
    let npm =
        PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("npm/node_modules");
    let native = match name {
        "claude" => Some(npm.join("@anthropic-ai/claude-code/bin/claude.exe")),
        _ => None,
    };
    if let Some(p) = native.filter(|p| p.is_file()) {
        return Ok(Executable {
            program: p,
            prefix: vec![],
        });
    }
    let script = match name {
        "codex" => npm.join("@openai/codex/bin/codex.js"),
        "claude" => npm.join("@anthropic-ai/claude-code/cli.js"),
        "opencode" => npm.join("opencode-ai/bin/opencode"),
        _ => bail!("未知运行时"),
    };
    if script.is_file() {
        return Ok(Executable {
            program: path_exe("node").context("未找到 Node.js")?,
            prefix: vec![script.to_string_lossy().into_owned()],
        });
    }
    bail!("未找到 {name} CLI，请先安装并在终端完成登录")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_and_protocol_paths() {
        assert_eq!(crate::ai::agent_roster::runtime_ids().len(), 9);
        assert!(external("glm"));
        assert!(!external("codex"));
        assert!(!supported("model-provider"));
        let root = crate::test_support::FixtureDir::new();
        let cursor = root.path().join("Cursor.exe");
        std::fs::write(&cursor, []).unwrap();
        assert!(configure(root.path(), "cursor", cursor.to_str().unwrap()).is_err());
        let agent = root.path().join("agent.exe");
        std::fs::write(&agent, []).unwrap();
        configure(root.path(), "cursor", agent.to_str().unwrap()).unwrap();
        assert_eq!(
            resolve_legacy(root.path(), "cursor").unwrap().program,
            agent
        );
        assert!(configure(root.path(), "antigravity", cursor.to_str().unwrap()).is_err());
    }
}
