//! 指纹浏览器自检：在浏览器自己的进程里量环境，再把结果回传给主程序。
//!
//! 主窗口是 WebView2，指纹浏览器是另一套 Chromium（独立资料目录、独立
//! `--fingerprint` 种子），字体和 Emoji 这类系统级信号可以共用结论，但 UA、
//! 语言、时区、硬件并发必须现场量。做法是起一个只绑 127.0.0.1 的短命 HTTP
//! 服务：页面与脚本由主程序下发，脚本在浏览器里跑完把 JSON POST 回来。
//!
//! 边界：端口随机、票据一次性、最长 120 秒、请求体上限 256 KiB，只回显
//! 本模块生成的页面；脚本由前端 `selfcheck/probe.ts` 生成，不接受外部输入。

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use std::{
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

/// 一次自检最多等多久；超时后端口自动关闭，页面再回传也没人收。
const LIFETIME: Duration = Duration::from_secs(120);
const MAX_BODY: usize = 256 * 1024;
const MAX_SCRIPT: usize = 256 * 1024;

#[derive(Default)]
pub struct SelfcheckState(Mutex<Option<Session>>);

struct Session {
    ticket: String,
    report: Arc<Mutex<Option<Value>>>,
    started: Instant,
    stop: Arc<AtomicBool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelfcheckStart {
    ticket: String,
    url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelfcheckStatus {
    ready: bool,
    report: Option<Value>,
    expires_in: u64,
}

struct Endpoint {
    ticket: String,
    html: String,
    report: Arc<Mutex<Option<Value>>>,
    stop: Arc<AtomicBool>,
}

const TEMPLATE: &str = r#"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>指纹浏览器自检</title>
<style>
body{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;
font:14px/1.7 system-ui,-apple-system,"Segoe UI",sans-serif;background:#fafafa;color:#18181b}
main{max-width:560px;padding:32px 36px;background:#fff;border:1px solid #e4e4e7;border-radius:16px}
h1{margin:0 0 8px;font-size:16px;font-weight:600}
p{margin:0;color:#71717a}
#selfcheck-note{margin-top:12px;font-size:12px;color:#a1a1aa;word-break:break-all}
</style>
</head>
<body>
<main>
<h1 id="selfcheck-status">正在采集指纹浏览器环境…</h1>
<p>约 1 秒完成，结果回传后本页会尝试自动关闭。</p>
<p id="selfcheck-note"></p>
</main>
<script>__SCRIPT__</script>
</body>
</html>
"#;

fn page(script: &str) -> String {
    TEMPLATE.replace("__SCRIPT__", script)
}

fn text(request: Request, status: StatusCode, body: &str) {
    let header =
        Header::from_bytes("Content-Type", "text/plain; charset=utf-8").expect("静态响应头合法");
    let _ = request.respond(
        Response::from_string(body.to_string())
            .with_status_code(status)
            .with_header(header),
    );
}

fn html(request: Request, body: &str) {
    let kind =
        Header::from_bytes("Content-Type", "text/html; charset=utf-8").expect("静态响应头合法");
    let cache = Header::from_bytes("Cache-Control", "no-store").expect("静态响应头合法");
    let _ = request.respond(
        Response::from_string(body.to_string())
            .with_status_code(StatusCode(200))
            .with_header(kind)
            .with_header(cache),
    );
}

fn read_body(request: &mut Request) -> Option<String> {
    if request
        .body_length()
        .is_some_and(|length| length > MAX_BODY)
    {
        return None;
    }
    let mut buffer = String::new();
    request
        .as_reader()
        .take(MAX_BODY as u64)
        .read_to_string(&mut buffer)
        .ok()?;
    Some(buffer)
}

fn handle(mut request: Request, endpoint: &Endpoint) {
    let url = request.url().to_string();
    let method = request.method().clone();
    let (path, query) = match url.split_once('?') {
        Some((path, query)) => (path, query),
        None => (url.as_str(), ""),
    };
    let ticket = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("t="))
        .unwrap_or_default();
    let allowed = ticket == endpoint.ticket;
    match (method, path) {
        (Method::Get, "/") if allowed => html(request, &endpoint.html),
        (Method::Post, "/report") if allowed => match read_body(&mut request)
            .and_then(|body| serde_json::from_str::<Value>(&body).ok())
        {
            Some(value) => {
                *endpoint.report.lock() = Some(value);
                text(request, StatusCode(200), r#"{"ok":true}"#);
                // 结果已收到，端口不再需要对外。
                endpoint.stop.store(true, Ordering::Release);
            }
            None => text(
                request,
                StatusCode(400),
                r#"{"ok":false,"error":"报告格式无效"}"#,
            ),
        },
        (Method::Get, "/favicon.ico") => text(request, StatusCode(204), ""),
        (Method::Get, "/") | (Method::Post, "/report") => {
            text(request, StatusCode(403), "ticket 无效或已过期")
        }
        _ => text(request, StatusCode(404), "not found"),
    }
}

fn serve(server: Server, endpoint: Endpoint, lifetime: Duration) {
    let deadline = Instant::now() + lifetime;
    while !endpoint.stop.load(Ordering::Acquire) && Instant::now() < deadline {
        let request = match server.recv_timeout(Duration::from_millis(200)) {
            Ok(Some(request)) => request,
            Ok(None) => continue,
            Err(_) => break,
        };
        handle(request, &endpoint);
    }
}

/// 启动一次自检：起本地服务 → 记录会话 → 在指纹浏览器里打开页面。
#[tauri::command]
pub async fn browser_selfcheck_start(
    app: AppHandle,
    script: String,
) -> Result<SelfcheckStart, String> {
    if script.len() > MAX_SCRIPT || script.contains("</script") {
        return Err("自检脚本无效".into());
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("无法启动自检端口：{error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("无法启动自检端口：{error}"))?
        .port();
    let server = Server::from_listener(listener, None)
        .map_err(|error| format!("无法启动自检端口：{error}"))?;

    let ticket = uuid::Uuid::new_v4().simple().to_string();
    let report: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
    let stop = Arc::new(AtomicBool::new(false));
    let endpoint = Endpoint {
        ticket: ticket.clone(),
        html: page(&script),
        report: report.clone(),
        stop: stop.clone(),
    };

    {
        let state = app.state::<SelfcheckState>();
        let mut guard = state.0.lock();
        // 上一轮还没结束时先关掉它的端口，避免同时挂着多个自检服务。
        if let Some(previous) = guard.as_ref() {
            previous.stop.store(true, Ordering::Release);
        }
        *guard = Some(Session {
            ticket: ticket.clone(),
            report,
            started: Instant::now(),
            stop: stop.clone(),
        });
    }

    std::thread::Builder::new()
        .name("yunkuo-selfcheck".into())
        .spawn(move || serve(server, endpoint, LIFETIME))
        .map_err(|error| format!("无法启动自检线程：{error}"))?;

    let url = format!("http://127.0.0.1:{port}/?t={ticket}");
    // 与正常打开浏览器同一套前置校验：代理、TUN、出站模式不满足就不启动。
    if let Err(error) = crate::fingerprint_browser::browser::launch_url(&app, &url).await {
        stop.store(true, Ordering::Release);
        *app.state::<SelfcheckState>().0.lock() = None;
        return Err(error);
    }
    Ok(SelfcheckStart { ticket, url })
}

#[tauri::command]
pub fn browser_selfcheck_status(app: AppHandle, ticket: String) -> SelfcheckStatus {
    let state = app.state::<SelfcheckState>();
    let guard = state.0.lock();
    let Some(session) = guard.as_ref().filter(|session| session.ticket == ticket) else {
        return SelfcheckStatus {
            ready: false,
            report: None,
            expires_in: 0,
        };
    };
    let report = session.report.lock().clone();
    // 回传完成后自检窗口就没有用了；命令行打开的标签页不允许脚本自己关，
    // 所以由这边结束掉那个临时实例，避免桌面上多留一个空白浏览器。
    if report.is_some() {
        app.state::<crate::fingerprint_browser::browser::Runtime>()
            .stop_selfcheck();
    }
    SelfcheckStatus {
        ready: report.is_some(),
        report,
        expires_in: LIFETIME.saturating_sub(session.started.elapsed()).as_secs(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn call(port: u16, raw: &str) -> String {
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("连接自检端口");
        stream.write_all(raw.as_bytes()).expect("写入请求");
        stream.flush().expect("刷新请求");
        let mut buffer = String::new();
        stream.read_to_string(&mut buffer).expect("读取响应");
        buffer
    }

    fn start(
        ticket: &str,
    ) -> (
        u16,
        Arc<Mutex<Option<Value>>>,
        Arc<AtomicBool>,
        std::thread::JoinHandle<()>,
    ) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("绑定端口");
        let port = listener.local_addr().expect("读取端口").port();
        let server = Server::from_listener(listener, None).expect("启动服务");
        let report: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let endpoint = Endpoint {
            ticket: ticket.into(),
            html: page("/* fixture */"),
            report: report.clone(),
            stop: stop.clone(),
        };
        let handle = std::thread::spawn(move || serve(server, endpoint, Duration::from_secs(5)));
        (port, report, stop, handle)
    }

    #[test]
    fn serves_page_and_collects_report() {
        let (port, report, _stop, handle) = start("abc123");
        let page_response = call(
            port,
            "GET /?t=abc123 HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        );
        assert!(page_response.contains("200 OK"), "{page_response}");
        assert!(page_response.contains("指纹浏览器自检"), "{page_response}");

        let body = r#"{"at":1,"ua":"fixture"}"#;
        let response = call(
            port,
            &format!(
                "POST /report?t=abc123 HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ),
        );
        assert!(response.contains("200 OK"), "{response}");
        assert_eq!(report.lock().as_ref().unwrap()["ua"], "fixture");
        // 收到结果后服务自行退出。
        handle.join().expect("自检线程退出");
    }

    #[test]
    fn wrong_ticket_is_rejected() {
        let (port, report, stop, handle) = start("abc123");
        let response = call(
            port,
            "POST /report?t=guess HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
        );
        assert!(response.contains("403"), "{response}");
        assert!(report.lock().is_none());
        stop.store(true, Ordering::Release);
        handle.join().expect("自检线程退出");
    }
}
