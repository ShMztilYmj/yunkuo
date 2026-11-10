// AI 用量监控（参照 Javis603/token-monitor 思路，本地优先）：
// - 只读本地会话/账号元数据；逐行解析后仅缓存计数、状态及展示元数据，不保留提示词或正文
// - Claude Code：~/.claude/projects/**/*.jsonl（assistant usage，按 message.id 去重流式累计）
// - DeepSeek Harness：~/.dsh/sessions/**/session.v4.jsonl.zstd（独立来源，拼接帧、消息去重、剔除种子历史）
// - Codex：~/.codex/sessions/**/*.jsonl（token_count 事件 total_token_usage 为会话累计 →
//   按相邻事件差值累加，按明确所有者与本地 ordinal 边界剔除继承历史；~/.codex/auth.json 提供账号与套餐）
// - Codex 快照交由 account_store：DPAPI 当前用户加密、原子激活与加密恢复副本。
// - OpenCode / ZCode：只读 SQLite；Kimi Code：wire；Grok Build：每轮 usage
// - 九款 Token 统一由随附的固定版本 tokscale 引擎读取；本地 lifecycle 独立提供状态
//
// 职责拆分：model（数据结构）、scan（会话扫描与聚合）、quota（额度窗口与文案）、
// commands（Tauri 命令）。子模块通过本模块的 re-export 互相引用，测试也走同一份名字。
mod commands;
mod model;
mod quota;
mod scan;

pub(crate) use commands::*;
pub(crate) use model::*;
pub(crate) use scan::*;

pub(crate) mod activity;
mod deepseek;
pub(crate) mod engine;
mod providers;
mod sources;

/// 本机混合代理端口。额度查询一律经它出网：直连会把真实出口 IP 交给服务商，
/// 与账号所在地区对不上时风险很高，所以这里失败关闭——端口不通就报错保留旧读数。
static PROXY_PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);

pub(crate) fn set_proxy_port(port: u16) {
    PROXY_PORT.store(port, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn proxy_port() -> Option<u16> {
    match PROXY_PORT.load(std::sync::atomic::Ordering::Relaxed) {
        0 => None,
        port => Some(port),
    }
}

/// 交给子进程的代理环境变量：大小写各写一遍（不同 HTTP 库认的不一样），
/// `NO_PROXY` 排除回环，避免代理自身或本地探测被再次代理。
pub(crate) fn proxy_env(port: u16) -> Vec<(&'static str, String)> {
    let url = format!("http://127.0.0.1:{port}");
    let bypass = "localhost,127.0.0.1,::1".to_string();
    [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ]
    .into_iter()
    .map(|name| (name, url.clone()))
    .chain([("NO_PROXY", bypass.clone()), ("no_proxy", bypass)])
    .collect()
}
#[cfg(test)]
mod tests {
    use super::quota::*;
    use super::*;

    #[test]
    fn quota_queries_are_forced_through_the_local_proxy() {
        let env = proxy_env(7897);
        for name in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            assert!(
                env.iter()
                    .any(|(key, value)| *key == name && value == "http://127.0.0.1:7897"),
                "{name} 未指向本机代理"
            );
        }
        // 回环必须绕过，否则代理自身的探测会被再代理一次。
        assert!(
            env.iter()
                .any(|(key, value)| *key == "NO_PROXY" && value.contains("127.0.0.1"))
        );
        set_proxy_port(7897);
        assert_eq!(proxy_port(), Some(7897));
        set_proxy_port(0);
        assert_eq!(proxy_port(), None);
    }

    #[tokio::test]
    async fn native_activity_mode_keeps_lifecycle_and_releases_fallback_token_state() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("session.jsonl");
        let now = chrono::Utc::now().to_rfc3339();
        std::fs::write(&path,format!("{}\n",serde_json::json!({"type":"assistant","sessionId":"fixture","timestamp":now,"message":{"id":"m","stop_reason":"end_turn","usage":{"input_tokens":100,"output_tokens":20}}}))).unwrap();
        let full = scan_claude_file(&path, None, ScanMode::Usage)
            .await
            .unwrap();
        assert_eq!(agg_session_total(&full.days), 120);
        let activity = scan_claude_file(&path, Some(full), ScanMode::Activity)
            .await
            .unwrap();
        assert!(activity.days.is_empty());
        assert!(activity.messages.is_empty());
        assert_eq!(
            activity
                .activity
                .snapshot("claude", &path, 0)
                .unwrap()
                .state,
            "completed"
        );
        let restored = scan_claude_file(&path, Some(activity), ScanMode::Usage)
            .await
            .unwrap();
        assert_eq!(agg_session_total(&restored.days), 120);

        let event = |value: serde_json::Value| value.to_string() + "\n";
        std::fs::write(&path,event(serde_json::json!({"type":"session_meta","timestamp":now,"payload":{"id":"c"}}))+&event(serde_json::json!({"type":"event_msg","timestamp":now,"payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"output_tokens":20,"total_tokens":120}},"rate_limits":{"primary":{"used_percent":20}}}}))+&event(serde_json::json!({"type":"event_msg","timestamp":now,"payload":{"type":"task_complete"}}))).unwrap();
        let full = codex_scan_file(&path, None, ScanMode::Usage).await.unwrap();
        assert_eq!(full.events.len(), 1);
        let activity = codex_scan_file(&path, Some(full), ScanMode::Activity)
            .await
            .unwrap();
        assert!(activity.events.is_empty());
        assert!(activity.records.is_empty());
        assert_eq!(activity.previous, [0; 5]);
        assert!(activity.rate.is_some());
        assert_eq!(
            activity
                .activity
                .snapshot("chatgpt", &path, 0)
                .unwrap()
                .state,
            "completed"
        );
        let restored = codex_scan_file(&path, Some(activity), ScanMode::Usage)
            .await
            .unwrap();
        assert_eq!(restored.events.len(), 1);
    }

    fn claude_event(total: u64) -> String {
        format!(
            "{}\n",
            serde_json::json!({"type":"assistant","timestamp":"2026-10-17T12:00:00Z", "message":{"usage":{"input_tokens":total,"output_tokens":0}}})
        )
    }
    #[tokio::test]
    async fn claude_cache_handles_append_partial_line_replacement_and_deletion() {
        use std::io::Write;
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("session.jsonl");
        std::fs::write(&path, claude_event(10)).unwrap();
        let mut cache = ClaudeCache::default();
        let dirs = vec![fixture.path().to_owned()];
        assert_eq!(
            collect_claude_at(dirs.clone(), &mut cache).await.all.total,
            10
        );
        assert_eq!(
            collect_claude_at(dirs.clone(), &mut cache).await.all.total,
            10
        );
        assert_eq!(cache.files[&path].read_bytes, 0);
        let event = claude_event(20);
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(event[..event.len() - 1].as_bytes()).unwrap();
        file.flush().unwrap();
        assert_eq!(
            collect_claude_at(dirs.clone(), &mut cache).await.all.total,
            10
        );
        file.write_all(b"\n").unwrap();
        drop(file);
        assert_eq!(
            collect_claude_at(dirs.clone(), &mut cache).await.all.total,
            30
        );
        std::fs::write(&path, claude_event(7) + &" ".repeat(1000) + "\n").unwrap();
        assert_eq!(
            collect_claude_at(dirs.clone(), &mut cache).await.all.total,
            7
        );
        std::fs::remove_file(&path).unwrap();
        assert_eq!(collect_claude_at(dirs, &mut cache).await.all.total, 0);
        assert!(cache.files.is_empty());
    }
    #[tokio::test]
    async fn claude_100mb_unchanged_scan_reads_zero_content() {
        use std::io::Write;
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("large.jsonl");
        let mut file = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        let line = format!(
            "{{\"type\":\"user\",\"message\":\"{}\"}}\n",
            "x".repeat(1000)
        );
        for _ in 0..102_400 {
            file.write_all(line.as_bytes()).unwrap();
        }
        file.write_all(claude_event(42).as_bytes()).unwrap();
        file.flush().unwrap();
        drop(file);
        let mut cache = ClaudeCache::default();
        let dirs = vec![fixture.path().to_owned()];
        let first = std::time::Instant::now();
        assert_eq!(
            collect_claude_at(dirs.clone(), &mut cache).await.all.total,
            42
        );
        let first_ms = first.elapsed().as_millis();
        let first_bytes = cache.files[&path].read_bytes;
        let warm = std::time::Instant::now();
        assert_eq!(collect_claude_at(dirs, &mut cache).await.all.total, 42);
        let warm_ms = warm.elapsed().as_millis();
        assert_eq!(cache.files[&path].read_bytes, 0);
        println!(
            "CLAUDE_SYNTHETIC first_bytes={first_bytes} first_ms={first_ms} warm_bytes=0 warm_ms={warm_ms}"
        );
    }

    #[tokio::test]
    async fn claude_streamed_message_is_counted_once_and_daily_sessions_are_distinct() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("session.jsonl");
        let now = chrono::Utc::now();
        let event = |output| {
            serde_json::json!({"type":"assistant", "sessionId":"fixture", "timestamp":now.to_rfc3339(),
            "message":{"id":"message-1", "usage":{"input_tokens":100,"output_tokens":output,"cache_read_input_tokens":60}}}).to_string()+"\n"
        };
        std::fs::write(&path, event(10) + &event(10) + &event(20)).unwrap();
        let mut cache = ClaudeCache::default();
        let agg = collect_claude_at(vec![fixture.path().to_owned()], &mut cache).await;
        assert_eq!(agg.all.total, 180);
        assert_eq!(agg.last7.total, 180);
        assert_eq!(agg.last30.total, 180);
        assert_eq!(agg.last7.sessions, 1);
        assert_eq!(agg.daily.len(), 1);
        assert_eq!(daily_usage(&agg)[0].usage.total, 180);
    }

    #[test]
    fn claude_official_usage_maps_percent_windows() {
        let now = rfc3339_to_unix("2026-01-01T00:00:00Z");
        let value = serde_json::json!({
            "five_hour": {"utilization": 42.0, "resets_at": "2026-01-01T02:30:00Z"},
            "seven_day": {"utilization": 10.0, "resets_at": "2026-01-05T00:00:00Z"},
            "seven_day_opus": null
        });
        let rate = claude_rate_from_usage(&value, now);
        let primary = rate.primary.as_ref().unwrap();
        assert_eq!(primary.utilization, 0.42);
        assert_eq!(primary.limit_window_seconds, 5 * 3600);
        assert_eq!(primary.reset_after_seconds, 2 * 3600 + 30 * 60);
        let secondary = rate.secondary.as_ref().unwrap();
        assert_eq!(secondary.utilization, 0.1);
        assert_eq!(secondary.limit_window_seconds, 7 * 86400);
        assert_eq!(rate.captured_at, Some(now));
        assert!(rate.account_verified);
    }

    #[test]
    fn claude_official_usage_does_not_invent_missing_windows() {
        let value = serde_json::json!({
            "five_hour": {"utilization": "bad"},
            "seven_day": null
        });
        let rate = claude_rate_from_usage(&value, 100);
        assert!(rate.primary.is_none());
        assert!(rate.secondary.is_none());
        assert_eq!(rate.captured_at, None);
    }

    #[test]
    fn quota_retains_event_timestamp_and_missing_reset_is_unknown() {
        let rate = RateSnapshot {
            captured_at: 100,
            primary: Some(serde_json::json!({"used_percent":95,"window_minutes":300})),
            secondary: None,
        };
        let result = rate_from_snapshot(&Some(rate), 1000);
        assert_eq!(result.captured_at, Some(100));
        assert!(!result.account_verified);
        assert_eq!(result.primary.as_ref().unwrap().reset_at, None);
        assert_eq!(result.primary.unwrap().limit_window_seconds, 18000);
    }

    fn token_event(timestamp: &str, input: u64, output: u64, cached: u64) -> String {
        format!(
            "{}\n",
            serde_json::json!({"type":"event_msg", "timestamp":timestamp,
            "payload":{"type":"token_count", "info":{"total_token_usage":{
                "input_tokens":input,"output_tokens":output,"cached_input_tokens":cached,"total_tokens":input+output
            }}}})
        )
    }

    #[tokio::test]
    async fn codex_first_event_counts_and_cache_is_not_added_twice() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("session.jsonl");
        let header = format!(
            "{}\n",
            serde_json::json!({"type":"session_meta","payload":{"id":"regular"}})
        );
        std::fs::write(
            &path,
            header + &token_event("2026-10-13T23:00:00Z", 100, 20, 60),
        )
        .unwrap();
        let first = codex_scan_file(&path, None, ScanMode::Usage).await.unwrap();
        assert_eq!(first.events[0].1, [100, 20, 60, 0, 120]);
        let cached = codex_scan_file(&path, Some(first), ScanMode::Usage)
            .await
            .unwrap();
        assert_eq!(cached.events.len(), 1);
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(token_event("2026-10-14T02:00:00Z", 150, 30, 90).as_bytes())
            .unwrap();
        drop(file);
        let scan = codex_scan_file(&path, Some(cached), ScanMode::Usage)
            .await
            .unwrap();
        let mut agg = WindowAgg::default();
        aggregate_codex(&scan, &mut agg, rfc3339_to_unix("2026-10-14T00:00:00Z"), 0);
        assert_eq!(agg.all.total, 180);
        assert_eq!(agg.today.total, 60);
        assert_eq!(agg.week.total, 180);
    }

    #[tokio::test]
    async fn codex_fork_excludes_inherited_history_and_counts_first_child_call() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("fork.jsonl");
        let meta = format!(
            "{}\n",
            serde_json::json!({"type":"session_meta","payload":{
                "id":"child", "forked_from_id":"parent", "timestamp":"2026-10-14T00:00:00Z"
            }})
        );
        let source = format!(
            "{}\n",
            serde_json::json!({"type":"session_meta","payload":{"id":"parent"}})
        );
        let owned = format!(
            "{}\n",
            serde_json::json!({"type":"event_msg","payload":{"type":"thread_settings_applied","thread_id":"child"}})
        );
        let data = meta
            + &source
            + &token_event("2026-10-14T00:00:01Z", 100, 20, 60)
            + &owned
            + &token_event("2026-10-14T01:00:00Z", 150, 30, 90);
        std::fs::write(&path, data).unwrap();
        let scan = codex_scan_file(&path, None, ScanMode::Usage).await.unwrap();
        assert_eq!(scan.events.len(), 1);
        assert_eq!(scan.events[0].1, [50, 10, 30, 0, 60]);
    }

    #[tokio::test]
    async fn legacy_fork_without_proven_boundary_is_incomplete_not_guessed() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("legacy.jsonl");
        let header = format!(
            "{}\n",
            serde_json::json!({"type":"session_meta","payload":{"id":"child","forked_from_id":"parent"}})
        );
        std::fs::write(
            &path,
            header + &token_event("2026-10-14T00:00:01Z", 100, 20, 60),
        )
        .unwrap();
        let scan = codex_scan_file(&path, None, ScanMode::Usage).await.unwrap();
        let mut agg = WindowAgg::default();
        aggregate_codex(&scan, &mut agg, 0, 0);
        assert_eq!(agg.all.total, 0);
        assert_eq!(agg.incomplete_files, 1);
    }

    #[tokio::test]
    async fn structured_usage_filters_thread_deduplicates_response_and_ignores_snapshots() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("structured.jsonl");
        let header = format!(
            "{}\n",
            serde_json::json!({"type":"session_meta","payload":{"id":"child","forked_from_id":"parent"}})
        );
        let record = |owner: &str, response: &str| {
            serde_json::json!({"type":"token_usage_record", "timestamp":"2026-10-14T00:00:01Z",
            "payload":{"thread_id":owner,"session_id":"shared-parent-session","response_id":response,
                "usage":{"input_tokens":50,"output_tokens":10,"cached_input_tokens":30,"reasoning_output_tokens":5,"total_tokens":60},
                "thread_token_usage":{"total_tokens":180}}})
        };
        let child = record("child", "response-1");
        let data = format!(
            "{header}{}\n{child}\n{child}\n{}\n{}",
            record("parent", "inherited"),
            serde_json::json!({"type":"compacted","payload":{"latest_token_usage_record":child["payload"]}}),
            token_event("2026-10-14T00:00:01Z", 150, 30, 90)
        );
        std::fs::write(&path, data).unwrap();
        let scan = codex_scan_file(&path, None, ScanMode::Usage).await.unwrap();
        let mut agg = WindowAgg::default();
        aggregate_codex(&scan, &mut agg, 0, 0);
        assert_eq!(agg.all.total, 60);
        assert_eq!(agg.all.cached, 30);
        assert_eq!(scan.records.len(), 1);
    }

    #[tokio::test]
    async fn local_ordinal_boundary_excludes_inherited_prefix() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("ordinal.jsonl");
        let header = format!(
            "{}\n",
            serde_json::json!({"type":"session_meta","payload":{"id":"child","parent_thread_id":"parent","subagent_history_start_ordinal":10}})
        );
        let event = |ordinal, input, output| {
            let mut value: serde_json::Value =
                serde_json::from_str(&token_event("2026-10-14T00:00:01Z", input, output, 0))
                    .unwrap();
            value["ordinal"] = serde_json::json!(ordinal);
            format!("{value}\n")
        };
        std::fs::write(&path, header + &event(9, 100, 20) + &event(10, 150, 30)).unwrap();
        let scan = codex_scan_file(&path, None, ScanMode::Usage).await.unwrap();
        assert_eq!(scan.events.len(), 1);
        assert_eq!(scan.events[0].1[4], 60);
    }

    /// 本机冒烟测试：对真实 ~/.claude、~/.codex 跑一次采集，人工核对输出
    #[tokio::test]
    #[ignore = "需要用户明确选择访问真实账号数据"]
    async fn scan_real_home_data() {
        let claude = collect_claude(ScanMode::Usage).await;
        println!("CLAUDE all    = {:?}", claude.all);
        println!("CLAUDE today  = {:?}", claude.today);
        println!("CLAUDE last_ts= {}", claude.last_ts);

        let (codex, rate_snap) = collect_codex(ScanMode::Usage).await;
        println!("CODEX all     = {:?}", codex.all);
        println!("CODEX today   = {:?}", codex.today);
        println!("CODEX last_ts = {}", codex.last_ts);
        let rate = rate_from_snapshot(&rate_snap, chrono::Utc::now().timestamp());
        println!(
            "CODEX rate    = primary {:?} / secondary {:?}",
            rate.primary
                .as_ref()
                .map(|w| (w.utilization, w.reset_after_seconds)),
            rate.secondary
                .as_ref()
                .map(|w| (w.utilization, w.reset_after_seconds))
        );

        let acc = read_claude_account().await;
        println!(
            "CLAUDE account = {} / {:?}",
            acc.email.clone().unwrap_or_default(),
            acc.plan
        );

        let cacc = read_codex_account().await;
        println!(
            "CODEX account = {} / {:?}",
            cacc.email.clone().unwrap_or_default(),
            cacc.plan
        );

        let snaps = crate::ai::identity::account_store::Store::new(home().join(".codex"))
            .list()
            .unwrap();
        println!(
            "CODEX encrypted snapshot count = {}",
            snaps.iter().filter(|s| !s.legacy).count()
        );

        assert!(true, "smoke run completed");
    }
}
