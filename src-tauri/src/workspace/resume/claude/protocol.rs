//! Claude stream-json exchange. It never grants a tool permission.
use crate::workspace::resume::model::CONTINUATION;
use anyhow::Result;
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

pub type Terminal = (&'static str, &'static str);
#[derive(Debug, PartialEq)]
pub enum Event {
    Ignore,
    Accepted,
    Running,
    Stop(Terminal),
    Reply(Value, Option<Terminal>),
}
#[derive(Clone, Copy)]
pub enum Progress {
    Accepted,
    Running,
}
fn contains_marker(content: &Value, marker: &str) -> bool {
    content.as_str().is_some_and(|s| s.contains(marker))
        || content.as_array().is_some_and(|a| {
            a.iter().any(|v| {
                v["type"] == "text" && v["text"].as_str().is_some_and(|s| s.contains(marker))
            })
        })
}
pub fn event(v: &Value, session: &str, marker: &str) -> Event {
    if v["session_id"].is_string() && v["session_id"] != session {
        return Event::Stop((
            "submission_unknown",
            "CLI 返回了不同会话标识，已停止；不会重发",
        ));
    }
    if v["type"] == "control_request" {
        if v["request"]["subtype"] == "can_use_tool" {
            return Event::Reply(
                json!({"type":"control_response","response":{"subtype":"success","request_id":v["request_id"],"response":{"behavior":"deny","message":"自动续跑需要用户审批，请在 Claude Code 中确认后继续。"}}}),
                Some(("stopped", "需要用户审批，请在 Claude Code 中确认后继续")),
            );
        }
        return Event::Reply(
            json!({"type":"control_response","response":{"subtype":"error","request_id":v["request_id"],"error":"自动续跑不提供交互式批准"}}),
            None,
        );
    }
    if v["session_id"] != session {
        return Event::Ignore;
    }
    match v["type"].as_str() {
        Some("user") if contains_marker(&v["message"]["content"], marker) => Event::Accepted,
        Some("assistant")
            if v["isApiErrorMessage"] != true
                && v["message"]["model"]
                    .as_str()
                    .is_some_and(|m| !m.is_empty() && m != "<synthetic>") =>
        {
            Event::Running
        }
        Some("stream_event")
            if v["event"]["type"] == "message_start"
                && v["event"]["message"]["model"]
                    .as_str()
                    .is_some_and(|m| !m.is_empty() && m != "<synthetic>") =>
        {
            Event::Running
        }
        Some("result") => Event::Stop(
            if v["permission_denials"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
            {
                ("stopped", "存在未批准的操作，请在 Claude Code 中确认后继续")
            } else if v["is_error"] == false && v["subtype"] == "success" {
                ("completed", "Claude 本轮已完成")
            } else {
                (
                    "stopped",
                    "Claude 本轮已停止，请在原会话查看原因；不会自动重发",
                )
            },
        ),
        _ => Event::Ignore,
    }
}
pub async fn exchange<W, R>(
    input: &mut W,
    output: R,
    session: &str,
    marker: &str,
    cancelled: impl Fn() -> bool,
    mut progress: impl FnMut(Progress) -> Result<()>,
    max_duration: Duration,
) -> Result<Terminal>
where
    W: AsyncWrite + Unpin,
    R: AsyncRead + Unpin,
{
    let cancelled_result = ("cancelled", "已停止本软件启动的 Claude 恢复进程");
    if cancelled() {
        return Ok(cancelled_result);
    }
    let message = format!("{CONTINUATION}\n\n{marker}");
    input.write_all(format!("{}\n", json!({"type":"user","session_id":session,"message":{"role":"user","content":message},"parent_tool_use_id":null})).as_bytes()).await?;
    input.flush().await?;
    let mut lines = BufReader::new(output).lines();
    let start = Instant::now();
    let mut accepted = false;
    let mut running = false;
    loop {
        if cancelled() {
            return Ok(cancelled_result);
        }
        if start.elapsed() >= max_duration {
            return Ok(("stopped", "本次续跑超过 1 小时，已停止，请核对进度后继续"));
        }
        tokio::select! {
            line = lines.next_line() => {
                let Some(line) = line? else { break; };
                if line.len() > 2 * 1024 * 1024 { break; }
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue; };
                match event(&v, session, marker) {
                    Event::Ignore => {},
                    Event::Accepted => if !accepted && !running { accepted = true; progress(Progress::Accepted)?; },
                    Event::Running => if !running { running = true; progress(Progress::Running)?; },
                    Event::Stop(t) => return Ok(t),
                    Event::Reply(reply, terminal) => {
                        input.write_all(format!("{reply}\n").as_bytes()).await?;
                        input.flush().await?;
                        if let Some(t) = terminal { return Ok(t); }
                    }
                }
            },
            _ = tokio::time::sleep(Duration::from_millis(200)) => {},
        }
    }
    Ok(("submission_unknown", "恢复进程退出但结果未获确认，不会重发"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use tokio::io::AsyncReadExt;
    const ID: &str = "fixture-session";
    const MARKER: &str = "[fixture-resume]";
    #[test]
    fn unrelated_output_and_synthetic_errors_do_not_report_started() {
        for v in [
            json!({"type":"stream_event","session_id":ID,"event":{"type":"ping"}}),
            json!({"type":"assistant","session_id":ID,"isApiErrorMessage":true,"message":{"model":"<synthetic>"}}),
            json!({"type":"result","subtype":"success","is_error":false}),
        ] {
            assert_eq!(event(&v, ID, MARKER), Event::Ignore);
        }
        assert!(matches!(
            event(
                &json!({"type":"result","session_id":"another-session","subtype":"success","is_error":false}),
                ID,
                MARKER
            ),
            Event::Stop(("submission_unknown", _))
        ));
    }
    #[tokio::test]
    async fn accepted_started_and_complete_are_distinct() {
        let stream = [json!({"type":"user","session_id":ID,"message":{"content":[{"type":"text","text":MARKER}]}}), json!({"type":"stream_event","session_id":ID,"event":{"type":"ping"}}), json!({"type":"assistant","session_id":ID,"message":{"model":"fixture-model"}}), json!({"type":"result","session_id":ID,"subtype":"success","is_error":false})].iter().map(|v| format!("{v}\n")).collect::<String>();
        let mut input = Vec::new();
        let mut states = Vec::new();
        let terminal = exchange(
            &mut input,
            stream.as_bytes(),
            ID,
            MARKER,
            || false,
            |s| {
                states.push(match s {
                    Progress::Accepted => "queued",
                    Progress::Running => "running",
                });
                Ok(())
            },
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(states, ["queued", "running"]);
        assert_eq!(terminal.0, "completed");
        let sent: Value = serde_json::from_slice(&input).unwrap();
        assert_eq!(sent["session_id"], ID);
        assert!(
            sent["message"]["content"]
                .as_str()
                .unwrap()
                .contains(MARKER)
        );
    }
    #[tokio::test]
    async fn tool_approval_is_denied_and_stops_the_recovery() {
        let stream = format!(
            "{}\n",
            json!({"type":"control_request","request_id":"approval","request":{"subtype":"can_use_tool","tool_name":"Bash"}})
        );
        let mut input = Vec::new();
        let terminal = exchange(
            &mut input,
            stream.as_bytes(),
            ID,
            MARKER,
            || false,
            |_| Ok(()),
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(terminal.0, "stopped");
        let sent: Vec<Value> = String::from_utf8(input)
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1]["response"]["response"]["behavior"], "deny");
        assert_eq!(sent[1]["response"]["request_id"], "approval");
        assert!(matches!(
            event(
                &json!({"type":"result","session_id":ID,"subtype":"success","is_error":false,"permission_denials":[{}]}),
                ID,
                MARKER
            ),
            Event::Stop(("stopped", _))
        ));
    }
    #[tokio::test]
    async fn cancellation_before_input_sends_nothing_and_mid_run_stops() {
        let mut input = Vec::new();
        assert_eq!(
            exchange(
                &mut input,
                &b""[..],
                ID,
                MARKER,
                || true,
                |_| Ok(()),
                Duration::from_secs(1)
            )
            .await
            .unwrap()
            .0,
            "cancelled"
        );
        assert!(input.is_empty());
        let cancel = Arc::new(AtomicBool::new(false));
        let (mut fake, output) = tokio::io::duplex(4096);
        let flag = cancel.clone();
        let server = tokio::spawn(async move {
            let mut line = [0; 1];
            while fake.read_exact(&mut line).await.is_ok() {
                if line[0] == b'\n' {
                    break;
                }
            }
            flag.store(true, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(500)).await;
        });
        let (read, mut write) = tokio::io::split(output);
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            exchange(
                &mut write,
                read,
                ID,
                MARKER,
                || cancel.load(Ordering::SeqCst),
                |_| Ok(()),
                Duration::from_secs(1),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result.0, "cancelled");
        server.abort();
    }
    #[tokio::test]
    async fn closed_stream_without_result_is_never_retried_or_completed() {
        let result = exchange(
            &mut Vec::new(),
            &b""[..],
            ID,
            MARKER,
            || false,
            |_| Ok(()),
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(result.0, "submission_unknown");
    }
}
