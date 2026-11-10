//! 用量快照的数据结构：只保留计数与展示元数据，不含提示词或正文。

use crate::ai::aimon::activity::AiSession;
use serde::Serialize;
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AiUsage {
    pub input: u64,
    pub output: u64,
    pub cached: u64,
    pub cache_write: u64,
    pub total: u64,
    pub sessions: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiAccount {
    pub email: Option<String>,
    pub name: Option<String>,
    /// free / plus / pro / max …（原样透传）
    pub plan: Option<String>,
    pub plan_label: Option<String>,
    pub workspace: Option<String>,
    pub billing_type: Option<String>,
    /// 该账号是否有本地快照可切换（Codex 多账号）
    pub switchable: bool,
}

/// 官方额度快照：Codex 来自本地 rollout，Claude 来自只读 OAuth 用量接口。
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiRateWindow {
    /// 0-1，超过 1 = 已超额
    pub utilization: f64,
    pub limit_window_seconds: u64,
    pub reset_after_seconds: u64,
    pub reset_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiRateLimit {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub metrics: Vec<AiQuotaMetric>,
    pub primary: Option<AiRateWindow>,
    pub secondary: Option<AiRateWindow>,
    pub captured_at: Option<i64>,
    /// 日志没有可核验的额度账号标识，不能归到当前登录账号。
    pub account_verified: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiQuotaMetric {
    pub label: String,
    pub used_percent: f64,
    pub remaining_percent: Option<f64>,
    pub remaining_label: Option<String>,
    pub reset_at: Option<i64>,
    pub account_label: String,
    pub plan: String,
    pub captured_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiDailyUsage {
    pub date: String,
    pub usage: AiUsage,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderWindow {
    pub label: String,
    /// 已用百分比；与 AiQuotaMetric 一样允许超过 100。
    pub used_percent: f64,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderBalance {
    pub label: String,
    /// 平台返回的原始金额字符串，避免二次浮点换算。
    pub amount: String,
    pub currency: String,
}

/// DSH 实际加载的模型源：订阅制给百分比窗口，余额制给币种余额。
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderQuota {
    pub id: String,
    pub label: String,
    /// subscription / balance / unsupported
    pub kind: String,
    /// ok / error / unconfigured / unsupported / unsynced
    pub status: String,
    pub note: String,
    pub windows: Vec<AiProviderWindow>,
    pub balances: Vec<AiProviderBalance>,
    pub models: Vec<String>,
    /// 最近一次同步时刻；未同步过为 0。
    pub captured_at: i64,
    /// 当前 Agent 正在使用的模型源（默认模型或已选服务商）。
    pub active: bool,
}

/// 模型源额度的同步状态：只有 Agent 运行时才按固定间隔查询，关闭后保留最后一份快照。
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderSync {
    /// Agent 正在运行 → 额度按间隔刷新。
    pub live: bool,
    /// 固定同步间隔（秒）。
    pub interval_seconds: u64,
    /// 最近一次同步的时刻（Unix 秒）；从未同步时为 None。
    pub last_synced_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiUsageItem {
    /// 数据来源工具键：claude / chatgpt / deepseek 等，与模型名称无关。
    pub key: String,
    pub complete: bool,
    pub usage_available: bool,
    /// 生命周期与 Token 能力独立，只有状态事件时也能驱动 Bot 和提醒。
    pub session_available: bool,
    pub input_includes_cached: Option<bool>,
    pub data_source: String,
    pub today: AiUsage,
    pub week: AiUsage,
    pub last7: AiUsage,
    pub last30: AiUsage,
    pub all: AiUsage,
    pub account: AiAccount,
    pub rate: AiRateLimit,
    /// DSH 实际加载的模型源额度；其他工具为空，前端据此分行展示。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<AiProviderQuota>,
    /// 模型源额度的同步状态；没有模型源时不带该字段。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_sync: Option<AiProviderSync>,
    /// 一句话状态（最近活动 + 用量规模提示）
    pub note: String,
    pub last_activity_ts: Option<i64>,
    /// 采样时间（Unix 秒）
    pub scanned_at: i64,
    pub daily: Vec<AiDailyUsage>,
    pub recent_sessions: Vec<AiSession>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSnapshot {
    pub items: Vec<AiUsageItem>,
}
