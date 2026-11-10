//! 模型源额度：识别各 Agent 实际加载的模型源，再按来源读取额度。
//!
//! - DSH：`cordis.patch.yml` 与 `.credentials.yaml`；订阅制查 usage，官方账号查余额。
//! - ZCode：`v2/provider_config.json`、`v2/config.json` 与 `coding-plan-cache.json`；
//!   OpenCode 源查订阅窗口，编程套餐只显示开通状态（额度由 ZCode 自己缓存）。
//!
//! 只读本机配置与凭据；查询失败如实报告，不能把失败当成零余额，也不能凭模型名猜测来源。

use super::*;
use serde_json::Value;
use std::path::Path;
use std::time::{Duration, Instant};

mod zcode;

pub(crate) use zcode::snapshot as zcode_snapshot;

const OPENCODE_USAGE: &str = "https://opencode.ai/zen/go/v1/usage";
const PLATFORM_ORIGIN: &str = "https://platform.deepseek.com";
const BALANCE_PATH: &str = "/api/v0/users/get_user_summary";
/// 额度同步的固定间隔：Agent 运行中最多每分钟查询一次。
pub(crate) const SYNC_INTERVAL_SECS: u64 = 60;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_PROVIDERS: usize = 8;
const USER_AGENT: &str = "YunKuo/0.2 (model-source quota)";

// ---------- 模型源识别 ----------

#[derive(Debug, Clone, Default, PartialEq)]
struct ProviderSeed {
    id: String,
    base: Option<String>,
    models: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Subscription,
    Balance,
    Unsupported,
}

impl Kind {
    fn key(self) -> &'static str {
        match self {
            Self::Subscription => "subscription",
            Self::Balance => "balance",
            Self::Unsupported => "unsupported",
        }
    }
}

fn rank_of(key: &str) -> u8 {
    match key {
        "subscription" => 0,
        "balance" => 1,
        _ => 2,
    }
}

fn classify(id: &str) -> Kind {
    let id = id.to_ascii_lowercase();
    if id.contains("opencode") {
        Kind::Subscription
    } else if id.contains("deepseek") {
        Kind::Balance
    } else {
        Kind::Unsupported
    }
}

fn label_for(id: &str, kind: Kind) -> String {
    match kind {
        Kind::Subscription if id.eq_ignore_ascii_case("opencode-go") => "OpenCode Go".into(),
        Kind::Balance => "DeepSeek 官方".into(),
        _ => id.into(),
    }
}

fn push_seed<'a>(seeds: &'a mut Vec<ProviderSeed>, id: &str) -> &'a mut ProviderSeed {
    let index = match seeds
        .iter()
        .position(|seed| seed.id.eq_ignore_ascii_case(id))
    {
        Some(index) => index,
        None => {
            seeds.push(ProviderSeed {
                id: id.to_owned(),
                ..Default::default()
            });
            seeds.len() - 1
        }
    };
    &mut seeds[index]
}

fn merge_seed(seeds: &mut Vec<ProviderSeed>, seed: ProviderSeed) {
    let target = push_seed(seeds, &seed.id);
    if seed.base.is_some() {
        target.base = seed.base;
    }
    for model in seed.models {
        if !target.models.iter().any(|known| known == &model) {
            target.models.push(model);
        }
    }
}

/// 解析一个 profile 的 `cordis.patch.yml`：默认模型行给出 provider/model，pi-ai 行给出
/// providers 映射与 baseURL，账号插件行声明官方账号来源。
fn parse_profile(text: &str) -> Vec<ProviderSeed> {
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(text) else {
        return Vec::new();
    };
    let Some(entries) = value.as_sequence() else {
        return Vec::new();
    };
    let mut seeds: Vec<ProviderSeed> = Vec::new();
    for entry in entries {
        let Some(row) = entry.as_mapping() else {
            continue;
        };
        let name = row
            .get("name")
            .and_then(|value| value.as_str())
            .or_else(|| row.get("id").and_then(|value| value.as_str()))
            .unwrap_or_default();
        if name.contains("dsh-llm-deepseek-account") {
            push_seed(&mut seeds, "deepseek-account-platform");
            continue;
        }
        let Some(config) = row.get("config") else {
            continue;
        };
        if name.ends_with("dsh-agent-default-model") {
            let Some(provider) = config.get("provider").and_then(|value| value.as_str()) else {
                continue;
            };
            let seed = push_seed(&mut seeds, provider);
            if let Some(model) = config.get("model").and_then(|value| value.as_str()) {
                if !seed.models.iter().any(|known| known == model) {
                    seed.models.push(model.to_owned());
                }
            }
        } else if name.ends_with("dsh-llm-pi-ai") {
            let Some(providers) = config.get("providers").and_then(|value| value.as_mapping())
            else {
                continue;
            };
            for (key, value) in providers {
                let Some(id) = key.as_str() else {
                    continue;
                };
                let seed = push_seed(&mut seeds, id);
                if let Some(base) = ["baseURL", "baseUrl", "base_url"]
                    .iter()
                    .find_map(|field| value.get(*field).and_then(|base| base.as_str()))
                {
                    seed.base = Some(base.trim_end_matches('/').to_owned());
                }
            }
        }
    }
    seeds
}

fn collect_seeds(root: &Path) -> Vec<ProviderSeed> {
    let mut seeds = Vec::new();
    let Ok(profiles) = std::fs::read_dir(root.join(".dsh").join("profiles")) else {
        return seeds;
    };
    for profile in profiles.flatten() {
        for file in ["cordis.patch.yml", "cordis.yml"] {
            let Ok(text) = std::fs::read_to_string(profile.path().join(file)) else {
                continue;
            };
            for seed in parse_profile(&text) {
                merge_seed(&mut seeds, seed);
            }
        }
    }
    seeds
}

// ---------- 凭据 ----------

#[derive(Debug, Clone, Default, PartialEq)]
struct Credentials {
    opencode_key: Option<String>,
    /// (grant token, issuer)
    official: Option<(String, String)>,
}

fn parse_credentials(text: &str) -> Credentials {
    let mut credentials = parse_credentials_file(text);
    if credentials.opencode_key.is_none() {
        credentials.opencode_key = std::env::var("OPENCODE_GO_API_KEY")
            .ok()
            .map(|key| key.trim().to_owned())
            .filter(|key| !key.is_empty());
    }
    credentials
}

fn parse_credentials_file(text: &str) -> Credentials {
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(text) else {
        return Credentials::default();
    };
    let opencode_key = value
        .get("refs")
        .and_then(|refs| refs.get("OPENCODE_GO_API_KEY"))
        .and_then(|key| key.as_str())
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_owned);
    let official = value
        .get("records")
        .and_then(|records| records.get("deepseek-account-platform/default"))
        .and_then(|record| {
            let payload = record.get("payload")?;
            let token = payload.get("token")?.as_str()?.trim();
            if token.is_empty() {
                return None;
            }
            let issuer = payload
                .get("issuer")
                .and_then(|issuer| issuer.as_str())
                .map(str::trim)
                .filter(|issuer| !issuer.is_empty())
                .unwrap_or(PLATFORM_ORIGIN);
            Some((token.to_owned(), issuer.to_owned()))
        });
    Credentials {
        opencode_key,
        official,
    }
}

fn merge_seeds(mut seeds: Vec<ProviderSeed>, credentials: &Credentials) -> Vec<ProviderSeed> {
    if credentials.official.is_some()
        && !seeds.iter().any(|seed| classify(&seed.id) == Kind::Balance)
    {
        seeds.push(ProviderSeed {
            id: "deepseek-account-platform".into(),
            ..Default::default()
        });
    }
    if credentials.opencode_key.is_some()
        && !seeds
            .iter()
            .any(|seed| classify(&seed.id) == Kind::Subscription)
    {
        seeds.push(ProviderSeed {
            id: "opencode-go".into(),
            ..Default::default()
        });
    }
    seeds.truncate(MAX_PROVIDERS);
    seeds
}

// ---------- 额度查询 ----------

enum QuotaPayload {
    Windows(Vec<AiProviderWindow>),
    Balances(Vec<AiProviderBalance>),
}

fn http_client() -> reqwest::Client {
    // 模型源额度查询带的是本机凭据：同样只经本机代理出网，不直连。
    let builder = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent(USER_AGENT);
    let builder = match crate::ai::aimon::proxy_port()
        .and_then(|port| reqwest::Proxy::all(format!("http://127.0.0.1:{port}")).ok())
    {
        Some(proxy) => builder.proxy(proxy),
        None => builder.no_proxy(),
    };
    builder.build().unwrap_or_default()
}

fn request_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "查询超时".into()
    } else {
        "网络请求失败".into()
    }
}

fn opencode_url(base: Option<&str>) -> String {
    let Some(base) = base.map(str::trim).filter(|base| !base.is_empty()) else {
        return OPENCODE_USAGE.to_owned();
    };
    let base = base.trim_end_matches('/');
    let candidate = if base.ends_with("/v1") {
        format!("{base}/usage")
    } else if base.ends_with("/zen/go") {
        format!("{base}/v1/usage")
    } else {
        return OPENCODE_USAGE.to_owned();
    };
    // 只信任 opencode.ai 或显式 /zen/go 的入口；其他中转站的 /v1 不猜 /usage。
    if candidate.contains("opencode.ai") || base.contains("/zen/go") {
        candidate
    } else {
        OPENCODE_USAGE.to_owned()
    }
}

async fn fetch_subscription(
    base: Option<&str>,
    key: &str,
) -> Result<Vec<AiProviderWindow>, String> {
    let response = http_client()
        .get(opencode_url(base))
        .bearer_auth(key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(request_error)?;
    let status = response.status();
    if matches!(status.as_u16(), 401 | 403) {
        return Err("密钥无效或已过期".into());
    }
    if !status.is_success() {
        return Err(format!("接口返回 HTTP {}", status.as_u16()));
    }
    let value: Value = response
        .json()
        .await
        .map_err(|_| "接口响应不是有效 JSON".to_string())?;
    parse_opencode_usage(&value)
}

async fn query_balance(issuer: &str, token: &str) -> Result<QuotaPayload, String> {
    let issuer = issuer.trim_end_matches('/');
    let response = http_client()
        .get(format!("{issuer}{BALANCE_PATH}"))
        .header("x-dsh-auth-token", token)
        .header(reqwest::header::ACCEPT, "application/json, text/plain, */*")
        .header(reqwest::header::ORIGIN, issuer)
        .header(reqwest::header::REFERER, format!("{issuer}/usage"))
        .send()
        .await
        .map_err(request_error)?;
    let status = response.status();
    if status.as_u16() == 401 {
        return Err("官方授权已失效".into());
    }
    if !status.is_success() {
        return Err(format!("接口返回 HTTP {}", status.as_u16()));
    }
    let value: Value = response
        .json()
        .await
        .map_err(|_| "接口响应不是有效 JSON".to_string())?;
    parse_official_balance(&value).map(QuotaPayload::Balances)
}

fn parse_opencode_usage(value: &Value) -> Result<Vec<AiProviderWindow>, String> {
    let usage = value
        .get("usage")
        .and_then(Value::as_object)
        .ok_or("响应缺少 usage")?;
    let mut windows = Vec::new();
    for (field, label) in [("rolling", "滚动"), ("weekly", "每周"), ("monthly", "每月")] {
        let Some(window) = usage.get(field) else {
            continue;
        };
        let Some(percent) = window
            .get("percent")
            .and_then(Value::as_f64)
            .filter(|percent| percent.is_finite())
        else {
            continue;
        };
        let resets_at = window
            .get("resetsAt")
            .and_then(Value::as_str)
            .map(rfc3339_to_unix)
            .filter(|time| *time > 0);
        windows.push(AiProviderWindow {
            label: label.into(),
            used_percent: percent,
            resets_at,
        });
    }
    if windows.is_empty() {
        Err("接口没有返回可用额度窗口".into())
    } else {
        Ok(windows)
    }
}

fn parse_official_balance(value: &Value) -> Result<Vec<AiProviderBalance>, String> {
    let wallet = value.pointer("/data/biz_data").ok_or("响应缺少余额字段")?;
    let mut rows = Vec::new();
    for (field, label, keep_zero) in [
        ("normal_wallets", "充值余额", true),
        ("bonus_wallets", "赠金", false),
    ] {
        for entry in wallet
            .get(field)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(amount) = entry.get("balance").and_then(Value::as_str) else {
                continue;
            };
            if !keep_zero && amount.trim().parse::<f64>().is_ok_and(|value| value <= 0.0) {
                continue;
            }
            rows.push(AiProviderBalance {
                label: label.into(),
                amount: amount.to_owned(),
                currency: entry
                    .get("currency")
                    .and_then(Value::as_str)
                    .unwrap_or("CNY")
                    .to_owned(),
            });
        }
    }
    if rows.is_empty() {
        Err("账号没有可展示的余额".into())
    } else {
        Ok(rows)
    }
}

// ---------- 快照 ----------

/// 当前使用的模型源：`dsh-agent-default-model.provider`。
fn parse_active_provider(text: &str) -> Option<String> {
    let value = serde_yaml::from_str::<serde_yaml::Value>(text).ok()?;
    for entry in value.as_sequence()? {
        let row = entry.as_mapping()?;
        let name = row
            .get("name")
            .and_then(|value| value.as_str())
            .or_else(|| row.get("id").and_then(|value| value.as_str()))
            .unwrap_or_default();
        if !name.ends_with("dsh-agent-default-model") {
            continue;
        }
        let provider = row
            .get("config")
            .and_then(|config| config.get("provider"))
            .and_then(|provider| provider.as_str())
            .map(str::trim)
            .filter(|provider| !provider.is_empty());
        if let Some(provider) = provider {
            return Some(provider.to_owned());
        }
    }
    None
}

fn active_provider(root: &Path) -> Option<String> {
    let profiles = std::fs::read_dir(root.join(".dsh").join("profiles")).ok()?;
    for profile in profiles.flatten() {
        for file in ["cordis.patch.yml", "cordis.yml"] {
            let Ok(text) = std::fs::read_to_string(profile.path().join(file)) else {
                continue;
            };
            if let Some(provider) = parse_active_provider(&text) {
                return Some(provider);
            }
        }
    }
    None
}

fn discover() -> (Vec<ProviderSeed>, Credentials) {
    let root = home();
    let credentials = std::fs::read_to_string(root.join(".dsh").join(".credentials.yaml"))
        .ok()
        .map(|text| parse_credentials(&text))
        .unwrap_or_default();
    let seeds = merge_seeds(collect_seeds(&root), &credentials);
    (seeds, credentials)
}

/// 只读本机配置得到的服务商行：尚未同步时也能列出服务商与模型。
fn blank_row(seed: &ProviderSeed) -> AiProviderQuota {
    let kind = classify(&seed.id);
    AiProviderQuota {
        id: seed.id.clone(),
        label: label_for(&seed.id, kind),
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
        rank_of(&left.kind)
            .cmp(&rank_of(&right.kind))
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn discovered_rows(seeds: &[ProviderSeed]) -> Vec<AiProviderQuota> {
    let mut rows: Vec<AiProviderQuota> = seeds.iter().map(blank_row).collect();
    sort_rows(&mut rows);
    rows
}

/// 把上一次成功同步的结果贴回当前配置发现的行：新增的服务商保持未同步状态。
fn merge_synced(rows: &mut [AiProviderQuota], cached: &[AiProviderQuota]) {
    for row in rows.iter_mut() {
        let Some(old) = cached.iter().find(|old| old.id == row.id) else {
            continue;
        };
        if old.captured_at <= 0 {
            continue;
        }
        row.status = old.status.clone();
        row.note = old.note.clone();
        row.windows = old.windows.clone();
        row.balances = old.balances.clone();
        row.captured_at = old.captured_at;
    }
}

fn mark_active(rows: &mut [AiProviderQuota], active: Option<&str>) {
    let Some(active) = active else {
        return;
    };
    for row in rows.iter_mut() {
        row.active = row.id.eq_ignore_ascii_case(active);
    }
}

async fn probe() -> Vec<AiProviderQuota> {
    let (seeds, credentials) = discover();
    let now = chrono::Utc::now().timestamp();
    let mut rows: Vec<AiProviderQuota> = Vec::new();
    let mut jobs = tokio::task::JoinSet::new();
    for seed in seeds {
        let kind = classify(&seed.id);
        let mut row = blank_row(&seed);
        row.status = "unconfigured".into();
        row.note.clear();
        row.captured_at = now;
        match kind {
            Kind::Unsupported => {
                row.status = "unsupported".into();
                row.note = "暂不支持该模型源的额度查询".into();
                rows.push(row);
            }
            Kind::Subscription => {
                let Some(key) = credentials.opencode_key.clone() else {
                    row.note = "未找到 OPENCODE_GO_API_KEY".into();
                    rows.push(row);
                    continue;
                };
                let base = seed.base.clone();
                jobs.spawn(async move {
                    (
                        row,
                        fetch_subscription(base.as_deref(), &key)
                            .await
                            .map(QuotaPayload::Windows),
                    )
                });
            }
            Kind::Balance => {
                let Some((token, issuer)) = credentials.official.clone() else {
                    row.note = "未登录 DeepSeek 官方账号".into();
                    rows.push(row);
                    continue;
                };
                jobs.spawn(async move { (row, query_balance(&issuer, &token).await) });
            }
        }
    }
    while let Some(joined) = jobs.join_next().await {
        let Ok((mut row, result)) = joined else {
            continue;
        };
        match result {
            Ok(QuotaPayload::Windows(windows)) => {
                row.status = "ok".into();
                row.windows = windows;
            }
            Ok(QuotaPayload::Balances(balances)) => {
                row.status = "ok".into();
                row.balances = balances;
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

/// 读取 DSH 模型源额度：Agent 运行中按固定间隔查询，关闭后只返回最后一次快照。
pub(crate) async fn dsh_snapshot(live: bool) -> Vec<AiProviderQuota> {
    static CACHE: once_cell::sync::Lazy<
        tokio::sync::Mutex<Option<(Instant, Vec<AiProviderQuota>)>>,
    > = once_cell::sync::Lazy::new(Default::default);
    let active = active_provider(&home());
    let mut cache = CACHE.lock().await;
    let cached = cache
        .as_ref()
        .map(|(_, rows)| rows.clone())
        .unwrap_or_default();
    let mut rows = if live {
        let due = cache.as_ref().map_or(true, |(at, _)| {
            at.elapsed() >= Duration::from_secs(SYNC_INTERVAL_SECS)
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
        merge_synced(&mut rows, &cached);
        rows
    };
    mark_active(&mut rows, active.as_deref());
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE: &str = r#"
- id: agent-default-model
  name: "@deepseek-ai/dsh-agent-default-model"
  config:
    provider: opencode-go
    model: deepseek-v4.1-flash
- id: llm-pi-ai
  name: "@deepseek-ai/dsh-llm-pi-ai"
  config:
    providers:
      opencode-go:
        apiKeyEnv: OPENCODE_GO_API_KEY
        baseURL: https://opencode.ai/zen/go/v1
- id: llm-deepseek-account
  name: "@deepseek-ai/dsh-llm-deepseek-account"
"#;

    const CREDENTIALS: &str = r#"
version: 1
records:
  deepseek-account-platform/default:
    kind: grant
    payload:
      version: 1
      token: fixture-token
      issuer: https://platform.deepseek.com
refs:
  OPENCODE_GO_API_KEY: oc_sk_fixture
"#;

    #[test]
    fn profile_declares_the_loaded_sources_and_default_model() {
        let seeds = parse_profile(PROFILE);
        assert_eq!(seeds.len(), 2);
        assert_eq!(seeds[0].id, "opencode-go");
        assert_eq!(
            seeds[0].base.as_deref(),
            Some("https://opencode.ai/zen/go/v1")
        );
        assert_eq!(seeds[0].models, vec!["deepseek-v4.1-flash".to_owned()]);
        assert_eq!(seeds[1].id, "deepseek-account-platform");
    }

    #[test]
    fn credentials_supply_both_sources_without_inventing_them() {
        let credentials = parse_credentials(CREDENTIALS);
        assert_eq!(credentials.opencode_key.as_deref(), Some("oc_sk_fixture"));
        let (token, issuer) = credentials.official.unwrap();
        assert_eq!(token, "fixture-token");
        assert_eq!(issuer, "https://platform.deepseek.com");
        assert!(parse_credentials_file("version: 1").opencode_key.is_none());
    }

    #[test]
    fn sources_merge_from_config_and_credentials_once() {
        let credentials = parse_credentials(CREDENTIALS);
        let seeds = merge_seeds(parse_profile(PROFILE), &credentials);
        assert_eq!(seeds.len(), 2);
        let empty = merge_seeds(Vec::new(), &credentials);
        assert_eq!(empty.len(), 2);
        assert_eq!(empty[0].id, "deepseek-account-platform");
        assert_eq!(empty[1].id, "opencode-go");
    }

    #[test]
    fn opencode_windows_keep_percent_and_reset_and_skip_malformed_rows() {
        let value: Value = serde_json::from_str(
            r#"{"usage":{
                "rolling":{"status":"ok","percent":7,"resetsAt":"2026-10-31T07:34:42.000Z"},
                "weekly":{"status":"ok","percent":70,"resetsAt":"2026-11-02T00:00:00.000Z"},
                "monthly":{"status":"ok","percent":"oops"},
                "other":{"percent":99}
            }}"#,
        )
        .unwrap();
        let windows = parse_opencode_usage(&value).unwrap();
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].label, "滚动");
        assert_eq!(windows[0].used_percent, 7.0);
        assert!(windows[0].resets_at.is_some());
        assert_eq!(windows[1].label, "每周");
        assert!(parse_opencode_usage(&serde_json::json!({})).is_err());
    }

    #[test]
    fn official_balance_keeps_raw_amounts_and_hides_zero_bonus() {
        let value: Value = serde_json::from_str(
            r#"{"code":0,"data":{"biz_data":{
                "normal_wallets":[{"currency":"CNY","balance":"1.2992943200000000"}],
                "bonus_wallets":[{"currency":"CNY","balance":"0"}]
            }}}"#,
        )
        .unwrap();
        let rows = parse_official_balance(&value).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "充值余额");
        assert_eq!(rows[0].amount, "1.2992943200000000");
        assert_eq!(rows[0].currency, "CNY");
        assert!(parse_official_balance(&serde_json::json!({"data":{"biz_data":{}}})).is_err());
    }

    #[test]
    fn usage_url_follows_configured_api_roots_but_not_unknown_ones() {
        assert_eq!(
            opencode_url(Some("https://opencode.ai/zen/go/v1/")),
            "https://opencode.ai/zen/go/v1/usage"
        );
        assert_eq!(
            opencode_url(Some("https://opencode.ai/zen/go")),
            "https://opencode.ai/zen/go/v1/usage"
        );
        assert_eq!(
            opencode_url(Some("https://relay.invalid/v1")),
            OPENCODE_USAGE
        );
        assert_eq!(opencode_url(None), OPENCODE_USAGE);
    }

    #[test]
    fn unknown_relays_are_reported_as_unsupported() {
        assert_eq!(classify("my-relay"), Kind::Unsupported);
        assert_eq!(classify("opencode-go"), Kind::Subscription);
        assert_eq!(classify("deepseek-account-platform"), Kind::Balance);
    }

    #[test]
    fn default_model_row_names_the_active_provider() {
        assert_eq!(
            parse_active_provider(PROFILE).as_deref(),
            Some("opencode-go")
        );
        assert_eq!(
            parse_active_provider("- id: other\n  name: other\n"),
            None,
            "没有默认模型行时不猜当前服务商"
        );
    }

    #[test]
    fn unsynced_rows_keep_config_metadata_and_pick_up_the_last_sync() {
        let seeds = vec![ProviderSeed {
            id: "opencode-go".into(),
            base: None,
            models: vec!["deepseek-v4.1-flash".into()],
        }];
        let mut rows = discovered_rows(&seeds);
        assert_eq!(rows[0].status, "unsynced");
        assert_eq!(rows[0].captured_at, 0);
        assert_eq!(rows[0].models, vec!["deepseek-v4.1-flash"]);
        let cached = vec![AiProviderQuota {
            id: "opencode-go".into(),
            status: "ok".into(),
            windows: vec![AiProviderWindow {
                label: "每周".into(),
                used_percent: 40.0,
                resets_at: None,
            }],
            captured_at: 1_800_000_000,
            ..blank_row(&seeds[0])
        }];
        merge_synced(&mut rows, &cached);
        assert_eq!(rows[0].status, "ok");
        assert_eq!(rows[0].captured_at, 1_800_000_000);
        assert_eq!(rows[0].windows.len(), 1);
        mark_active(&mut rows, Some("OPENCODE-GO"));
        assert!(rows[0].active, "服务商 id 比较不区分大小写");
    }

    #[test]
    fn provider_quota_serializes_camel_case_for_the_client() {
        let row = AiProviderQuota {
            id: "opencode-go".into(),
            label: "OpenCode Go".into(),
            kind: "subscription".into(),
            status: "ok".into(),
            note: String::new(),
            windows: vec![AiProviderWindow {
                label: "每周".into(),
                used_percent: 70.0,
                resets_at: Some(1_800_000_000),
            }],
            balances: Vec::new(),
            models: vec!["deepseek-v4.1-flash".into()],
            captured_at: 1_800_000_100,
            active: false,
        };
        let value = serde_json::to_value(&row).unwrap();
        assert_eq!(value["kind"], "subscription");
        assert_eq!(value["windows"][0]["usedPercent"], 70.0);
        assert_eq!(value["windows"][0]["resetsAt"], 1_800_000_000);
        assert_eq!(value["capturedAt"], 1_800_000_100);
        assert_eq!(value["active"], false);
    }

    #[tokio::test]
    #[ignore = "读取本机 DSH 配置并查询额度，只输出来源与窗口，不打印密钥或金额"]
    async fn local_sources_only() {
        let rows = probe().await;
        for row in &rows {
            let windows: Vec<String> = row
                .windows
                .iter()
                .map(|window| format!("{}={:.0}%", window.label, window.used_percent))
                .collect();
            let balances: Vec<String> = row
                .balances
                .iter()
                .map(|balance| format!("{}:{}", balance.label, balance.currency))
                .collect();
            println!(
                "DSH_PROVIDER id={} kind={} status={} windows=[{}] balances=[{}]",
                row.id,
                row.kind,
                row.status,
                windows.join(","),
                balances.join(",")
            );
        }
        assert!(rows.iter().any(|row| row.id == "opencode-go"));
        assert!(rows.iter().any(|row| row.kind == "balance"));
        assert!(
            rows.iter()
                .all(|row| row.status == "ok" || row.status == "error")
        );
    }
}
