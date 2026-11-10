//! ZCode 模型源：读取 v2 配置中的服务商，OpenCode 源查订阅窗口，编程套餐读开通状态。
//!
//! ZCode 把自定义服务商写在 `provider_config.json`，内置/模板服务商写在 `config.json`；
//! 编程套餐的额度是渲染进程缓存（LevelDB 里的 usage-entitlement），这里不越权解析，
//! 只按 `coding-plan-cache.json` 给出开通/未开通，避免把缺失当成零额度。

use super::*;
use serde_json::Value;
use std::path::Path;
use std::time::Instant;

#[derive(Debug, Clone, Default, PartialEq)]
struct Seed {
    id: String,
    label: String,
    base: Option<String>,
    key: Option<String>,
    models: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Subscription,
    Plan,
    Unsupported,
}

impl Kind {
    fn key(self) -> &'static str {
        match self {
            Self::Subscription => "subscription",
            Self::Plan => "plan",
            Self::Unsupported => "unsupported",
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Subscription => 0,
            Self::Plan => 1,
            Self::Unsupported => 2,
        }
    }
}

fn kind_of(seed: &Seed) -> Kind {
    let haystack = format!(
        "{} {} {}",
        seed.id,
        seed.label,
        seed.base.as_deref().unwrap_or_default()
    )
    .to_ascii_lowercase();
    if haystack.contains("coding-plan") || haystack.contains("zcode-plan") {
        Kind::Plan
    } else if haystack.contains("opencode") {
        Kind::Subscription
    } else {
        Kind::Unsupported
    }
}

fn push(seeds: &mut Vec<Seed>, seed: Seed) {
    if seed.id.is_empty() {
        return;
    }
    if let Some(existing) = seeds.iter_mut().find(|known| known.id == seed.id) {
        if seed.base.is_some() {
            existing.base = seed.base;
        }
        if seed.key.is_some() {
            existing.key = seed.key;
        }
        if !seed.label.is_empty() {
            existing.label = seed.label;
        }
        for model in seed.models {
            if !existing.models.contains(&model) {
                existing.models.push(model);
            }
        }
        return;
    }
    seeds.push(seed);
}

fn models_from_object(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_object)
        .map(|models| models.keys().take(3).cloned().collect())
        .unwrap_or_default()
}

fn models_from_order(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(Value::as_str)
                .take(3)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// `provider_config.json`：用户新增的自定义服务商（例如 opencode-go-chat）。
fn parse_provider_config(value: &Value) -> Vec<Seed> {
    let Some(rules) = value.pointer("/config/providerConfigRules/providerRules") else {
        return Vec::new();
    };
    let Some(rules) = rules.as_array() else {
        return Vec::new();
    };
    let mut seeds = Vec::new();
    for rule in rules {
        let id = rule
            .get("providerId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let label = rule
            .get("providerName")
            .and_then(Value::as_str)
            .unwrap_or(id);
        let config = rule.get("config");
        let key = config
            .and_then(|config| config.pointer("/access/apiKey"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_owned);
        let mut base = None;
        if format!("{id} {label}")
            .to_ascii_lowercase()
            .contains("opencode")
        {
            base = Some("https://opencode.ai/zen/go/v1".into());
        }
        push(
            &mut seeds,
            Seed {
                id: id.to_owned(),
                label: label.to_owned(),
                base,
                key,
                models: models_from_order(config.and_then(|config| config.get("modelOrder"))),
            },
        );
    }
    seeds
}

/// `config.json`：内置与模板服务商；只收显式启用的、带密钥的自定义项，以及编程套餐。
fn parse_config(value: &Value) -> Vec<Seed> {
    let Some(providers) = value.get("provider").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut seeds = Vec::new();
    for (id, provider) in providers {
        let name = provider.get("name").and_then(Value::as_str).unwrap_or(id);
        let options = provider.get("options");
        let base = options
            .and_then(|options| options.get("baseURL"))
            .and_then(Value::as_str)
            .map(|base| base.trim_end_matches('/').to_owned());
        let key = options
            .and_then(|options| options.get("apiKey"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_owned);
        let enabled = provider
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let source = provider.get("source").and_then(Value::as_str);
        let plan = format!("{} {}", id, base.as_deref().unwrap_or_default())
            .to_ascii_lowercase()
            .contains("coding-plan")
            || id.contains("coding-plan")
            || base
                .as_deref()
                .is_some_and(|base| base.contains("zcode-plan"));
        let keep = if plan {
            // 未开通/已禁用的套餐不占额度行；只有显式启用或未声明且带凭据的套餐才展示。
            enabled || (key.is_some() && provider.get("enabled").is_none())
        } else {
            enabled || (source == Some("custom") && key.is_some())
        };
        if !keep {
            continue;
        }
        push(
            &mut seeds,
            Seed {
                id: id.clone(),
                label: name.to_owned(),
                base,
                key,
                models: models_from_object(provider.get("models")),
            },
        );
    }
    seeds
}

fn merge(mut seeds: Vec<Seed>, overrides: Vec<Seed>) -> Vec<Seed> {
    for seed in overrides {
        push(&mut seeds, seed);
    }
    // 先按可查询程度排序再截断，避免把带额度接口的来源挤出上限。
    seeds.sort_by_key(|seed| kind_of(seed).rank());
    seeds.truncate(MAX_PROVIDERS);
    seeds
}

/// `coding-plan-cache.json`：ZCode 自己记录的内置套餐开通状态。
fn parse_plan_status(value: &Value) -> std::collections::HashMap<String, (bool, String)> {
    let mut statuses = std::collections::HashMap::new();
    let Some(items) = value
        .pointer("/entryStatus/items")
        .and_then(Value::as_object)
    else {
        return statuses;
    };
    for (id, entry) in items {
        let status = entry.get("status").and_then(Value::as_str).unwrap_or("");
        let reason = entry
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or_default();
        statuses.insert(id.clone(), (status == "available", reason.to_owned()));
    }
    statuses
}

fn plan_note(available: bool, reason: &str) -> String {
    if available {
        "已开通 · 套餐用量由 ZCode 缓存".into()
    } else {
        match reason {
            "coding_plan_not_entitled" => "未开通该编程套餐".into(),
            "" => "套餐状态未知".into(),
            other => format!("套餐不可用（{other}）"),
        }
    }
}

fn read_json(path: &Path) -> Option<Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
}

fn zcode_root() -> std::path::PathBuf {
    home().join(".zcode").join("v2")
}

fn discover() -> (Vec<Seed>, std::collections::HashMap<String, (bool, String)>) {
    let root = zcode_root();
    let custom = read_json(&root.join("provider_config.json"))
        .map(|value| parse_provider_config(&value))
        .unwrap_or_default();
    let config = read_json(&root.join("config.json"))
        .map(|value| parse_config(&value))
        .unwrap_or_default();
    let plans = read_json(&root.join("coding-plan-cache.json"))
        .map(|value| parse_plan_status(&value))
        .unwrap_or_default();
    (merge(config, custom), plans)
}

/// 只读本机配置得到的服务商行：尚未同步额度时也能列出服务商与模型。
fn blank_row(seed: &Seed) -> AiProviderQuota {
    let kind = kind_of(seed);
    AiProviderQuota {
        id: seed.id.clone(),
        label: seed.label.clone(),
        kind: kind.key().into(),
        status: "unsynced".into(),
        note: "启动 Agent 后自动同步额度".into(),
        windows: Vec::new(),
        balances: Vec::new(),
        models: seed.models.clone(),
        captured_at: 0,
        active: false,
    }
}

fn sort_rows(rows: &mut [AiProviderQuota]) {
    rows.sort_by(|left, right| {
        kind_rank(&left.kind)
            .cmp(&kind_rank(&right.kind))
            .then_with(|| status_rank(&left.status).cmp(&status_rank(&right.status)))
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn discovered_rows(seeds: &[Seed]) -> Vec<AiProviderQuota> {
    let mut rows: Vec<AiProviderQuota> = seeds.iter().map(blank_row).collect();
    sort_rows(&mut rows);
    rows
}

/// 当前使用的服务商：`setting.json` 的 `modelProviderFamilySelectedKeys`，
/// 值形如 `coding-plan:builtin:bigmodel-coding-plan`，取能对上的最长服务商 id。
fn active_provider(root: &Path, rows: &[AiProviderQuota]) -> Option<String> {
    let value = read_json(&root.join("setting.json"))?;
    let selected = value.get("modelProviderFamilySelectedKeys")?.as_object()?;
    let mut best: Option<&str> = None;
    for key in selected.values().filter_map(Value::as_str) {
        for row in rows {
            if !key.ends_with(row.id.as_str()) {
                continue;
            }
            if best.map_or(true, |id| row.id.len() > id.len()) {
                best = Some(row.id.as_str());
            }
        }
    }
    best.map(str::to_owned)
}

async fn probe() -> Vec<AiProviderQuota> {
    let (seeds, plans) = discover();
    let now = chrono::Utc::now().timestamp();
    let mut rows: Vec<AiProviderQuota> = Vec::new();
    let mut jobs = tokio::task::JoinSet::new();
    for seed in seeds {
        let kind = kind_of(&seed);
        let mut row = blank_row(&seed);
        row.status = "unconfigured".into();
        row.note.clear();
        row.captured_at = now;
        match kind {
            Kind::Subscription => {
                let Some(key) = seed.key.clone() else {
                    row.note = "该服务商未保存密钥".into();
                    rows.push(row);
                    continue;
                };
                let base = seed.base.clone();
                jobs.spawn(async move { (row, fetch_subscription(base.as_deref(), &key).await) });
            }
            Kind::Plan => {
                match plans.get(&seed.id) {
                    Some((true, _)) => {
                        row.status = "ok".into();
                        row.note = plan_note(true, "");
                    }
                    Some((false, reason)) => {
                        row.status = "unconfigured".into();
                        row.note = plan_note(false, reason);
                    }
                    None => {
                        row.status = "unconfigured".into();
                        row.note = "ZCode 尚未记录该套餐状态".into();
                    }
                }
                rows.push(row);
            }
            Kind::Unsupported => {
                row.status = "unsupported".into();
                row.note = "ZCode 未提供该服务商的额度查询".into();
                rows.push(row);
            }
        }
    }
    while let Some(joined) = jobs.join_next().await {
        let Ok((mut row, result)) = joined else {
            continue;
        };
        match result {
            Ok(windows) => {
                row.status = "ok".into();
                row.windows = windows;
            }
            Err(note) => {
                row.status = "error".into();
                row.note = note;
            }
        }
        rows.push(row);
    }
    sort_rows(&mut rows);
    rows
}

fn kind_rank(key: &str) -> u8 {
    match key {
        "subscription" => 0,
        "plan" => 1,
        _ => 2,
    }
}

fn status_rank(key: &str) -> u8 {
    match key {
        "ok" => 0,
        "unconfigured" => 1,
        "error" => 2,
        _ => 3,
    }
}

/// 读取 ZCode 模型源额度：Agent 运行中按固定间隔查询，关闭后只返回最后一次快照。
pub(crate) async fn snapshot(live: bool) -> Vec<AiProviderQuota> {
    static CACHE: once_cell::sync::Lazy<
        tokio::sync::Mutex<Option<(Instant, Vec<AiProviderQuota>)>>,
    > = once_cell::sync::Lazy::new(Default::default);
    let root = zcode_root();
    let mut cache = CACHE.lock().await;
    let cached = cache
        .as_ref()
        .map(|(_, rows)| rows.clone())
        .unwrap_or_default();
    let mut rows = if live {
        let due = cache.as_ref().map_or(true, |(at, _)| {
            at.elapsed() >= Duration::from_secs(super::SYNC_INTERVAL_SECS)
        });
        if due {
            let rows = probe().await;
            *cache = Some((Instant::now(), rows.clone()));
            rows
        } else {
            cached
        }
    } else {
        // Agent 未运行：只读配置列服务商，额度保留最后一次同步的结果。
        let (seeds, _) = discover();
        let mut rows = discovered_rows(&seeds);
        super::merge_synced(&mut rows, &cached);
        rows
    };
    let active = active_provider(&root, &rows);
    for row in &mut rows {
        row.active = active.as_deref() == Some(row.id.as_str());
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_opencode_provider_is_recognized_from_provider_config() {
        let value = serde_json::json!({
            "config": {
                "providerOrder": ["opencode-go-chat"],
                "providerConfigRules": { "providerRules": [{
                    "providerId": "opencode-go-chat",
                    "providerName": "OpenCode Go (Chat)",
                    "config": {
                        "access": { "type": "api-key", "apiKey": "oc_sk_fixture" },
                        "modelOrder": ["glm-5.3", "deepseek-v4.1-flash"]
                    }
                }]}
            }
        });
        let seeds = parse_provider_config(&value);
        assert_eq!(seeds.len(), 1);
        assert_eq!(
            seeds[0].base.as_deref(),
            Some("https://opencode.ai/zen/go/v1")
        );
        assert_eq!(seeds[0].key.as_deref(), Some("oc_sk_fixture"));
        assert_eq!(seeds[0].models, vec!["glm-5.3", "deepseek-v4.1-flash"]);
        assert_eq!(kind_of(&seeds[0]), Kind::Subscription);
    }

    #[test]
    fn config_keeps_enabled_plans_and_custom_keys_but_skips_empty_builtins() {
        let value = serde_json::json!({
            "provider": {
                "builtin:bigmodel-start-plan": {
                    "name": "BigModel- Coding Plan",
                    "enabled": true,
                    "options": { "baseURL": "https://zcode.z.ai/api/v1/zcode-plan/anthropic/", "apiKey": "jwt" },
                    "models": { "GLM-5.3": {}, "GLM-5.3-Flash": {} }
                },
                "builtin:bigmodel": {
                    "name": "Bigmodel - API Key",
                    "options": { "baseURL": "https://open.bigmodel.cn/api/anthropic", "apiKey": "" },
                    "models": { "GLM-5.3": {} }
                },
                "relay": {
                    "name": "GLM01",
                    "source": "custom",
                    "options": { "baseURL": "https://ark.cn-beijing.volces.com/api/coding", "apiKey": "ark" },
                    "models": { "glm-5.3": {} }
                }
            }
        });
        let seeds = parse_config(&value);
        assert_eq!(seeds.len(), 2);
        assert_eq!(
            seeds[0].base.as_deref(),
            Some("https://zcode.z.ai/api/v1/zcode-plan/anthropic")
        );
        assert_eq!(kind_of(&seeds[0]), Kind::Plan);
        assert_eq!(kind_of(&seeds[1]), Kind::Unsupported);
        assert_eq!(seeds[0].models, vec!["GLM-5.3", "GLM-5.3-Flash"]);
    }

    #[test]
    fn plan_cache_maps_available_and_entitlement_reasons() {
        let value = serde_json::json!({
            "entryStatus": { "items": {
                "builtin:bigmodel-start-plan": { "status": "available" },
                "builtin:bigmodel-coding-plan": { "status": "unavailable", "reason": "coding_plan_not_entitled" }
            }}
        });
        let plans = parse_plan_status(&value);
        assert!(plans["builtin:bigmodel-start-plan"].0);
        assert!(!plans["builtin:bigmodel-coding-plan"].0);
        assert_eq!(
            plan_note(false, "coding_plan_not_entitled"),
            "未开通该编程套餐"
        );
        assert!(plan_note(true, "").starts_with("已开通"));
    }

    #[test]
    fn selected_provider_key_maps_to_the_longest_provider_id() {
        let fixture = crate::test_support::FixtureDir::new();
        std::fs::write(
            fixture.path().join("setting.json"),
            serde_json::json!({
                "modelProviderFamilySelectedKeys": {
                    "bigmodel": "coding-plan:builtin:bigmodel-coding-plan"
                }
            })
            .to_string(),
        )
        .unwrap();
        let rows = vec![
            AiProviderQuota {
                id: "builtin:bigmodel".into(),
                ..Default::default()
            },
            AiProviderQuota {
                id: "builtin:bigmodel-coding-plan".into(),
                ..Default::default()
            },
        ];
        assert_eq!(
            active_provider(fixture.path(), &rows).as_deref(),
            Some("builtin:bigmodel-coding-plan")
        );
        assert_eq!(
            active_provider(fixture.path(), &rows[..1]),
            None,
            "选中的服务商不在列表里时不猜"
        );
    }

    #[test]
    fn unsynced_rows_are_listed_before_the_first_sync() {
        let seeds = vec![Seed {
            id: "glm02".into(),
            label: "GLM02".into(),
            base: None,
            key: None,
            models: vec!["glm-5.3-flash".into()],
        }];
        let rows = discovered_rows(&seeds);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, "unsynced");
        assert_eq!(rows[0].captured_at, 0);
        assert_eq!(rows[0].label, "GLM02");
    }

    #[test]
    fn merge_prefers_the_newest_key_and_base_without_duplicates() {
        let custom = vec![Seed {
            id: "opencode-go-chat".into(),
            label: "OpenCode Go (Chat)".into(),
            base: Some("https://opencode.ai/zen/go/v1".into()),
            key: Some("fresh".into()),
            models: vec!["glm-5.3".into()],
        }];
        let config = vec![Seed {
            id: "opencode-go-chat".into(),
            label: "old".into(),
            base: None,
            key: Some("stale".into()),
            models: vec!["deepseek-v4.1-flash".into()],
        }];
        let seeds = merge(config, custom);
        assert_eq!(seeds.len(), 1);
        assert_eq!(seeds[0].key.as_deref(), Some("fresh"));
        assert_eq!(seeds[0].models.len(), 2);
    }

    #[tokio::test]
    #[ignore = "读取本机 ZCode 配置并查询 OpenCode 额度，只输出来源与状态，不打印密钥"]
    async fn local_sources_only() {
        let rows = probe().await;
        for row in &rows {
            println!(
                "ZCODE_PROVIDER id={} kind={} status={} windows={} note={}",
                row.id,
                row.kind,
                row.status,
                row.windows.len(),
                row.note
            );
        }
        assert!(rows.iter().any(|row| row.kind == "subscription"));
        assert!(rows.iter().any(|row| row.kind == "plan"));
    }
}
