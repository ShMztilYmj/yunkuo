//! 各工具只读适配器。数据目录决定工具身份，模型只作为会话元数据。
//! OpenCode / ZCode: SQLite 的计数和生命周期列；Kimi: wire；Grok: usage.json。
use super::activity::AiSession;
use super::{
    AiUsageItem, NativePlan, ScanMode, WindowAgg, add_codex_usage, home, local_day, lookback_start,
    window_bounds,
};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::{BufRead, Read},
    path::{Path, PathBuf},
};

#[derive(Clone, Default)]
pub(super) struct Source {
    pub agg: WindowAgg,
    pub available: bool,
    pub detail: String,
    pub input_includes_cache: bool,
}
#[derive(Default)]
struct Record {
    project: String,
    model: Option<String>,
    state: String,
    last: i64,
    completed: Option<i64>,
    child: bool,
}
#[derive(Default)]
struct Ledger {
    records: HashMap<String, Record>,
    // 同一模型消息的重复快照、流式累计和 fork 复制只计一次。
    tokens: HashMap<String, (String, i64, [u64; 5])>,
    incomplete: bool,
    available: bool,
}
fn text(value: &Value, key: &str) -> String {
    value[key]
        .as_str()
        .unwrap_or_default()
        .chars()
        .take(160)
        .collect()
}
fn folder(value: &str) -> String {
    value
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .chars()
        .take(80)
        .collect()
}
fn ts(value: &Value) -> i64 {
    value
        .as_f64()
        .filter(|n| n.is_finite() && *n > 0.)
        .map(|n| {
            if n > 100_000_000_000. {
                (n / 1000.) as i64
            } else {
                n as i64
            }
        })
        .or_else(|| value.as_str().map(super::rfc3339_to_unix))
        .unwrap_or(0)
}
impl Ledger {
    fn event(&mut self, id: &str, time: i64, state: &str) {
        if time <= 0 || time > chrono::Utc::now().timestamp() + 60 {
            return;
        }
        let record = self.records.entry(id.into()).or_default();
        if time < record.last {
            return;
        }
        record.last = time;
        if !state.is_empty() {
            record.state = state.into();
            record.completed = (state == "completed").then_some(time);
        }
    }
    fn usage(&mut self, id: &str, message: String, time: i64, amounts: [u64; 5]) {
        if time <= 0 || time > chrono::Utc::now().timestamp() + 60 {
            self.incomplete = true;
            return;
        }
        self.available = true;
        let entry = self
            .tokens
            .entry(message)
            .or_insert_with(|| (id.into(), time, [0; 5]));
        for (old, amount) in entry.2.iter_mut().zip(amounts) {
            *old = (*old).max(amount);
        }
    }
    fn finish(self, key: &str) -> Source {
        let mut agg = WindowAgg::default();
        agg.incomplete_files = u64::from(self.incomplete);
        let (today, week, _) = window_bounds();
        let cutoffs = [i64::MIN, today, week, lookback_start(7), lookback_start(30)];
        let mut sessions: [HashSet<String>; 5] = Default::default();
        let mut daily_sessions: BTreeMap<i64, HashSet<String>> = Default::default();
        let mut session_totals: HashMap<String, u64> = HashMap::new();
        for (id, timestamp, amounts) in self.tokens.values() {
            let day = local_day(*timestamp);
            agg.last_ts = agg.last_ts.max(*timestamp);
            let total = session_totals.entry(id.clone()).or_default();
            *total = total.saturating_add(amounts[4]);
            for (i, cutoff) in cutoffs.into_iter().enumerate() {
                if day < cutoff {
                    continue;
                }
                let target = match i {
                    0 => &mut agg.all,
                    1 => &mut agg.today,
                    2 => &mut agg.week,
                    3 => &mut agg.last7,
                    _ => &mut agg.last30,
                };
                add_codex_usage(target, amounts);
                sessions[i].insert(id.clone());
            }
            add_codex_usage(agg.daily.entry(day).or_default(), amounts);
            daily_sessions.entry(day).or_default().insert(id.clone());
        }
        for (i, ids) in sessions.into_iter().enumerate() {
            let target = match i {
                0 => &mut agg.all,
                1 => &mut agg.today,
                2 => &mut agg.week,
                3 => &mut agg.last7,
                _ => &mut agg.last30,
            };
            target.sessions = ids.len() as u64;
        }
        for (day, ids) in daily_sessions {
            if let Some(total) = agg.daily.get_mut(&day) {
                total.sessions = ids.len() as u64;
            }
        }
        for (id, record) in self.records {
            if record.child || record.last <= 0 {
                continue;
            }
            agg.last_ts = agg.last_ts.max(record.last);
            let state = if record.state.is_empty()
                || matches!(record.state.as_str(), "running" | "asking")
                    && chrono::Utc::now().timestamp() - record.last >= 1800
            {
                "unknown".into()
            } else {
                record.state
            };
            agg.sessions.push(AiSession {
                id: format!("{key}:{id}"),
                tool: key.into(),
                project: if record.project.is_empty() {
                    "本地会话".into()
                } else {
                    record.project
                },
                model: record.model,
                state,
                last_activity_at: record.last,
                completed_at: record.completed,
                total: session_totals.get(&id).copied().unwrap_or(0),
                usage_available: Some(session_totals.contains_key(&id)),
            });
        }
        Source {
            agg,
            available: self.available,
            ..Default::default()
        }
    }
}

fn database_mode(path: &Path, key: &str, mode: ScanMode) -> Source {
    if !path.is_file() {
        return Source {
            detail: "未发现本地会话数据库".into(),
            ..Default::default()
        };
    }
    let mut ledger = Ledger::default();
    let result = (|| -> rusqlite::Result<()> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(std::time::Duration::from_millis(150))?;
        connection.execute_batch("PRAGMA query_only=ON; BEGIN DEFERRED;")?;
        let mut query = connection.prepare("SELECT id, directory, parent_id FROM session")?;
        for row in query.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })? {
            let (id, directory, parent) = row?;
            ledger.records.insert(
                id,
                Record {
                    project: folder(&directory),
                    child: parent.is_some(),
                    ..Default::default()
                },
            );
        }
        let zcode_usage = key == "glm"
            && connection.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='model_usage'",
                [],
                |r| r.get::<_, i64>(0),
            )? > 0
            && connection.query_row("SELECT EXISTS(SELECT 1 FROM model_usage)", [], |r| {
                r.get::<_, bool>(0)
            })?;
        // 只从消息 JSON 提取元数据和计数，不向缓存/前端传递正文。
        let sql = "SELECT id, session_id, time_created, json_object('role',json_extract(data,'$.role'),'model',coalesce(json_extract(data,'$.modelID'),json_extract(data,'$.modelId')),'created',json_extract(data,'$.time.created'),'completed',json_extract(data,'$.time.completed'),'finish',json_extract(data,'$.finish'),'tokens',json_extract(data,'$.tokens'),'error',json_type(data,'$.error')) FROM message ORDER BY time_created, id LIMIT 250001";
        let sql = if mode.counts() {
            sql.to_owned()
        } else {
            sql.replace("'tokens',json_extract(data,'$.tokens'),", "")
        };
        let mut query = connection.prepare(&sql)?;
        let mut count = 0;
        for row in query.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })? {
            let (message, id, created, data) = row?;
            count += 1;
            if count > 250000 {
                ledger.incomplete = true;
                break;
            }
            let value: Value = serde_json::from_str(&data)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            let start = ts(&value["created"]).max(ts(&created.into()));
            let end = ts(&value["completed"]);
            let role = value["role"].as_str().unwrap_or_default();
            if let Some(model) = value["model"].as_str() {
                ledger.records.entry(id.clone()).or_default().model =
                    Some(model.chars().take(160).collect());
            }
            if role == "user" {
                ledger.event(&id, start, "running");
                continue;
            }
            if role != "assistant" {
                continue;
            }
            let finish = value["finish"].as_str().unwrap_or_default();
            let state = if value["error"].as_str().is_some_and(|s| s != "null") {
                "ended"
            } else if matches!(finish, "stop" | "end_turn" | "completed") {
                "completed"
            } else if matches!(finish, "length" | "content-filter" | "error") {
                "ended"
            } else {
                "running"
            };
            ledger.event(&id, end.max(start), state);
            if mode.counts() && !zcode_usage && value["tokens"].is_object() {
                let usage = &value["tokens"];
                if usage["input"].as_u64().is_none() || usage["output"].as_u64().is_none() {
                    ledger.incomplete = true;
                    continue;
                }
                let n = |name: &str| usage[name].as_u64().unwrap_or(0);
                let cached = usage
                    .pointer("/cache/read")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let write = usage
                    .pointer("/cache/write")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let input = n("input");
                let output =
                    n("output").saturating_add(if key == "opencode" { n("reasoning") } else { 0 });
                // OpenCode 分开存放 visibleOutput 和 reasoning，统一为输出；总量优先保留 provider 值。
                let total = usage["total"].as_u64().unwrap_or_else(|| {
                    if key == "glm" {
                        input.saturating_add(output)
                    } else {
                        input
                            .saturating_add(output)
                            .saturating_add(cached)
                            .saturating_add(write)
                    }
                });
                ledger.usage(&id, message, start, [input, output, cached, write, total]);
            }
        }
        if zcode_usage {
            if mode.counts() {
                let mut query = connection.prepare("SELECT id, session_id, coalesce(completed_at,started_at), input_tokens, output_tokens, cache_read_input_tokens, cache_creation_input_tokens, coalesce(computed_total_tokens,provider_total_tokens,input_tokens+output_tokens), model_id FROM model_usage ORDER BY started_at LIMIT 250001")?;
                let mut rows = query.query([])?;
                let mut count = 0;
                while let Some(row) = rows.next()? {
                    count += 1;
                    if count > 250000 {
                        ledger.incomplete = true;
                        break;
                    }
                    let message: String = row.get(0)?;
                    let id: String = row.get(1)?;
                    let time: i64 = row.get(2)?;
                    let mut amounts = [0; 5];
                    for (n, column) in [3, 4, 5, 6, 7].into_iter().enumerate() {
                        amounts[n] = row.get::<_, Option<i64>>(column)?.unwrap_or(0).max(0) as u64;
                    }
                    ledger.usage(&id, message, ts(&time.into()), amounts);
                    if let Some(model) = row.get::<_, Option<String>>(8)? {
                        ledger.records.entry(id).or_default().model =
                            Some(model.chars().take(160).collect());
                    }
                }
            }
            // turn_usage 是模型循环的最终事实，不将一次工具调用或回复片段当完成。
            if connection.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='turn_usage'",
                [],
                |r| r.get::<_, i64>(0),
            )? > 0
            {
                let mut query = connection.prepare("SELECT session_id, status, started_at, completed_at FROM turn_usage ORDER BY started_at")?;
                for row in query.query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                    ))
                })? {
                    let (id, status, start, end) = row?;
                    let state = match status.as_str() {
                        "completed" => "completed",
                        "running" | "pending" => "running",
                        "cancelled" | "error" => "ended",
                        _ => "unknown",
                    };
                    ledger.event(&id, ts(&end.unwrap_or(start).into()), state);
                }
            }
        }
        connection.execute_batch("ROLLBACK;")?;
        Ok(())
    })();
    if result.is_err() {
        ledger.incomplete = true;
    }
    let mut source = ledger.finish(key);
    source.input_includes_cache = key == "glm";
    source.detail = if result.is_err() {
        "数据库读取不完整，仅显示已核验部分；可稍后刷新"
    } else {
        "只读本地会话数据库，不包含云端或其他设备的记录"
    }
    .into();
    source
}

fn database_cached_mode(path: &Path, key: &str, mode: ScanMode) -> Source {
    type Stamp = Vec<(PathBuf, Option<(u64, std::time::SystemTime)>)>;
    type Entry = (Stamp, String, ScanMode, std::time::Instant, Source);
    static CACHE: once_cell::sync::Lazy<parking_lot::Mutex<HashMap<(PathBuf, String), Entry>>> =
        once_cell::sync::Lazy::new(Default::default);
    let paths = [
        path.to_owned(),
        PathBuf::from(format!("{}-wal", path.to_string_lossy())),
    ];
    let stamp: Stamp = paths
        .into_iter()
        .map(|path| {
            let metadata = std::fs::metadata(&path)
                .ok()
                .and_then(|m| Some((m.len(), m.modified().ok()?)));
            (path, metadata)
        })
        .collect();
    let day = chrono::Local::now().format("%F %z").to_string();
    let mut cache = CACHE.lock();
    let cache_key = (path.to_owned(), key.to_owned());
    if let Some((old, old_day, old_mode, at, source)) = cache.get(&cache_key) {
        if *old == stamp
            && *old_day == day
            && *old_mode == mode
            && (source.agg.incomplete_files == 0 || at.elapsed().as_secs() < 30)
        {
            let mut source = source.clone();
            for session in &mut source.agg.sessions {
                if matches!(session.state.as_str(), "running" | "asking")
                    && chrono::Utc::now().timestamp() - session.last_activity_at >= 1800
                {
                    session.state = "unknown".into();
                }
            }
            return source;
        }
    }
    let source = database_mode(path, key, mode);
    if cache.len() > 8 {
        cache.clear();
    }
    cache.insert(
        cache_key,
        (stamp, day, mode, std::time::Instant::now(), source.clone()),
    );
    source
}

fn read_json(path: &Path) -> Option<Value> {
    if std::fs::metadata(path).ok()?.len() > 16 * 1024 * 1024 {
        return None;
    }
    serde_json::from_reader(std::fs::File::open(path).ok()?).ok()
}
fn files(root: &Path, name: &str, limit: usize) -> (Vec<PathBuf>, bool) {
    let mut pending = vec![(root.to_owned(), 0)];
    let mut out = Vec::new();
    let mut complete = true;
    let mut visited = 0;
    while let Some((dir, depth)) = pending.pop() {
        visited += 1;
        if visited > 50000 {
            complete = false;
            break;
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    complete = false;
                }
                continue;
            }
        };
        for entry in entries {
            let Ok(entry) = entry else {
                complete = false;
                continue;
            };
            let Ok(kind) = entry.file_type() else {
                complete = false;
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() && depth < 7 {
                pending.push((entry.path(), depth + 1));
            }
            if kind.is_file() && entry.file_name() == name {
                out.push(entry.path());
            }
            if out.len() >= limit {
                complete = false;
                break;
            }
        }
        if out.len() >= limit {
            break;
        }
    }
    out.sort();
    (out, complete)
}
fn lines(path: &Path, mut on_value: impl FnMut(Value)) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if meta.len() > 128 * 1024 * 1024 {
        return false;
    }
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut reader = std::io::BufReader::new(file);
    let mut line = String::new();
    let mut complete = true;
    loop {
        line.clear();
        match reader
            .by_ref()
            .take(8 * 1024 * 1024 + 1)
            .read_line(&mut line)
        {
            Ok(0) => break,
            Ok(n) if n <= 8 * 1024 * 1024 => {}
            _ => return false,
        }
        if !line.ends_with('\n') {
            return false;
        }
        match serde_json::from_str(&line) {
            Ok(value) => on_value(value),
            Err(_) => complete = false,
        }
    }
    complete
}
fn kimi_mode(roots: &[PathBuf], mode: ScanMode) -> Source {
    let mut ledger = Ledger::default();
    for root in roots {
        let (paths, complete) = files(root, "wire.jsonl", 20000);
        ledger.incomplete |= !complete;
        for path in paths {
            let parent = path.parent().unwrap_or(root);
            let parent_name = parent
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            let nested = parent
                .parent()
                .is_some_and(|p| p.file_name().is_some_and(|n| n == "agents"));
            let session_dir = if nested {
                parent.parent().and_then(Path::parent).unwrap_or(parent)
            } else {
                parent
            };
            let session_id = session_dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let child = nested && parent_name != "main";
            let id = if child {
                format!("{session_id}:{parent_name}")
            } else {
                session_id
            };
            let metadata = read_json(&session_dir.join("state.json")).unwrap_or(Value::Null);
            ledger.records.entry(id.clone()).or_insert_with(|| Record {
                child,
                project: folder(
                    metadata["work_dir"]
                        .as_str()
                        .or_else(|| metadata["cwd"].as_str())
                        .unwrap_or_default(),
                ),
                model: metadata["model"].as_str().map(str::to_owned),
                ..Default::default()
            });
            let mut turn = 0;
            let mut step = 0;
            let mut recognized = false;
            let complete = lines(&path, |value| {
                let message = &value["message"];
                let kind = message["type"].as_str().unwrap_or_default();
                let payload = &message["payload"];
                let time = ts(&value["timestamp"]);
                let state = match kind {
                    "TurnBegin" => {
                        turn += 1;
                        "running"
                    }
                    "StepBegin" => {
                        step = payload["n"].as_u64().unwrap_or(step + 1);
                        "running"
                    }
                    "TurnEnd" => "completed",
                    "StepInterrupted" => "ended",
                    "ApprovalRequest" => "asking",
                    "ApprovalResponse" => "running",
                    _ => "",
                };
                if !state.is_empty() {
                    recognized = true;
                    ledger.event(&id, time, state);
                }
                if kind == "StatusUpdate" {
                    recognized = true;
                    ledger.event(&id, time, "");
                    if !mode.counts() {
                        return;
                    }
                    if let Some(usage) = payload["token_usage"].as_object() {
                        if usage.get("input_other").and_then(Value::as_u64).is_none()
                            || usage.get("output").and_then(Value::as_u64).is_none()
                        {
                            ledger.incomplete = true;
                            return;
                        }
                        recognized = true;
                        let n = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or(0);
                        let amounts = [
                            n("input_other"),
                            n("output"),
                            n("input_cache_read"),
                            n("input_cache_creation"),
                            n("input_other")
                                .saturating_add(n("output"))
                                .saturating_add(n("input_cache_read"))
                                .saturating_add(n("input_cache_creation")),
                        ];
                        let message = payload["message_id"]
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| format!("{id}:turn:{turn}:step:{step}"));
                        ledger.usage(&id, message, time, amounts);
                        ledger.event(&id, time, "");
                    }
                }
            });
            ledger.incomplete |= !complete || !recognized;
        }
    }
    let mut source = ledger.finish("kimi");
    source.detail = "Kimi Code wire 事件；不将 context_tokens 上下文占用计为消费量".into();
    source
}
fn grok(root: &Path) -> Source {
    let mut ledger = Ledger::default();
    let (paths, complete) = files(root, "usage.json", 20000);
    ledger.incomplete |= !complete;
    for path in paths {
        let Some(value) = read_json(&path) else {
            ledger.incomplete = true;
            continue;
        };
        let id = text(&value, "sessionId");
        if id.is_empty() {
            ledger.incomplete = true;
            continue;
        }
        let summary = read_json(&path.with_file_name("summary.json")).unwrap_or(Value::Null);
        let kind = summary["session_kind"].as_str().unwrap_or_default();
        // 官方主会话 usage 已 fold 子代理用量，不能再把子目录计数重复累加。
        if matches!(kind, "subagent" | "subagent_fork") {
            continue;
        }
        let parent = summary["parent_session_id"]
            .as_str()
            .filter(|s| !s.is_empty());
        let boundary = summary["forked_at"]
            .as_str()
            .or_else(|| summary["created_at"].as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|time| time.timestamp_millis());
        if parent.is_some() && boundary.is_none() {
            ledger.incomplete = true;
            continue;
        }
        if !summary.is_object() {
            ledger.incomplete = true;
        }
        let record = ledger.records.entry(id.clone()).or_default();
        record.project = folder(
            summary
                .pointer("/info/cwd")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        );
        record.model = value
            .pointer("/session/primaryModelId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        ledger.event(&id, ts(&value["updatedAt"]), "");
        let turns = value["turns"].as_array();
        let mut summed = 0u64;
        if let Some(turns) = turns {
            for turn in turns {
                if ["totalTokens", "inputTokens", "outputTokens"]
                    .iter()
                    .any(|key| turn[key].as_u64().is_none())
                {
                    ledger.incomplete = true;
                    continue;
                }
                let n = |name: &str| turn[name].as_u64().unwrap_or(0);
                let time = ts(&turn["endedAt"]);
                summed = summed.saturating_add(n("totalTokens"));
                if parent.is_some() {
                    let ended = turn["endedAt"]
                        .as_str()
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                        .map(|time| time.timestamp_millis());
                    if ended.is_none() {
                        ledger.incomplete = true;
                        continue;
                    }
                    if ended <= boundary {
                        continue;
                    }
                }
                let Some(number) = turn["turnNumber"].as_u64() else {
                    ledger.incomplete = true;
                    continue;
                };
                // turns 是每轮增量；session 是累计值，二者不能同时累加。
                ledger.usage(
                    &id,
                    format!("{id}:{number}"),
                    time,
                    [
                        n("inputTokens"),
                        n("outputTokens"),
                        n("cachedReadTokens"),
                        n("cacheCreationTokens"),
                        n("totalTokens"),
                    ],
                );
                ledger.incomplete |= turn["usageIsIncomplete"].as_bool() == Some(true);
            }
        }
        if summed
            != value
                .pointer("/session/totalTokens")
                .and_then(Value::as_u64)
                .unwrap_or(summed)
        {
            ledger.incomplete = true;
        }
        ledger.incomplete |= value
            .pointer("/session/usageIsIncomplete")
            .and_then(Value::as_bool)
            == Some(true);
        // usage 的写入并非完成事件，生命周期由官方 hook 单独提供。
    }
    let mut source = ledger.finish("grok");
    source.input_includes_cache = true;
    source.detail = "Grok Build 每轮本地用量，剔除继承历史及父会话已汇总的子代理记录；缺失元数据时标记统计不完整".into();
    source
}

fn item(key: &str, source: Source, now: i64) -> AiUsageItem {
    let agg = source.agg;
    AiUsageItem {
        key: key.into(),
        complete: agg.incomplete_files == 0,
        usage_available: source.available,
        session_available: !agg.sessions.is_empty(),
        input_includes_cached: Some(source.input_includes_cache),
        data_source: format!("{} 本地会话", crate::ai::agent_tools::label(key)),
        daily: super::daily_usage(&agg),
        recent_sessions: super::recent_sessions(&agg),
        last_activity_ts: (agg.last_ts > 0).then_some(agg.last_ts),
        scanned_at: now,
        note: source.detail,
        today: agg.today,
        week: agg.week,
        last7: agg.last7,
        last30: agg.last30,
        all: agg.all,
        ..Default::default()
    }
}
pub(super) async fn collect(plan: NativePlan) -> Vec<AiUsageItem> {
    // Cache only sanitized results, keyed by the same immutable engine plan used by this request.
    type Entry = (std::time::Instant, NativePlan, Vec<AiUsageItem>);
    static CACHE: once_cell::sync::Lazy<parking_lot::Mutex<Option<Entry>>> =
        once_cell::sync::Lazy::new(Default::default);
    tokio::task::spawn_blocking(move || {
        let mut cache = CACHE.lock();
        if let Some((at, old, items)) = cache.as_ref() {
            if *old == plan && at.elapsed().as_secs() < 10 {
                return items.clone();
            }
        }
        let now = chrono::Utc::now().timestamp();
        let base = home();
        let opencode = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| base.join(".local/share"))
            .join("opencode/opencode.db");
        let mut kimi_roots: Vec<PathBuf> = [
            std::env::var_os("KIMI_CODE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| base.join(".kimi-code"))
                .join("sessions"),
            std::env::var_os("KIMI_SHARE_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| base.join(".kimi"))
                .join("sessions"),
        ]
        .into_iter()
        .collect();
        kimi_roots.sort();
        kimi_roots.dedup();
        let mut items = Vec::new();
        if plan.enabled("opencode") {
            items.push(item(
                "opencode",
                database_cached_mode(&opencode, "opencode", plan.mode("opencode")),
                now,
            ));
        }
        if plan.enabled("glm") {
            items.push(item(
                "glm",
                database_cached_mode(
                    &base.join(".zcode/cli/db/db.sqlite"),
                    "glm",
                    plan.mode("glm"),
                ),
                now,
            ));
        }
        if plan.enabled("kimi") {
            items.push(item("kimi", kimi_mode(&kimi_roots, plan.mode("kimi")), now));
        }
        // Grok lifecycle is supplied by hooks. Do not re-open usage.json when the engine owns its counts.
        let grok = if plan.enabled("grok") && plan.mode("grok").counts() {
            grok(
                &std::env::var_os("GROK_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| base.join(".grok"))
                    .join("sessions"),
            )
        } else {
            Source::default()
        };
        if plan.enabled("grok") {
            items.push(item("grok", grok, now));
        }
        for key in ["cursor", "antigravity"]
            .into_iter()
            .filter(|key| plan.enabled(key))
        {
            items.push(item(
                key,
                Source {
                    detail: "等待统一用量引擎读取；工作、等待及完成由独立状态连接提供".into(),
                    ..Default::default()
                },
                now,
            ));
        }
        *cache = Some((std::time::Instant::now(), plan, items.clone()));
        items
    })
    .await
    .unwrap_or_default()
}

#[cfg(test)]
fn database(path: &Path, key: &str) -> Source {
    database_mode(path, key, ScanMode::Usage)
}
#[cfg(test)]
fn database_cached(path: &Path, key: &str) -> Source {
    database_cached_mode(path, key, ScanMode::Usage)
}
#[cfg(test)]
fn kimi(roots: &[PathBuf]) -> Source {
    kimi_mode(roots, ScanMode::Usage)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn database_mode_cache_switches_to_activity_and_back_without_stale_counts() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("mode.db");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE session(id TEXT,directory TEXT,parent_id TEXT);CREATE TABLE message(id TEXT,session_id TEXT,time_created INTEGER,data TEXT);INSERT INTO session VALUES('s','C:/demo',NULL);").unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        connection.execute("INSERT INTO message VALUES('m','s',?1,?2)",rusqlite::params![now,serde_json::json!({"role":"assistant","finish":"stop","time":{"created":now,"completed":now},"tokens":{"input":100,"output":20}}).to_string()]).unwrap();
        drop(connection);
        for key in ["opencode", "glm"] {
            assert_eq!(
                database_cached_mode(&path, key, ScanMode::Usage)
                    .agg
                    .all
                    .total,
                120
            );
            let activity = database_cached_mode(&path, key, ScanMode::Activity);
            assert_eq!(activity.agg.all.total, 0);
            assert!(activity.agg.daily.is_empty());
            assert!(!activity.available);
            assert_eq!(activity.agg.sessions[0].state, "completed");
            assert_eq!(
                database_cached_mode(&path, key, ScanMode::Usage)
                    .agg
                    .all
                    .total,
                120
            );
        }
    }
    #[test]
    fn zcode_activity_reads_final_turns_without_querying_model_token_columns() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("turns.db");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE session(id TEXT,directory TEXT,parent_id TEXT);CREATE TABLE message(id TEXT,session_id TEXT,time_created INTEGER,data TEXT);CREATE TABLE model_usage(session_id TEXT);CREATE TABLE turn_usage(session_id TEXT,status TEXT,started_at INTEGER,completed_at INTEGER);INSERT INTO session VALUES('s','C:/demo',NULL);INSERT INTO model_usage VALUES('s');").unwrap();
        let now = chrono::Utc::now().timestamp_millis() - 10000;
        connection.execute("INSERT INTO message VALUES('m','s',?1,?2)",rusqlite::params![now,serde_json::json!({"role":"assistant","finish":"tool-calls","time":{"created":now,"completed":now}}).to_string()]).unwrap();
        connection
            .execute(
                "INSERT INTO turn_usage VALUES('s','completed',?1,?2)",
                rusqlite::params![now, now + 1000],
            )
            .unwrap();
        drop(connection);
        let source = database_mode(&path, "glm", ScanMode::Activity);
        assert_eq!(source.agg.incomplete_files, 0);
        assert_eq!(source.agg.sessions[0].state, "completed");
        assert_eq!(source.agg.all.total, 0);
    }
    #[test]
    fn opencode_and_zcode_keep_source_even_when_model_is_claude() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("usage.db");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE session(id TEXT,directory TEXT,parent_id TEXT); CREATE TABLE message(id TEXT,session_id TEXT,time_created INTEGER,data TEXT); INSERT INTO session VALUES('same','C:/project',NULL);").unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let data = serde_json::json!({"role":"assistant","modelID":"claude-fixture","time":{"created":now,"completed":now},"finish":"stop","tokens":{"input":100,"output":20,"total":170,"cache":{"read":50,"write":0}}});
        connection
            .execute(
                "INSERT INTO message VALUES('m','same',?1,?2)",
                rusqlite::params![now, data.to_string()],
            )
            .unwrap();
        drop(connection);
        for key in ["opencode", "glm"] {
            let source = database(&path, key);
            assert_eq!(source.agg.all.total, 170);
            assert_eq!(source.agg.sessions[0].tool, key);
            assert_eq!(source.agg.sessions[0].state, "completed");
            assert_eq!(
                source.agg.sessions[0].model.as_deref(),
                Some("claude-fixture")
            );
        }
        assert_eq!(database(&path, "opencode").agg.all.total, 170);
    }
    #[test]
    fn kimi_counts_message_once_and_keeps_status_without_usage() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("project/session/wire.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let now = chrono::Utc::now().timestamp();
        let event = |kind: &str, payload: Value| {
            serde_json::json!({"timestamp":now,"message":{"type":kind,"payload":payload}})
                .to_string()
                + "\n"
        };
        let update = event(
            "StatusUpdate",
            serde_json::json!({"message_id":"m","context_tokens":999999,"token_usage":{"input_other":100,"output":20,"input_cache_read":50}}),
        );
        std::fs::write(
            &path,
            event("TurnBegin", Value::Null)
                + &update
                + &update
                + &event("ApprovalRequest", Value::Null),
        )
        .unwrap();
        let source = kimi(&[fixture.path().to_owned()]);
        assert_eq!(source.agg.all.total, 170);
        assert_eq!(source.agg.sessions[0].state, "asking");
        let activity = kimi_mode(&[fixture.path().to_owned()], ScanMode::Activity);
        assert!(!activity.available);
        assert!(activity.agg.daily.is_empty());
        assert_eq!(activity.agg.all.total, 0);
        assert_eq!(activity.agg.sessions[0].state, "asking");
        std::fs::write(&path, event("TurnBegin", Value::Null)).unwrap();
        let source = kimi(&[fixture.path().to_owned()]);
        assert!(!source.available);
        assert_eq!(source.agg.sessions[0].state, "running");
    }
    #[test]
    fn grok_counts_turns_without_readding_session_total_or_claiming_completion() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("project/session/usage.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path,serde_json::json!({"sessionId":"s","updatedAt":chrono::Utc::now().to_rfc3339(),"session":{"totalTokens":170},"turns":[{"turnNumber":1,"endedAt":chrono::Utc::now().to_rfc3339(),"inputTokens":150,"outputTokens":20,"cachedReadTokens":50,"totalTokens":170}]}).to_string()).unwrap();
        let source = grok(fixture.path());
        assert_eq!(source.agg.all.total, 170);
        assert_eq!(source.agg.sessions[0].state, "unknown");
    }
    #[test]
    fn grok_inherited_usage_and_aggregated_subagents_are_not_counted_twice() {
        let fixture = crate::test_support::FixtureDir::new();
        let now = chrono::Utc::now();
        let boundary = now - chrono::Duration::seconds(20);
        let inherited = now - chrono::Duration::seconds(30);
        for (id, kind, parent, time) in [
            ("main", "new", None, inherited),
            ("fork", "fork", Some("main"), now),
            ("child", "subagent", Some("main"), now),
        ] {
            let dir = fixture.path().join(id);
            std::fs::create_dir_all(&dir).unwrap();
            let mut turns = vec![
                serde_json::json!({"turnNumber":1,"endedAt":inherited.to_rfc3339(),"inputTokens":100,"outputTokens":20,"totalTokens":120}),
            ];
            if id != "main" {
                turns.push(serde_json::json!({"turnNumber":2,"endedAt":time.to_rfc3339(),"inputTokens":10,"outputTokens":5,"totalTokens":15}));
            }
            std::fs::write(dir.join("usage.json"),serde_json::json!({"sessionId":id,"updatedAt":time.to_rfc3339(),"session":{"totalTokens":if id=="main"{120}else{135}},"turns":turns}).to_string()).unwrap();
            std::fs::write(dir.join("summary.json"),serde_json::json!({"info":{"cwd":"C:/demo"},"session_kind":kind,"parent_session_id":parent,"forked_at":boundary.to_rfc3339()}).to_string()).unwrap();
        }
        let source = grok(fixture.path());
        assert_eq!(source.agg.all.total, 135);
        assert_eq!(source.agg.sessions.len(), 2);
        assert_eq!(source.agg.incomplete_files, 0);
    }
    #[test]
    fn database_cache_reuses_unchanged_counts_and_follows_wal_changes() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("wal.db");
        let connection = Connection::open(&path).unwrap();
        connection.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE session(id TEXT,directory TEXT,parent_id TEXT); CREATE TABLE message(id TEXT,session_id TEXT,time_created INTEGER,data TEXT); INSERT INTO session VALUES('s','C:/demo',NULL);").unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let data = serde_json::json!({"role":"assistant","finish":"stop","time":{"created":now,"completed":now},"tokens":{"input":10,"output":5,"total":15}});
        connection
            .execute(
                "INSERT INTO message VALUES('m1','s',?1,?2)",
                rusqlite::params![now, data.to_string()],
            )
            .unwrap();
        assert_eq!(database_cached(&path, "opencode").agg.all.total, 15);
        assert_eq!(database_cached(&path, "opencode").agg.all.total, 15);
        connection
            .execute(
                "INSERT INTO message VALUES('m2','s',?1,?2)",
                rusqlite::params![now, data.to_string()],
            )
            .unwrap();
        assert_eq!(database_cached(&path, "opencode").agg.all.total, 30);
    }
    #[test]
    fn malformed_database_and_missing_sources_are_not_zero_usage_evidence() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("broken.db");
        std::fs::write(&path, b"broken").unwrap();
        let source = database(&path, "glm");
        assert!(!source.available);
        assert!(source.agg.incomplete_files > 0);
        assert!(!database(&fixture.path().join("missing"), "opencode").available);
    }
    #[test]
    #[ignore = "只读本机计数核验，不输出消息正文或账号"]
    fn real_databases_count_only() {
        for (key, path) in [
            ("opencode", home().join(".local/share/opencode/opencode.db")),
            ("glm", home().join(".zcode/cli/db/db.sqlite")),
        ] {
            let at = std::time::Instant::now();
            let source = database(&path, key);
            assert!(source.available);
            assert_eq!(source.agg.incomplete_files, 0);
            println!(
                "{key}: total={}, sessions={}, elapsed_ms={}",
                source.agg.all.total,
                source.agg.sessions.len(),
                at.elapsed().as_millis()
            );
        }
    }
}
