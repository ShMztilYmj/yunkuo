use super::{
    model::{self, Compatibility, Quota},
    native::Desktop,
    source,
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
};

fn command(exe: &Path, home: &Path) -> Command {
    let mut c = Command::new(exe);
    c.env("CODEX_HOME", home)
        .current_dir(home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .creation_flags(0x08000000);
    c
}
pub async fn compatible(home: &Path, d: &Desktop) -> Result<String> {
    source::schemas(home)?;
    let mut c = command(&d.exe, home);
    c.arg("--version");
    let out = tokio::time::timeout(Duration::from_secs(8), c.output())
        .await
        .context("Codex 版本核验超时")??;
    if !out.status.success() {
        bail!("Codex 引擎版本核验失败");
    }
    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let mut c = command(&d.exe, home);
    c.args(["queue", "--help"]);
    let out = tokio::time::timeout(Duration::from_secs(8), c.output())
        .await
        .context("Codex 消息接口核验超时")??;
    let help = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() || !help.contains("--thread") || !help.contains("--message") {
        bail!("此 Codex 版本不支持定向消息队列");
    }
    Ok(model::label(&version, 80))
}
pub async fn report(home: &Path) -> Compatibility {
    let now = chrono::Utc::now().timestamp();
    let result = async {
        let d = super::native::desktop(home)?;
        let version = compatible(home, &d).await?;
        let a = source::account(home)?;
        let (_, q) = usage(home, &d, &a).await?;
        if q.available.is_none() {
            bail!("实时额度格式尚不能识别");
        }
        Ok::<_, anyhow::Error>(version)
    }
    .await;
    match result {
        Ok(v) => Compatibility {
            supported: true,
            engine_version: Some(v),
            message: "引擎、只读历史和实时额度可用；发送前仍会核验目标会话".into(),
            checked_at: Some(now),
        },
        Err(e) => Compatibility {
            supported: false,
            message: public_error(&e),
            checked_at: Some(now),
            ..Default::default()
        },
    }
}
pub fn public_error(error: &anyhow::Error) -> String {
    // 只返回本模块自己构造的短中文消息；SQLite/IO 细节可能带出私有路径或凭据。
    super::super::public_error(error, "本机状态读取失败，稍后重试或重新执行兼容检查")
}
async fn write(input: &mut tokio::process::ChildStdin, v: Value) -> Result<()> {
    input.write_all(format!("{v}\n").as_bytes()).await?;
    input.flush().await?;
    Ok(())
}
async fn rpc(home: &Path, d: &Desktop, method: &str, params: Value) -> Result<Value> {
    if !matches!(
        method,
        "account/read" | "account/rateLimits/read" | "thread/queue/delete"
    ) {
        bail!("不允许的查询接口");
    }
    let mut c = command(&d.exe, home);
    c.arg("app-server").stdin(Stdio::piped());
    let mut child = c.spawn().context("无法启动只读额度查询")?;
    let result=tokio::time::timeout(Duration::from_secs(22),async{
        let mut input=child.stdin.take().context("查询输入不可用")?;
        let mut output=BufReader::new(child.stdout.take().context("查询输出不可用")?).lines();
        write(&mut input,json!({"id":1,"method":"initialize","params":{"clientInfo":{"name":"yunkuo_auto_resume","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}})).await?;
        loop {
            let line=output.next_line().await?.context("额度查询连接已退出")?;
            if line.len()>1024*1024{bail!("额度查询响应过大");}
            let v:Value=serde_json::from_str(&line)?;
            if v.get("method").is_some()&&v.get("id").is_some(){write(&mut input,json!({"id":v["id"],"error":{"code":-32601,"message":"Unsupported client request"}})).await?;continue;}
            if v["id"]==1 {
                if !v["error"].is_null(){bail!("额度查询初始化失败");}
                let got=Path::new(v["result"]["codexHome"].as_str().context("额度查询未返回目录")?).canonicalize()?;
                if got!=home||v["result"]["platformOs"]!="windows"{bail!("额度查询与桌面数据目录不匹配");}
                write(&mut input,json!({"method":"initialized"})).await?;
                write(&mut input,json!({"id":2,"method":method,"params":params})).await?;
            }else if v["id"]==2 {
                if !v["error"].is_null(){bail!("实时额度或队列接口暂不可用");}return Ok(v["result"].clone());
            }
        }
    }).await;
    let _ = child.kill().await;
    let _ = child.wait().await;
    result.context("实时额度查询超时")?
}
pub async fn usage(home: &Path, d: &Desktop, account: &source::Account) -> Result<(i64, Quota)> {
    let v = rpc(home, d, "account/read", json!({"refreshToken":false})).await?;
    if v["account"]["type"] != "chatgpt"
        || v["account"]["email"].as_str().map(str::to_lowercase) != Some(account.email.clone())
    {
        bail!("当前登录与守候账号不一致");
    }
    let v = rpc(home, d, "account/rateLimits/read", json!({})).await?;
    if source::account(home)?.key != account.key {
        bail!("查询期间账号发生变化");
    }
    let now = chrono::Utc::now().timestamp();
    Ok((now, model::quota(&v, now)))
}
pub fn spawn(home: &Path, d: &Desktop, thread: &str, message: &str) -> Result<Child> {
    if !model::valid_id(thread) || message.len() > 8192 || message.contains('\0') {
        bail!("继续消息参数无效");
    }
    command(&d.exe, home)
        .args(["queue", "--thread", thread, "--message", message])
        .spawn()
        .context("消息进程未能启动")
}
pub async fn finish(mut child: Child, thread: &str) -> Option<String> {
    let out = tokio::time::timeout(Duration::from_secs(35), async {
        let stdout = child.stdout.take()?;
        let mut buf = String::new();
        tokio::io::AsyncReadExt::take(stdout, 4096)
            .read_to_string(&mut buf)
            .await
            .ok()?;
        let status = child.wait().await.ok()?;
        if status.success() {
            parse_receipt(&buf, thread)
        } else {
            None
        }
    })
    .await;
    if out.is_err() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    out.ok().flatten()
}
fn parse_receipt(text: &str, thread: &str) -> Option<String> {
    let rest = text.trim().strip_prefix("Queued message ")?;
    let (id, tail) = rest.split_once(" for thread ")?;
    (model::valid_id(id) && tail == format!("{thread}.")).then(|| id.to_owned())
}
pub async fn delete(home: &Path, d: &Desktop, thread: &str, id: &str) -> bool {
    if !model::valid_id(thread) || !model::valid_id(id) {
        return false;
    }
    rpc(
        home,
        d,
        "thread/queue/delete",
        json!({"threadId":thread,"queuedSubmissionId":id}),
    )
    .await
    .is_ok_and(|v| v["deleted"] == true)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receipt_is_bound_to_exact_thread() {
        let thread = uuid::Uuid::new_v4().to_string();
        let id = uuid::Uuid::new_v4().to_string();
        assert_eq!(
            parse_receipt(
                &format!("Queued message {id} for thread {thread}.\n"),
                &thread
            ),
            Some(id)
        );
        assert!(parse_receipt("ok", &thread).is_none());
        assert!(
            parse_receipt(
                &format!(
                    "Queued message {} for thread {}.",
                    uuid::Uuid::new_v4(),
                    uuid::Uuid::new_v4()
                ),
                &thread
            )
            .is_none()
        );
    }
}
