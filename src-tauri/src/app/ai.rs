//! AI 启动门的命令实现：启动 Claude 与其他 AI 客户端前的统一校验。

use crate::ai::execution_policy::LaunchResult;
use crate::config::AppConfig;
use crate::platform::state::{ConfigUpdateMutex, data_dir};
use tauri::{AppHandle, Manager};

/// 所有启动入口共用会话内的检测结果，并复检系统状态。
///
/// `confirmed` 表示用户已经在提示框里确认过：此时只有“系统代理 + TUN”这类
/// 硬性要求仍然拦截，出口地区、时区、连通性等降级为提示。
pub(crate) async fn check_launch_locked(
    app: &AppHandle,
    tool: &str,
    confirmed: bool,
) -> (AppConfig, LaunchResult) {
    let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
    if let Err(error) = crate::ai::management::require_ready(&data_dir(app)) {
        return (
            cfg,
            LaunchResult {
                ok: false,
                failures: vec![format!("{error:#}")],
            },
        );
    }
    let report = match crate::diagnostics::report::snapshot(app) {
        Ok(report) => report,
        Err(reason) => {
            return (
                cfg,
                LaunchResult {
                    ok: false,
                    failures: vec![reason],
                },
            );
        }
    };
    if let Err(error) = crate::platform::files::environment_root(app)
        .and_then(|dir| crate::ai::identity::vault::require_ready(&dir.join("vault")))
    {
        return (
            cfg,
            LaunchResult {
                ok: false,
                failures: vec![format!("{error}")],
            },
        );
    }
    let dd = data_dir(app);
    // 共同保护不变，服务连通性按工具及其实际用户级连接单独检测。
    let mut common = cfg.clone();
    common.require_claude_reachable = false;
    let native = crate::diagnostics::detect::detect_native(&common, &dd).await;
    let (mut ok, mut failures) =
        crate::diagnostics::detect::gate_passes(&common, &native, report.score());
    if confirmed {
        // 已确认：只保留硬性要求（系统代理、TUN、内核），其余检查不再拦截。
        let strict = crate::diagnostics::detect::strict_failures(&native);
        ok = strict.is_empty();
        failures = strict;
    }
    if ok && !confirmed && cfg.require_claude_reachable && ["claude", "chatgpt"].contains(&tool) {
        match crate::ai::tool_setup::test_connection(tool, &cfg).await {
            Ok(probe) if probe.reachable => {}
            Ok(probe) => {
                ok = false;
                failures.push(format!("服务连通性：{} · {}", probe.endpoint, probe.note));
            }
            Err(error) => {
                ok = false;
                failures.push(format!("服务配置检查未通过：{error}"));
            }
        }
    }
    // 检查期间若开始了新一轮检测，旧结果不能继续授权启动。
    if !confirmed && !crate::diagnostics::report::still_current(app, &report) {
        ok = false;
        failures.push("环境或检测依据已变化，请重新检测后启动".into());
    }
    (cfg, LaunchResult { ok, failures })
}

// ---------- 启动 Claude ----------

/// 启动 Claude（后端重新执行门控校验）
#[tauri::command]
pub async fn launch_claude(app: AppHandle, confirmed: bool) -> Result<LaunchResult, String> {
    launch_tool(&app, "claude", confirmed).await
}

/// 已支持的 AI 工具共用环境门控和本地启动方式。
/// `confirmed` 为真时，只有系统代理与 TUN 这类硬性要求仍然拦截。
#[tauri::command]
pub async fn launch_ai(
    app: AppHandle,
    tool: String,
    confirmed: bool,
) -> Result<LaunchResult, String> {
    launch_tool(&app, &tool, confirmed).await
}

async fn launch_tool(app: &AppHandle, tool: &str, confirmed: bool) -> Result<LaunchResult, String> {
    if !crate::config::AI_TOOL_KEYS.contains(&tool) {
        return Err(format!("未知 AI 工具: {tool}"));
    }
    let ai_operation = app.state::<crate::ai::management::State>();
    let _ai_operation = ai_operation.operation.lock().await;
    let environment_operation = app.state::<crate::platform::state::EnvironmentOperation>();
    let _environment_operation = environment_operation.lock().await;
    let operation = app.state::<ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    let configured = app
        .state::<crate::config::ConfigMutex>()
        .lock()
        .tool_shortcut(tool)
        .unwrap_or_default()
        .to_string();
    if configured.trim().is_empty() {
        return Err("启动入口未配置，请先检测安装或选择快捷方式".into());
    }
    if !std::path::Path::new(configured.trim()).is_file() {
        return Err("启动入口不存在或已移动，请重新检测安装或更换快捷方式".into());
    }
    let (cfg, verdict) = check_launch_locked(app, tool, confirmed).await;
    if !verdict.ok {
        return Ok(verdict);
    }
    let shortcut = cfg
        .tool_shortcut(tool)
        .filter(|path| !path.trim().is_empty())
        .ok_or("请先为工具选择启动方式")?;
    crate::ai::launcher::launch_shortcut(shortcut).map_err(|e| format!("{e:#}"))?;
    Ok(LaunchResult {
        ok: true,
        failures: vec![],
    })
}

/// AI 消费者只依赖函数契约；应用编排负责组装跨模块检查。
pub(crate) fn check_execution(
    app: AppHandle,
    tool: String,
) -> crate::ai::execution_policy::CheckFuture {
    Box::pin(async move { check_launch_locked(&app, &tool, false).await })
}
