//! Tauri 命令入口：读取用量快照、切换 Codex 本地账号。

use super::model::*;
use super::quota::*;
use super::scan::*;
use super::{deepseek, engine, providers, sources};
use once_cell::sync::Lazy;
use serde::Serialize;
use std::time::{Duration, Instant};
use tauri::Manager;
// ---------- 命令入口 ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSwitchResult {
    pub ok: bool,
    pub message: String,
}

async fn codex_switch(which: String) -> Result<AiSwitchResult, String> {
    let root = home().join(".codex");
    tokio::task::spawn_blocking(move || {
        crate::ai::identity::account_store::ensure_codex_closed().map_err(|e| format!("{e:#}"))?;
        crate::ai::identity::account_store::Store::new(root)
            .activate(&which)
            .map_err(|e| format!("{e:#}"))?;
        Ok(AiSwitchResult {
            ok: true,
            message:
                "已原子切换 Codex 本地认证文件；重启 Codex 后生效。ChatGPT 桌面账号不受此操作管理。"
                    .into(),
        })
    })
    .await
    .map_err(|_| "账号操作未完成".to_string())?
}

// ---------- Claude（无多账号快照时也要有当前账号可展示） ----------

// ---------- 命令入口 ----------

#[tauri::command]
pub async fn ai_usage_status(app: tauri::AppHandle) -> Result<AiSnapshot, String> {
    // 「设置 → AI 模块」里关掉本地扫描后不再读任何会话文件，直接说明原因。
    let config = app.state::<crate::config::ConfigMutex>().lock().clone();
    if !config.ai_usage_scan {
        return Err("本地用量扫描已在「设置 → AI 模块」中关闭".into());
    }
    // 额度查询经本机混合端口出网，避免把真实出口 IP 暴露给服务商。
    crate::ai::aimon::set_proxy_port(config.mixed_port);
    let scope = crate::ai::agent_tools::MonitoringScope::current(&app);
    // 只有 Agent 进程在运行时才查询额度；关闭后沿用最后一次快照。
    let running = running_agents(&config);
    let snapshot = collect_scope(scope.clone(), running).await?;
    if scope != crate::ai::agent_tools::MonitoringScope::current(&app) {
        return Err("已添加 Agent 名单已更新，请重新读取统计".into());
    }
    Ok(snapshot)
}

/// 当前在运行的 Agent：进程清单只读，5 秒内复用同一份结果。
fn running_agents(config: &crate::config::AppConfig) -> std::collections::HashSet<String> {
    crate::ai::agent_tools::read(config)
        .map(|tools| {
            tools
                .into_iter()
                .filter(|tool| tool.online)
                .map(|tool| tool.key)
                .collect()
        })
        .unwrap_or_default()
}

/// 模型源额度的同步状态：最近一次成功同步的时刻取各行快照时间的最大值。
fn provider_sync(rows: &[AiProviderQuota], live: bool) -> Option<AiProviderSync> {
    if rows.is_empty() {
        return None;
    }
    Some(AiProviderSync {
        live,
        interval_seconds: providers::SYNC_INTERVAL_SECS,
        last_synced_at: rows
            .iter()
            .map(|row| row.captured_at)
            .filter(|at| *at > 0)
            .max(),
    })
}

struct ClaudeRateCache {
    checked_at: Instant,
    rate: AiRateLimit,
}

/// Claude 的官方额度接口有频率限制：运行中最多每分钟核验一次；失败时保留上一份
/// 快照，下一次读取不会立刻重试，避免 UI 轮询把接口打满。
static CLAUDE_RATE: Lazy<tokio::sync::Mutex<Option<ClaudeRateCache>>> =
    Lazy::new(|| tokio::sync::Mutex::new(None));

async fn claude_live_rate(running: bool) -> AiRateLimit {
    let mut cache = CLAUDE_RATE.lock().await;
    let previous = cache.as_ref().map(|cached| cached.rate.clone());
    if !running {
        return previous.unwrap_or_default();
    }
    if let Some(cached) = cache.as_ref() {
        if cached.checked_at.elapsed() < Duration::from_secs(60) {
            return cached.rate.clone();
        }
    }
    let rate = match crate::workspace::resume::claude::usage_value().await {
        Ok(value) => claude_rate_from_usage(&value, chrono::Utc::now().timestamp()),
        Err(_) => previous.unwrap_or_default(),
    };
    *cache = Some(ClaudeRateCache {
        checked_at: Instant::now(),
        rate: rate.clone(),
    });
    rate
}

async fn collect_scope(
    scope: crate::ai::agent_tools::MonitoringScope,
    running: std::collections::HashSet<String>,
) -> Result<AiSnapshot, String> {
    let now = chrono::Utc::now().timestamp();
    let mut items = Vec::new();
    let engine = engine::snapshot(scope.clone());
    let plan = engine.native_plan(scope.clone());
    let claude_live = scope.contains("claude") && running.contains("claude");
    let dsh_live = scope.contains("deepseek") && running.contains("deepseek");
    let zcode_live = scope.contains("glm") && running.contains("glm");
    let (
        claude_agg,
        claude_rate,
        (codex_agg, rate_snap),
        deepseek_agg,
        extra,
        deepseek_providers,
        zcode_providers,
    ) = tokio::join!(
        collect_claude(plan.mode("claude")),
        claude_live_rate(claude_live),
        collect_codex(plan.mode("chatgpt")),
        async {
            if scope.contains("deepseek") {
                deepseek::collect(plan.mode("deepseek")).await
            } else {
                WindowAgg::default()
            }
        },
        sources::collect(plan.clone()),
        async {
            if scope.contains("deepseek") {
                providers::dsh_snapshot(dsh_live).await
            } else {
                Vec::new()
            }
        },
        async {
            if scope.contains("glm") {
                providers::zcode_snapshot(zcode_live).await
            } else {
                Vec::new()
            }
        }
    );

    // --- Claude ---
    {
        let agg = claude_agg;
        let account = read_claude_account().await;
        let note = if agg.last_ts > 0 {
            items_note(agg.last_ts, &agg.all)
        } else if account.plan.is_some() && account.email.is_some() {
            "已登录 Claude（未发现本地 Claude Code 会话）".into()
        } else {
            "未发现本地 Claude 数据（未登录或未安装）".into()
        };
        items.push(AiUsageItem {
            key: "claude".into(),
            session_available: !agg.sessions.is_empty(),
            input_includes_cached: Some(false),
            daily: daily_usage(&agg),
            recent_sessions: recent_sessions(&agg),
            last7: agg.last7,
            last30: agg.last30,
            usage_available: !agg.daily.is_empty(),
            data_source: "Claude Code 本地会话".into(),
            complete: agg.incomplete_files == 0,
            today: agg.today,
            week: agg.week,
            note,
            account,
            rate: claude_rate,
            providers: Vec::new(),
            provider_sync: None,
            last_activity_ts: if agg.last_ts > 0 {
                Some(agg.last_ts)
            } else {
                None
            },
            scanned_at: now,
            all: agg.all,
        });
    }

    // --- Codex / ChatGPT ---
    {
        let agg = codex_agg;
        let account = read_codex_account().await;
        // 额度窗口直接取本机 rollout 里最新的 rate_limits 快照（官方顺手上报的数据，无需远程）
        let rate = rate_from_snapshot(&rate_snap, chrono::Utc::now().timestamp());
        items.push(AiUsageItem {
            key: "chatgpt".into(),
            session_available: !agg.sessions.is_empty(),
            input_includes_cached: Some(true),
            daily: daily_usage(&agg),
            recent_sessions: recent_sessions(&agg),
            last7: agg.last7,
            last30: agg.last30,
            usage_available: !agg.daily.is_empty(),
            data_source: "Codex 本地会话及认证快照，与 ChatGPT 桌面账号分开管理".into(),
            complete: agg.incomplete_files == 0,
            today: agg.today,
            week: agg.week,
            note: if agg.incomplete_files > 0 {
                format!(
                    "统计不完整：{} 个会话存在旧格式边界、缺失记录或读取问题；仅展示可核验部分",
                    agg.incomplete_files
                )
            } else {
                items_note(agg.last_ts, &agg.all)
            },
            account,
            rate,
            providers: Vec::new(),
            provider_sync: None,
            last_activity_ts: if agg.last_ts > 0 {
                Some(agg.last_ts)
            } else {
                None
            },
            scanned_at: now,
            all: agg.all,
        });
    }

    // DSH 的工具来源由独立日志目录决定，与内部使用的模型无关。
    if scope.contains("deepseek") {
        let agg = deepseek_agg;
        let sync = provider_sync(&deepseek_providers, dsh_live);
        items.push(AiUsageItem {
            key: "deepseek".into(),
            usage_available: !agg.daily.is_empty(),
            complete: agg.incomplete_files == 0,
            session_available: !agg.sessions.is_empty(),
            input_includes_cached: Some(false),
            data_source: "DeepSeek Harness 本地会话".into(),
            daily: daily_usage(&agg),
            recent_sessions: recent_sessions(&agg),
            note: if agg.incomplete_files > 0 {
                format!(
                    "统计不完整：{} 个 DSH 会话存在读取问题，仅展示可核验部分",
                    agg.incomplete_files
                )
            } else if agg.last_ts > 0 {
                items_note(agg.last_ts, &agg.all)
            } else {
                "未发现 DeepSeek Harness 本地会话记录".into()
            },
            last_activity_ts: (agg.last_ts > 0).then_some(agg.last_ts),
            scanned_at: now,
            today: agg.today,
            week: agg.week,
            last7: agg.last7,
            last30: agg.last30,
            all: agg.all,
            account: AiAccount::default(),
            rate: AiRateLimit::default(),
            providers: deepseek_providers,
            provider_sync: sync,
        });
    }
    items.extend(extra);
    // ZCode 的服务商额度挂在 glm 工具条目上；条目由本地扫描创建，缺失时不凭空补。
    if scope.contains("glm") && !zcode_providers.is_empty() {
        if let Some(item) = items.iter_mut().find(|item| item.key == "glm") {
            item.providers = zcode_providers;
            let sync = provider_sync(&item.providers, zcode_live);
            item.provider_sync = sync;
        }
    }
    engine.merge(&mut items);
    crate::ai::agent_events::merge(&mut items);
    // 固定工具顺序，九款通过同一快照提供给全部界面。
    items.sort_by_key(|item| {
        crate::ai::agent_tools::order()
            .into_iter()
            .position(|key| key == item.key.as_str())
            .unwrap_or(usize::MAX)
    });

    Ok(AiSnapshot { items })
}

/// Codex 账号切换：which 为当前用户加密快照的标识。
#[tauri::command]
pub async fn ai_switch_account(
    app: tauri::AppHandle,
    tool: String,
    which: Option<String>,
) -> Result<AiSwitchResult, String> {
    if tool == "chatgpt" || tool == "codex" {
        let Some(w) = which else {
            return Err("缺少 which 参数".into());
        };
        // 与受控启动使用同一锁顺序，持锁直到认证文件替换结束。
        let environment_operation = app.state::<crate::platform::state::EnvironmentOperation>();
        let _environment_operation = environment_operation.lock().await;
        let network_operation = app.state::<crate::platform::state::ConfigUpdateMutex>();
        let _network_operation = network_operation.lock().await;
        crate::ai::identity::ExecutionHooks::pause_codex(&app)?;
        return codex_switch(w).await;
    }
    Err(format!(
        "暂不支持 {tool} 的本地多账号切换：该工具需在官方应用内切换账号"
    ))
}

/// Codex 快照管理：保存或刷新当前账号的加密快照。
#[tauri::command]
pub async fn ai_snapshot_account(tool: String) -> Result<AiSwitchResult, String> {
    if tool != "chatgpt" && tool != "codex" {
        return Err("仅支持 Codex 本地账号快照".into());
    }
    let root = home().join(".codex");
    tokio::task::spawn_blocking(move || {
        crate::ai::identity::account_store::Store::new(root)
            .save()
            .map_err(|e| format!("{e:#}"))?;
        Ok(AiSwitchResult {
            ok: true,
            message: "当前账号快照已加密保存或刷新，仅当前 Windows 用户可解密。".into(),
        })
    })
    .await
    .map_err(|_| "保存快照未完成".to_string())?
}

#[tauri::command]
pub async fn ai_migrate_accounts() -> Result<AiSwitchResult, String> {
    let root = home().join(".codex");
    tokio::task::spawn_blocking(move || {
        let count = crate::ai::identity::account_store::Store::new(root)
            .migrate()
            .map_err(|e| format!("{e:#}"))?;
        Ok(AiSwitchResult {
            ok: true,
            message: format!("已加密迁移 {count} 个旧版快照；读回校验成功后移除了对应旧明文。"),
        })
    })
    .await
    .map_err(|_| "迁移快照未完成".to_string())?
}

/// 允许暴露给前端读取的 Codex 快照列表（不含敏感 token）
#[tauri::command]
pub async fn ai_list_accounts(tool: String) -> Result<serde_json::Value, String> {
    if tool != "chatgpt" && tool != "codex" {
        return Ok(serde_json::json!([]));
    }
    let root = home().join(".codex");
    tokio::task::spawn_blocking(move || {
        let list = crate::ai::identity::account_store::Store::new(root)
            .list()
            .map_err(|e| format!("{e:#}"))?;
        serde_json::to_value(list).map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "读取快照未完成".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_sync_reports_live_state_and_last_snapshot() {
        assert!(
            provider_sync(&[], true).is_none(),
            "没有模型源时不带同步状态"
        );
        let rows = vec![
            AiProviderQuota {
                captured_at: 0,
                ..Default::default()
            },
            AiProviderQuota {
                captured_at: 1_800_000_000,
                ..Default::default()
            },
        ];
        let paused = provider_sync(&rows, false).expect("有模型源时必须有同步状态");
        assert!(!paused.live);
        assert_eq!(paused.interval_seconds, providers::SYNC_INTERVAL_SECS);
        assert_eq!(paused.last_synced_at, Some(1_800_000_000));
        assert!(provider_sync(&rows, true).unwrap().live);
    }

    #[tokio::test]
    #[ignore = "读取本机 DSH 配置与官方额度接口，验证模型源行随快照下发"]
    async fn dsh_item_carries_model_source_quotas() {
        let scope = crate::ai::agent_tools::MonitoringScope::from_cards(&["deepseek".to_string()]);
        let snapshot = collect_scope(
            scope,
            std::collections::HashSet::from(["deepseek".to_string()]),
        )
        .await
        .unwrap();
        let item = snapshot
            .items
            .iter()
            .find(|item| item.key == "deepseek")
            .expect("DSH 条目必须存在");
        assert!(!item.providers.is_empty());
        assert!(
            item.providers
                .iter()
                .any(|provider| provider.kind == "subscription")
        );
        assert!(
            item.providers
                .iter()
                .any(|provider| provider.kind == "balance")
        );
        let value = serde_json::to_value(item).unwrap();
        assert!(value.get("providers").is_some_and(|rows| rows.is_array()));
    }

    #[tokio::test]
    #[ignore = "读取本机 ZCode 配置与 OpenCode 额度，验证模型源行随 glm 条目下发"]
    async fn zcode_item_carries_model_source_quotas() {
        let scope = crate::ai::agent_tools::MonitoringScope::from_cards(&["glm".to_string()]);
        let snapshot = collect_scope(scope, std::collections::HashSet::from(["glm".to_string()]))
            .await
            .unwrap();
        let item = snapshot
            .items
            .iter()
            .find(|item| item.key == "glm")
            .expect("ZCode 条目必须存在");
        assert!(!item.providers.is_empty());
        assert!(
            item.providers
                .iter()
                .any(|provider| provider.kind == "subscription" && provider.status == "ok")
        );
        assert!(
            item.providers
                .iter()
                .any(|provider| provider.kind == "plan")
        );
    }
}
