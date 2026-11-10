use super::source;
use crate::{
    ai::runtime::Executable,
    workspace::resume::model::{Compatibility, Quota, digest},
};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::Command};

pub fn command(exe: &Executable, cwd: &Path) -> Command {
    let mut c = Command::new(&exe.program);
    c.args(&exe.prefix)
        .current_dir(cwd)
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    c
}
pub fn resume_command(exe: &Executable, cwd: &Path, session: &str, model: &str) -> Command {
    let mut c = command(exe, cwd);
    c.args([
        "--resume",
        session,
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--replay-user-messages",
        "--permission-mode",
        "manual",
        "--model",
        model,
    ])
    .stdin(Stdio::piped());
    c
}
pub async fn stop_process(
    child: &mut tokio::process::Child,
    job: crate::platform::process_job::ProcessJob,
) -> Result<()> {
    let _ = child.start_kill();
    // Closing our Job Object terminates only this recovery's descendants.
    drop(job);
    tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .context("恢复进程停止超时")??;
    Ok(())
}
pub fn fingerprint(exe: &Executable) -> Result<String> {
    let mut parts = vec![exe.program.clone()];
    parts.extend(exe.prefix.iter().map(std::path::PathBuf::from));
    let mut value = String::new();
    for p in parts {
        let m = std::fs::metadata(&p)?;
        value.push_str(&format!(
            "{}:{}:{:?};",
            p.to_string_lossy(),
            m.len(),
            m.modified()?
        ));
    }
    Ok(digest(value))
}
async fn output(exe: &Executable, cwd: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let mut c = command(exe, cwd);
    c.args(args);
    let mut child = c.spawn().context("无法启动 Claude Code 检查")?;
    let result = tokio::time::timeout(Duration::from_secs(12), async {
        let mut out = Vec::new();
        child
            .stdout
            .take()
            .context("检查输出不可用")?
            .take(512 * 1024 + 1)
            .read_to_end(&mut out)
            .await?;
        if out.len() > 512 * 1024 || !child.wait().await?.success() {
            bail!("Claude Code 检查未通过");
        }
        Ok(out)
    })
    .await;
    if result.as_ref().is_err() || result.as_ref().is_ok_and(|r| r.is_err()) {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    result.context("Claude Code 检查超时")?
}
pub async fn check(home: &Path) -> Result<(Executable, String)> {
    let exe = crate::ai::runtime::resolve("claude")?;
    let version = String::from_utf8(output(&exe, home, &["--version"]).await?)?;
    let help = String::from_utf8(output(&exe, home, &["--help"]).await?)?;
    for flag in [
        "--resume",
        "--print",
        "--output-format",
        "--input-format",
        "--replay-user-messages",
        "--permission-mode",
    ] {
        if !help.contains(flag) {
            bail!("当前 Claude Code 缺少所需恢复接口，请更新后重新检查");
        }
    }
    if !help.contains("manual") {
        bail!("当前版本尚未验证受控权限模式，请更新 Claude Code");
    }
    let auth: Value =
        serde_json::from_slice(&output(&exe, home, &["auth", "status", "--json"]).await?)?;
    if auth["loggedIn"] != true
        || !matches!(
            auth["authMethod"].as_str(),
            Some("claude.ai" | "oauth_token")
        )
    {
        bail!("当前 CLI 没有使用 Claude 订阅登录");
    }
    Ok((
        exe,
        crate::workspace::resume::model::label(version.trim(), 80),
    ))
}
pub fn public_error(e: &anyhow::Error) -> String {
    super::super::public_error(e, "本地状态暂不可核验，请重新检查；未发送继续消息")
}
pub async fn usage(home: &Path) -> Result<(String, Quota)> {
    let (key, v) = usage_value(home).await?;
    Ok((key, source::quota(&v, chrono::Utc::now().timestamp())))
}
/// 原始额度响应：续跑只关心是否耗尽，AI 概览需要完整的 5 小时 / 每周窗口。
///
/// 只读 OAuth 用量接口，不发送模型请求。与 AI 监控共用本机代理，避免真实出口
/// 与账号地区不一致；代理未就绪时沿用 reqwest 的默认网络行为。
pub(crate) async fn usage_value(home: &Path) -> Result<(String, Value)> {
    let login = source::login(home)?;
    let mut builder = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(8));
    if let Some(port) = crate::ai::aimon::proxy_port() {
        builder = builder.proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))?);
    }
    let client = builder.build()?;
    let mut response = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .bearer_auth(login.token.as_str())
        .header("anthropic-beta", "oauth-2025-04-20")
        .header("Accept", "application/json")
        .send()
        .await
        .context("实时额度查询失败，请检查网络")?;
    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let seconds = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(60)
            .clamp(30, 3600);
        return Err(anyhow::Error::new(Cooldown(seconds)));
    }
    if matches!(response.status().as_u16(), 401 | 403) {
        bail!("订阅登录已失效或无额度查询权限，请在 Claude Code 中重新登录");
    }
    if !response.status().is_success() {
        bail!("Claude 实时额度服务暂不可用");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > 512 * 1024 {
            bail!("额度响应结构超出预期");
        }
        bytes.extend_from_slice(&chunk);
    }
    let v: Value = serde_json::from_slice(&bytes)?;
    if source::login(home)?.key != login.key {
        bail!("额度查询期间登录已变化");
    }
    Ok((login.key, v))
}
#[derive(Debug)]
pub struct Cooldown(pub u64);
impl std::fmt::Display for Cooldown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "额度查询受到限流，等待 {} 秒后核验", self.0)
    }
}
impl std::error::Error for Cooldown {}
pub async fn report(home: &Path) -> Compatibility {
    let result = probe(home).await;
    match result {
        Ok(v) => Compatibility {
            supported: true,
            engine_version: Some(v),
            message: "CLI 恢复接口与订阅额度可用；发送前仍会核验会话占用".into(),
            checked_at: Some(chrono::Utc::now().timestamp()),
        },
        Err(e) => Compatibility {
            message: public_error(&e),
            checked_at: Some(chrono::Utc::now().timestamp()),
            ..Default::default()
        },
    }
}
pub async fn probe(home: &Path) -> Result<String> {
    let (_, version) = check(home).await?;
    let (_, quota) = usage(home).await?;
    if quota.available.is_none() {
        bail!("实时额度数据暂不可识别");
    }
    Ok(version)
}
pub fn other_claude_running() -> bool {
    let system = sysinfo::System::new_with_specifics(
        sysinfo::RefreshKind::nothing().with_processes(
            sysinfo::ProcessRefreshKind::nothing()
                .with_exe(sysinfo::UpdateKind::Always)
                .with_cmd(sysinfo::UpdateKind::Always),
        ),
    );
    system.processes().values().any(|p| {
        let name = p.name().to_string_lossy().to_lowercase();
        name == "claude.exe"
            || name == "claude"
            || name == "node.exe"
                && p.cmd().iter().any(|s| {
                    let s = s.to_string_lossy().replace('\\', "/");
                    s.contains("@anthropic-ai/claude-code/") && s.ends_with("cli.js")
                })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resume_keeps_exact_session_directory_model_and_manual_permissions() {
        let dir = crate::test_support::FixtureDir::new();
        let exe = Executable {
            program: "fixture-claude.exe".into(),
            prefix: vec![],
        };
        let c = resume_command(&exe, dir.path(), "fixture-session", "fixture-model");
        let raw = c.as_std();
        assert_eq!(raw.get_current_dir(), Some(dir.path()));
        let args: Vec<_> = raw.get_args().map(|s| s.to_string_lossy()).collect();
        for pair in [
            ["--resume", "fixture-session"],
            ["--model", "fixture-model"],
            ["--permission-mode", "manual"],
        ] {
            assert!(args.windows(2).any(|a| a == pair));
        }
        assert!(
            !args
                .iter()
                .any(|s| s.contains("skip-permissions") || s.contains("fork-session"))
        );
    }
    #[tokio::test]
    async fn stopping_owned_process_waits_for_termination() {
        let exe = Executable {
            program: std::env::var_os("SystemRoot").unwrap().into(),
            prefix: vec![],
        };
        let exe = Executable {
            program: exe
                .program
                .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
            prefix: vec![],
        };
        let dir = crate::test_support::FixtureDir::new();
        let mut child = command(&exe, dir.path())
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 120",
            ])
            .spawn()
            .unwrap();
        let job = crate::platform::process_job::ProcessJob::attach(&child).unwrap();
        assert!(child.try_wait().unwrap().is_none());
        stop_process(&mut child, job).await.unwrap();
        assert!(child.try_wait().unwrap().is_some());
    }
}
