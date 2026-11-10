//! Opt-in release UI measurements. This entry never enters the production setup.
//! All business IPC is a fixture; only the real bundled frontend and WebView2 run.
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{RunEvent, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

/// Only a fully persisted measurement result permits the event loop to exit.
/// Unexpected native lifecycle events fail this probe, never the product itself.
#[derive(Default)]
struct Completion {
    result_committed: bool,
    exit_code: i32,
    unexpected_exit: Option<String>,
}

fn lifecycle(output: &Path, event: &str, details: Value) {
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(output.join("lifecycle.jsonl"))
    {
        let _ = writeln!(
            file,
            "{}",
            json!({
                "atUtc":chrono::Utc::now().to_rfc3339(), "pid":std::process::id(),
                "event":event, "details":details
            })
        );
        let _ = file.sync_data();
    }
}

fn pending_failure(completion: &Mutex<Completion>) -> Result<(), String> {
    match completion.lock().unwrap().unexpected_exit.clone() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn failure_result(error: &str, process_start: Instant) -> Value {
    json!({
        "schema":1, "mode":"release-native-WebView2-bundled-frontend-fixture-ipc-real-wall-clock",
        "completed":false, "measureCompleted":false, "error":error,
        "finishedAtUtc":chrono::Utc::now().to_rfc3339(),
        "processWallSeconds":process_start.elapsed().as_secs_f64(),
        "frontendObservationPassed":false, "fixtureComplete":false,
        "productionEndToEndPassed":false
    })
}

#[derive(Clone)]
struct Options {
    output: PathBuf,
    duration: u64,
    interval: u64,
    route_seconds: u64,
    routes: Vec<String>,
    hidden: bool,
    agents: u64,
}

fn options() -> Result<Options, String> {
    let mut options = Options {
        output: PathBuf::new(),
        duration: 120,
        interval: 10,
        route_seconds: 15,
        routes: vec![
            "/".into(),
            "/overview".into(),
            "/environment".into(),
            "/ai".into(),
            "/workspace/chat".into(),
            "/teams".into(),
            "/logs".into(),
        ],
        hidden: false,
        agents: 0,
    };
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        if argument == "--hidden" {
            options.hidden = true;
            continue;
        }
        let value = args
            .next()
            .ok_or_else(|| format!("缺少参数值：{argument}"))?;
        let number = || {
            value
                .parse::<u64>()
                .map_err(|_| format!("参数必须为整数：{argument}"))
        };
        match argument.as_str() {
            "--output" => options.output = PathBuf::from(value),
            "--duration-seconds" => options.duration = number()?,
            "--interval-seconds" => options.interval = number()?,
            "--route-seconds" => options.route_seconds = number()?,
            "--agent-count" => options.agents = number()?,
            "--routes" => options.routes = value.split(',').map(str::to_string).collect(),
            _ => return Err(format!("未知参数：{argument}")),
        }
    }
    let allowed = [
        "/",
        "/overview",
        "/environment",
        "/fingerprint-browser",
        "/ai",
        "/workspace/chat",
        "/teams",
        "/logs",
        "/settings?tab=proxy",
        "/detect",
        "/proxies",
        "/connections",
        "/rules",
    ];
    if !options.output.is_absolute()
        || !(1..=43_200).contains(&options.duration)
        || !(1..=300).contains(&options.interval)
        || !(2..=3_600).contains(&options.route_seconds)
        || options.agents > 64
        || options.routes.is_empty()
        || options
            .routes
            .iter()
            .any(|route| !allowed.contains(&route.as_str()))
    {
        return Err("需要绝对 --output；duration 1..43200、interval 1..300、route 2..3600、agent-count 0..64；routes 必须为允许的本地路由".into());
    }
    Ok(options)
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    // Readers never observe partially written ready/result JSON. Flush the file
    // before publishing it and before allowing a successful native exit.
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    fs::rename(temporary, path).map_err(|e| e.to_string())
}

fn append_sample(output: &Path, value: &Value) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(output.join("samples.jsonl"))
        .map_err(|e| e.to_string())?;
    writeln!(file, "{value}").map_err(|e| e.to_string())
}

async fn inspect(window: &WebviewWindow, script: &str) -> Result<Value, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let sender = Mutex::new(Some(sender));
    window
        .eval_with_callback(script, move |value| {
            if let Some(sender) = sender.lock().unwrap().take() {
                let _ = sender.send(value);
            }
        })
        .map_err(|e| e.to_string())?;
    let text = tokio::time::timeout(Duration::from_secs(8), receiver)
        .await
        .map_err(|_| "WebView2 执行采样超过 8 秒".to_string())?
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| format!("采样 JSON 无效：{e}"))
}

async fn measure(
    window: WebviewWindow,
    options: Options,
    process_start: Instant,
    profile: PathBuf,
    completion: Arc<Mutex<Completion>>,
) -> Result<Value, String> {
    let startup_limit = Instant::now() + Duration::from_secs(45);
    let first = loop {
        pending_failure(&completion)?;
        let sample = inspect(&window, "window.__nativeUiProbe?.snapshot() ?? null").await?;
        if sample["rendered"].as_bool() == Some(true) {
            break sample;
        }
        if Instant::now() >= startup_limit {
            return Err(format!("发布版前端没有在 45 秒内显示：{sample}"));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let start = Instant::now();
    let ready = json!({
        "schema":1, "mode":"release-native-WebView2-bundled-frontend-fixture-ipc-real-wall-clock",
        "pid":std::process::id(), "readyAtUtc":chrono::Utc::now().to_rfc3339(),
        "startupWallMs":process_start.elapsed().as_secs_f64()*1000.0,
        "durationSeconds":options.duration, "intervalSeconds":options.interval,
        "routeSeconds":options.route_seconds, "routes":options.routes, "fixtureAgentCount":options.agents,
        "profile":profile, "visible":window.is_visible().ok(), "focused":window.is_focused().ok(),
        "hiddenRequested":options.hidden, "webviewVersion":tauri::webview_version().ok(),
        "releaseBuild":!cfg!(debug_assertions), "firstSample":first,
        "limitations":["All business IPC, agent output and sockets are synthetic fixtures; no production backend, account, proxy or agent is exercised.",
          "Windows WebView2 default background/occlusion throttling is preserved; hidden or occluded windows can suspend rAF and throttle timers.",
          "JavaScript heap metrics are browser-provided estimates and may be unavailable; process-tree memory must be sampled externally.",
          "Long-task/rAF observation itself adds a small measurement cost; rAF samples are bounded to 2048 per interval.",
          "New isolated WebView2 profile is retained in the output directory for review; no real user profile is loaded."]
    });
    write_json(&options.output.join("ready.json"), &ready)?;
    let mut sample_count = 0;
    let mut route_index = 0;
    let mut next_route = Duration::from_secs(options.route_seconds);
    let mut next_sample = Duration::ZERO;
    let mut route_since = Instant::now();
    let mut issues = Vec::<Value>::new();
    let mut last = first;
    let reason = loop {
        pending_failure(&completion)?;
        let elapsed = start.elapsed();
        if options.output.join("STOP").exists() {
            break "stop-file";
        }
        if elapsed >= Duration::from_secs(options.duration) {
            break "duration";
        }
        if elapsed >= next_route && options.routes.len() > 1 {
            route_index = (route_index + 1) % options.routes.len();
            window
                .eval(format!(
                    "window.__nativeUiProbe.navigate({})",
                    serde_json::to_string(&options.routes[route_index]).unwrap()
                ))
                .map_err(|e| e.to_string())?;
            next_route += Duration::from_secs(options.route_seconds);
            route_since = Instant::now();
        }
        if elapsed >= next_sample {
            last = inspect(&window, "window.__nativeUiProbe.snapshot()").await?;
            let settled = route_since.elapsed() >= Duration::from_secs(2);
            let sample = json!({"sequence":sample_count, "atUtc":chrono::Utc::now().to_rfc3339(),
                "elapsedSeconds":start.elapsed().as_secs_f64(), "requestedRoute":options.routes[route_index],
                "routeSettled":settled, "ui":last});
            if settled
                && (last["rendered"].as_bool() != Some(true)
                    || last["errorBoundary"].as_bool() == Some(true))
            {
                issues.push(json!({"kind":"render-observation", "elapsedSeconds":start.elapsed().as_secs_f64(), "route":options.routes[route_index]}));
            }
            append_sample(&options.output, &sample)?;
            sample_count += 1;
            next_sample += Duration::from_secs(options.interval);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    last = inspect(&window, "window.__nativeUiProbe.snapshot()")
        .await
        .unwrap_or(last);
    pending_failure(&completion)?;
    let fixture_errors = last["fixtureErrorsTotal"].as_u64().unwrap_or(0);
    let runtime_errors = last["runtimeErrorsTotal"].as_u64().unwrap_or(0);
    Ok(json!({
        "schema":1, "mode":ready["mode"], "completed":true, "measureCompleted":true, "stopReason":reason,
        "startedAtUtc":ready["readyAtUtc"], "finishedAtUtc":chrono::Utc::now().to_rfc3339(),
        "actualWallSeconds":start.elapsed().as_secs_f64(), "startupWallMs":ready["startupWallMs"],
        "requestedDurationSeconds":options.duration, "samples":sample_count, "routes":options.routes,
        "fixtureAgentCount":options.agents, "profile":profile, "lastSample":last,
        "renderObservations":issues, "fixtureErrorsTotal":fixture_errors, "runtimeErrorsTotal":runtime_errors,
        "frontendObservationPassed":issues.is_empty() && runtime_errors==0,
        "fixtureComplete":fixture_errors==0, "productionEndToEndPassed":false,
        "limitations":ready["limitations"]
    }))
}

pub fn run() -> i32 {
    let process_start = Instant::now();
    let options = match options() {
        Ok(options) => options,
        Err(_) => return 2,
    };
    if fs::create_dir_all(&options.output).is_err() {
        return 2;
    }
    // An output directory is one run. Refuse stale ready/result/STOP files.
    if [
        "ready.json",
        "result.json",
        "STOP",
        "samples.jsonl",
        "lifecycle.jsonl",
    ]
    .iter()
    .any(|name| options.output.join(name).exists())
    {
        return 2;
    }
    let profile = options
        .output
        .join(format!("webview-profile-{}", uuid::Uuid::new_v4().simple()));
    let output = options.output.clone();
    let fallback_output = output.clone();
    let completion = Arc::new(Mutex::new(Completion {
        exit_code: 1,
        ..Completion::default()
    }));
    let setup_completion = completion.clone();
    lifecycle(
        &output,
        "probe-start",
        json!({"hidden":options.hidden,"durationSeconds":options.duration}),
    );
    let mut context = tauri::generate_context!();
    context.config_mut().app.windows.clear();
    context.config_mut().build.dev_url = None;
    context.config_mut().app.security.csp = Some(tauri::utils::config::Csp::Policy(
        "default-src 'self' data: blob:; script-src 'self' 'unsafe-inline' 'unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'none'; frame-src 'none'; object-src 'none'".into()));
    // Native Tauri defines immutable bridge members. Keep the browser preview mock
    // in a lexical fixture object and intercept the isolated Builder's transport.
    let preview = include_str!("../../../preview-mock.js")
        .replace("window.__TAURI_INTERNALS__", "fixtureInternals")
        .replace("window.__TAURI_EVENT_PLUGIN_INTERNALS__", "fixtureEvents");
    let script = format!(
        "(()=>{{\nlet fixtureInternals, fixtureEvents;\n{}\nwindow.__nativeUiProbeOptions={};\n{}\n}})();",
        preview,
        json!({"hidden":options.hidden,"agents":options.agents,"route":options.routes[0],"version":env!("CARGO_PKG_VERSION")}),
        include_str!("performance_probe.js")
    );
    let application = tauri::Builder::default()
        .invoke_system(r#"
            Object.defineProperty(window.__TAURI_INTERNALS__, 'postMessage', {
              value: (message) => {
                Promise.resolve().then(() => window.__nativeUiFixtureInvoke(message.cmd, message.payload))
                  .then(value => window.__TAURI_INTERNALS__.runCallback(message.callback, value),
                    error => window.__TAURI_INTERNALS__.runCallback(message.error, String(error)));
              }
            });
        "#)
        .invoke_handler(|invoke| { invoke.resolver.reject("性能探针禁止调用真实业务命令"); true })
        .setup(move |app| {
            let window = WebviewWindowBuilder::new(app, "performance-probe", WebviewUrl::App("index.html".into()))
                .title("云阔 · 隔离 UI 性能测试（虚构后台）")
                .inner_size(1200.0, 800.0).visible(!options.hidden).focused(false).focusable(false)
                .data_directory(profile.clone()).initialization_script(script)
                .additional_browser_args("--disable-background-networking --disable-component-update --no-first-run --no-default-browser-check --enable-precise-memory-info --host-resolver-rules=\"MAP * ~NOTFOUND\"")
                .on_navigation(|url| url.host_str()==Some("tauri.localhost") || url.scheme()=="tauri")
                .build()?;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let measured = measure(window, options, process_start, profile, setup_completion.clone()).await;
                lifecycle(&output, "measure-returned", json!({"success":measured.is_ok()}));
                let mut state = setup_completion.lock().unwrap();
                let measured = match &state.unexpected_exit {
                    Some(error) => Err(error.clone()),
                    None => measured,
                };
                let (result, code) = match measured {
                    Ok(result) => {
                        let code = if result["frontendObservationPassed"]==true && result["fixtureComplete"]==true {0} else {1};
                        (result, code)
                    },
                    Err(error) => (failure_result(&error, process_start),1),
                };
                let result_write = write_json(&output.join("result.json"), &result);
                let code = if result_write.is_ok() {code} else {1};
                lifecycle(&output, "result-write", json!({"committed":result_write.is_ok(),"error":result_write.err(),"exitCode":code}));
                state.exit_code = code;
                state.result_committed = output.join("result.json").is_file();
                // A disk failure must still terminate with failure; run_return
                // supplies a final attempt to persist a diagnostic report.
                if !state.result_committed {
                    state.unexpected_exit = Some("性能探针结果写入失败".into());
                }
                drop(state);
                handle.exit(code);
            });
            Ok(())
        }).build(context);
    match application {
        Ok(app) => {
            let callback_completion = completion.clone();
            let callback_output = fallback_output.clone();
            let returned_code = app.run_return(move |_, event| match event {
                RunEvent::WindowEvent {
                    label,
                    event: WindowEvent::CloseRequested { api, .. },
                    ..
                } => {
                    let mut state = callback_completion.lock().unwrap();
                    lifecycle(
                        &callback_output,
                        "window-close-requested",
                        json!({"label":label,"resultCommitted":state.result_committed}),
                    );
                    if !state.result_committed {
                        api.prevent_close();
                        state
                            .unexpected_exit
                            .get_or_insert_with(|| "性能探针测量完成前收到窗口关闭请求".into());
                    }
                }
                RunEvent::WindowEvent {
                    label,
                    event: WindowEvent::Destroyed,
                    ..
                } => {
                    let mut state = callback_completion.lock().unwrap();
                    lifecycle(
                        &callback_output,
                        "window-destroyed",
                        json!({"label":label,"resultCommitted":state.result_committed}),
                    );
                    if !state.result_committed {
                        state
                            .unexpected_exit
                            .get_or_insert_with(|| "性能探针测量完成前窗口已销毁".into());
                    }
                }
                RunEvent::ExitRequested { code, api, .. } => {
                    let mut state = callback_completion.lock().unwrap();
                    lifecycle(
                        &callback_output,
                        "exit-requested",
                        json!({"code":code,"resultCommitted":state.result_committed}),
                    );
                    if !state.result_committed
                        && state.unexpected_exit.as_deref() != Some("性能探针结果写入失败")
                    {
                        api.prevent_exit();
                        state.unexpected_exit.get_or_insert_with(|| {
                            format!("性能探针测量完成前收到退出请求：{code:?}")
                        });
                    }
                }
                RunEvent::Exit => lifecycle(&callback_output, "event-loop-exit", json!({})),
                _ => {}
            });
            lifecycle(
                &fallback_output,
                "event-loop-returned",
                json!({"exitCode":returned_code}),
            );
            let mut state = completion.lock().unwrap();
            if !state.result_committed {
                let error = state
                    .unexpected_exit
                    .as_deref()
                    .unwrap_or("性能探针事件循环在测量结果落盘前结束");
                let _ = write_json(
                    &fallback_output.join("result.json"),
                    &failure_result(error, process_start),
                );
                state.exit_code = 1;
            }
        }
        Err(error) => {
            let _ = write_json(
                &fallback_output.join("result.json"),
                &failure_result(&error.to_string(), process_start),
            );
            lifecycle(
                &fallback_output,
                "build-failed",
                json!({"error":error.to_string()}),
            );
        }
    }
    completion.lock().unwrap().exit_code
}
