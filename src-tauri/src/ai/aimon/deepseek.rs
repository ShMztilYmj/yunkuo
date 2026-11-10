//! DeepSeek Harness 自有日志的数据源，不能因所用模型或项目名归入其他工具。
use super::activity::SessionFacts;
use super::*;
use std::path::PathBuf;
use std::{
    collections::HashMap,
    io::{BufRead, Read},
    path::Path,
    time::SystemTime,
};

#[derive(Default)]
pub(super) struct Cache {
    timezone: String,
    mode: ScanMode,
    files: HashMap<PathBuf, FileScan>,
}

struct FileScan {
    size: u64,
    modified: Option<SystemTime>,
    days: std::collections::BTreeMap<i64, [u64; 5]>,
    facts: SessionFacts,
    last_ts: i64,
    incomplete: bool,
}

fn timestamp(value: &serde_json::Value) -> i64 {
    value
        .as_i64()
        .map(|time| {
            if time > 100_000_000_000 {
                time / 1000
            } else {
                time
            }
        })
        .or_else(|| value.as_str().map(rfc3339_to_unix))
        .unwrap_or(0)
}

fn scan_file(path: &Path, mode: ScanMode) -> std::io::Result<FileScan> {
    const MAX_BYTES: u64 = 256 * 1024 * 1024;
    const MAX_LINE: u64 = 16 * 1024 * 1024;
    let meta = std::fs::metadata(path)?;
    let mut scan = FileScan {
        size: meta.len(),
        modified: meta.modified().ok(),
        days: Default::default(),
        facts: Default::default(),
        last_ts: 0,
        incomplete: false,
    };
    if meta.len() > 64 * 1024 * 1024 {
        scan.incomplete = true;
        return Ok(scan);
    }
    let file = std::fs::File::open(path)?;
    let input: Box<dyn Read> = if path.extension().is_some_and(|ext| ext == "zstd") {
        // 默认解码全部拼接帧；第一帧只有头信息，不能在其后停止。
        Box::new(zstd::stream::read::Decoder::new(file)?)
    } else {
        Box::new(file)
    };
    let mut reader = std::io::BufReader::with_capacity(256 * 1024, input.take(MAX_BYTES));
    let mut line = String::new();
    let mut bytes = 0;
    let mut header = false;
    let mut seeded = false;
    let mut messages: HashMap<String, (i64, [u64; 5])> = HashMap::new();
    loop {
        line.clear();
        let count = match reader.by_ref().take(MAX_LINE + 1).read_line(&mut line) {
            Ok(0) => break,
            Ok(count) => count as u64,
            Err(_) => {
                scan.incomplete = true;
                break;
            }
        };
        bytes += count;
        if count > MAX_LINE || !line.ends_with('\n') {
            scan.incomplete = true;
            break;
        }
        let value: serde_json::Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => {
                scan.incomplete = true;
                continue;
            }
        };
        let kind = value["type"].as_str().unwrap_or_default();
        if !header {
            if kind != "session" || value["version"].as_u64() != Some(4) {
                scan.incomplete = true;
                break;
            }
            header = true;
            seeded = value["isSeeded"].as_bool() == Some(true);
            scan.facts.deepseek(&value);
            continue;
        }
        if kind == "session/end-seed" {
            seeded = false;
            continue;
        }
        if seeded {
            continue;
        }
        scan.facts.deepseek(&value);
        if !mode.counts() || kind != "assistant/message" {
            continue;
        }
        let Some(usage) = value
            .pointer("/data/usage")
            .filter(|usage| usage.is_object())
        else {
            continue;
        };
        let n = |key: &str| usage[key].as_u64().unwrap_or(0);
        let mut amounts = [
            n("inputTokens"),
            n("outputTokens"),
            n("cacheReadTokens"),
            n("cacheWriteTokens"),
            0,
        ];
        let sum = amounts[..4]
            .iter()
            .fold(0u64, |sum, n| sum.saturating_add(*n));
        amounts[4] = usage["totalTokens"].as_u64().unwrap_or(sum);
        let ts = timestamp(&value["time"]);
        if ts <= 0 {
            scan.incomplete = true;
            continue;
        }
        let mut day = local_day(ts);
        let id = value
            .pointer("/data/message/id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.is_empty())
            .map(|id| format!("message:{id}"))
            .or_else(|| value["seq"].as_u64().map(|seq| format!("seq:{seq}")));
        let Some(id) = id else {
            scan.incomplete = true;
            continue;
        };
        let prior = messages.entry(id).or_insert((day, [0; 5]));
        day = prior.0;
        for (amount, previous) in amounts.iter_mut().zip(prior.1.iter_mut()) {
            let current = *amount;
            *amount = current.saturating_sub(*previous);
            *previous = current.max(*previous);
        }
        if amounts.iter().any(|amount| *amount > 0) {
            let daily = scan.days.entry(day).or_default();
            for (total, amount) in daily.iter_mut().zip(amounts) {
                *total = total.saturating_add(amount);
            }
            scan.last_ts = scan.last_ts.max(ts);
        }
    }
    if !header || bytes >= MAX_BYTES {
        scan.incomplete = true;
    }
    Ok(scan)
}

pub(super) async fn collect_at(root: PathBuf, cache: &mut Cache) -> WindowAgg {
    let mut agg = WindowAgg::default();
    let mut seen = std::collections::HashSet::new();
    let mut dirs = vec![root];
    while let Some(dir) = dirs.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(entries) => entries,
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    agg.incomplete_files += 1;
                }
                continue;
            }
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let Ok(kind) = entry.file_type().await else {
                agg.incomplete_files += 1;
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                dirs.push(path);
                continue;
            }
            if !kind.is_file()
                || !matches!(
                    path.file_name().and_then(|name| name.to_str()),
                    Some("session.v4.jsonl.zstd" | "session.v4.jsonl")
                )
            {
                continue;
            }
            seen.insert(path.clone());
            let Ok(meta) = entry.metadata().await else {
                cache.files.remove(&path);
                agg.incomplete_files += 1;
                continue;
            };
            if cache.files.get(&path).is_some_and(|scan| {
                scan.size == meta.len() && scan.modified == meta.modified().ok()
            }) {
                continue;
            }
            let input = path.clone();
            let mode = cache.mode;
            match tokio::task::spawn_blocking(move || scan_file(&input, mode)).await {
                Ok(Ok(scan)) => {
                    cache.files.insert(path, scan);
                }
                _ => {
                    cache.files.remove(&path);
                    agg.incomplete_files += 1;
                }
            }
        }
    }
    cache.files.retain(|path, _| seen.contains(path));
    let (today, week, _) = window_bounds();
    let (last7, last30) = (lookback_start(7), lookback_start(30));
    for (path, scan) in &cache.files {
        if scan.incomplete {
            agg.incomplete_files += 1;
        }
        let mut active = [false; 5];
        for (day, amounts) in &scan.days {
            for (index, cutoff) in [i64::MIN, today, week, last7, last30]
                .into_iter()
                .enumerate()
            {
                if *day >= cutoff {
                    let target = match index {
                        0 => &mut agg.all,
                        1 => &mut agg.today,
                        2 => &mut agg.week,
                        3 => &mut agg.last7,
                        _ => &mut agg.last30,
                    };
                    add_codex_usage(target, amounts);
                    active[index] = true;
                }
            }
            let daily = agg.daily.entry(*day).or_default();
            add_codex_usage(daily, amounts);
            daily.sessions += 1;
        }
        for (index, present) in active.into_iter().enumerate() {
            let target = match index {
                0 => &mut agg.all,
                1 => &mut agg.today,
                2 => &mut agg.week,
                3 => &mut agg.last7,
                _ => &mut agg.last30,
            };
            target.sessions += u64::from(present);
        }
        agg.last_ts = agg.last_ts.max(scan.last_ts);
        if let Some(session) = scan
            .facts
            .snapshot("deepseek", path, agg_session_total(&scan.days))
        {
            agg.last_ts = agg.last_ts.max(session.last_activity_at);
            agg.sessions.push(session);
        }
    }
    agg
}

pub(super) async fn collect(mode: ScanMode) -> WindowAgg {
    static CACHE: once_cell::sync::Lazy<tokio::sync::Mutex<Cache>> =
        once_cell::sync::Lazy::new(Default::default);
    let timezone = crate::platform::system_time::timezone_key().unwrap_or_default();
    let mut cache = CACHE.lock().await;
    if cache.timezone != timezone || cache.mode != mode {
        cache.files.clear();
        cache.timezone = timezone;
        cache.mode = mode;
    }
    collect_at(home().join(".dsh/sessions"), &mut cache).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn activity_mode_preserves_completion_without_counting_compressed_usage() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("session.v4.jsonl.zstd");
        let mut events = fixture_events(false);
        let now = chrono::Utc::now().timestamp_millis() - 10000;
        for (i, event) in events.iter_mut().enumerate().skip(1) {
            event["time"] = (now + i as i64).into();
        }
        write_frames(&path, &events);
        let scan = scan_file(&path, ScanMode::Activity).unwrap();
        assert!(scan.days.is_empty());
        assert!(!scan.incomplete);
        assert_eq!(
            scan.facts.snapshot("deepseek", &path, 0).unwrap().state,
            "completed"
        );
        let scan = scan_file(&path, ScanMode::Usage).unwrap();
        assert_eq!(agg_session_total(&scan.days), 170);
    }
    fn fixture_events(seeded: bool) -> Vec<serde_json::Value> {
        let now = chrono::Utc::now().timestamp_millis();
        vec![
            serde_json::json!({"type":"session","version":4,"id":"fixture","cwd":"C:/same-project","isSeeded":seeded,"delegationDepth":0}),
            serde_json::json!({"type":"turn/start","seq":1,"time":now,"data":{"turn":"t"}}),
            serde_json::json!({"type":"assistant/message","seq":2,"time":now+1000,"data":{"message":{"id":"answer","role":"assistant","source":{"kind":"model","provider":"fixture","model":"claude-fixture"}},"usage":{"inputTokens":100,"outputTokens":20,"cacheReadTokens":50,"cacheWriteTokens":0,"totalTokens":170},"stream":[{"chunk":{"usage":{"totalTokens":170}}}]}}),
            serde_json::json!({"type":"turn/end","seq":3,"time":now+2000,"data":{"reason":{"kind":"completed"}}}),
        ]
    }
    fn write_frames(path: &std::path::Path, events: &[serde_json::Value]) {
        use std::io::Write;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = std::fs::File::create(path).unwrap();
        // DSH 是多个 Zstd 帧拼接，不能只解第一帧（第一帧仅含 session 头）。
        for event in events {
            let data = format!("{event}\n");
            file.write_all(&zstd::stream::encode_all(data.as_bytes(), 1).unwrap())
                .unwrap();
        }
    }
    #[tokio::test]
    async fn separate_harness_source_reads_all_compressed_frames() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("project/session/session.v4.jsonl.zstd");
        write_frames(&path, &fixture_events(false));
        let mut cache = Cache::default();
        let agg = collect_at(fixture.path().to_owned(), &mut cache).await;
        assert_eq!(agg.all.total, 170, "DeepSeek Harness 的独立日志不能被遗漏");
        assert_eq!(
            (agg.all.input, agg.all.output, agg.all.cached),
            (100, 20, 50)
        );
        assert_eq!(agg.all.sessions, 1);
        assert_eq!(
            agg.sessions[0].tool, "deepseek",
            "使用 Claude 模型也仍属于 DSH 工具"
        );
        assert_eq!(agg.sessions[0].state, "completed");
        assert_eq!(agg.sessions[0].model.as_deref(), Some("claude-fixture"));
        let mut claude_cache = ClaudeCache::default();
        let claude =
            collect_claude_at(vec![fixture.path().join("claude")], &mut claude_cache).await;
        assert_eq!(claude.all.total, 0);
    }
    #[tokio::test]
    async fn repeated_records_and_warm_scans_do_not_double_usage_and_deleted_files_disappear() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("project/session/session.v4.jsonl.zstd");
        let mut events = fixture_events(false);
        events.insert(3, events[2].clone());
        write_frames(&path, &events);
        let mut cache = Cache::default();
        assert_eq!(
            collect_at(fixture.path().to_owned(), &mut cache)
                .await
                .all
                .total,
            170
        );
        assert_eq!(
            collect_at(fixture.path().to_owned(), &mut cache)
                .await
                .all
                .total,
            170
        );
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            collect_at(fixture.path().to_owned(), &mut cache)
                .await
                .all
                .total,
            0
        );
        assert!(cache.files.is_empty());
    }
    #[tokio::test]
    async fn inherited_seed_is_excluded_and_child_usage_does_not_create_main_session() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("project/session/session.v4.jsonl.zstd");
        let mut events = fixture_events(true);
        events.push(serde_json::json!({"type":"session/end-seed","seq":4,"time":chrono::Utc::now().timestamp_millis(),"data":{}}));
        let mut fresh = fixture_events(false);
        fresh.remove(0);
        events.extend(fresh);
        write_frames(&path, &events);
        let mut cache = Cache::default();
        let agg = collect_at(fixture.path().to_owned(), &mut cache).await;
        assert_eq!(agg.all.total, 170);
        assert_eq!(agg.sessions.len(), 1);
        let mut child = fixture_events(false);
        child[0]["parentSession"] = serde_json::json!({"id":"parent"});
        write_frames(&path, &child);
        let agg = collect_at(fixture.path().to_owned(), &mut cache).await;
        assert_eq!(agg.all.total, 170);
        assert!(agg.sessions.is_empty());
    }
    #[tokio::test]
    async fn damaged_tail_keeps_verified_usage_and_marks_partial() {
        use std::io::Write;
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("project/session/session.v4.jsonl.zstd");
        write_frames(&path, &fixture_events(false));
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(&[0xde, 0xad, 0xbe, 0xef])
            .unwrap();
        let agg = collect_at(fixture.path().to_owned(), &mut Cache::default()).await;
        assert_eq!(agg.all.total, 170);
        assert_eq!(agg.incomplete_files, 1);
    }
    #[tokio::test]
    #[ignore = "诊断本机 DSH 会话，只输出计数和来源，不读取账号凭据"]
    async fn local_source_counts_only() {
        let start = std::time::Instant::now();
        let mut cache = Cache::default();
        let agg = collect_at(home().join(".dsh/sessions"), &mut cache).await;
        println!(
            "DSH_SOURCE_COUNTS files={} total={} today={} sessions={} main_sessions={} partial_files={} elapsed_ms={}",
            cache.files.len(),
            agg.all.total,
            agg.today.total,
            agg.all.sessions,
            agg.sessions.len(),
            agg.incomplete_files,
            start.elapsed().as_millis()
        );
        assert!(agg.all.total > 0);
        assert!(
            agg.sessions
                .iter()
                .all(|session| session.tool == "deepseek" && session.id.starts_with("deepseek:"))
        );
        let warm = std::time::Instant::now();
        let second = collect_at(home().join(".dsh/sessions"), &mut cache).await;
        println!(
            "DSH_WARM_COUNTS total={} elapsed_ms={}",
            second.all.total,
            warm.elapsed().as_millis()
        );
    }
}
