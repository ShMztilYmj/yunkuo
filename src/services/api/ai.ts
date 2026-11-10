import { call } from "./invoke";

/** 工具连接快照的后端载荷形状；由各 AI 模块消费。 */
export interface ToolConnection {
  key: string;
  model: string | null;
  source: string;
  endpoint: string | null;
  note: string;
}

// ---------- AI 用量监控（token-monitor 风格，本地数据优先） ----------

export interface AiUsageTokens {
  input: number;
  output: number;
  cached: number;
  cacheWrite: number;
  total: number;
  sessions: number;
}

export interface AiAccountInfo {
  email: string | null;
  name: string | null;
  /** free / plus / pro / max …（工具原样透传） */
  plan: string | null;
  planLabel: string | null;
  workspace: string | null;
  billingType: string | null;
  /** 是否有本地快照可一键切换（Codex 多账号） */
  switchable: boolean;
}

export interface AiRateWindow {
  /** 0-1，>1 表示已超额 */
  utilization: number;
  limitWindowSeconds: number;
  resetAfterSeconds: number;
  resetAt?: number | null;
}

export interface AiQuotaMetric {
  label: string;
  usedPercent: number;
  remainingPercent: number | null;
  remainingLabel: string | null;
  resetAt: number | null;
  accountLabel: string;
  plan: string;
  capturedAt: number;
}
export interface AiRateLimit {
  metrics?: AiQuotaMetric[];
  primary: AiRateWindow | null;
  secondary: AiRateWindow | null;
  capturedAt?: number | null;
  accountVerified?: boolean;
}

/** DSH 模型源的订阅窗口：已用百分比与重置时刻。 */
export interface AiProviderWindow {
  label: string;
  usedPercent: number;
  resetsAt: number | null;
}

/** DSH 官方账号余额：保留平台返回的原始金额字符串。 */
export interface AiProviderBalance {
  label: string;
  amount: string;
  currency: string;
}

/** 模型源额度；订阅制给窗口，余额制给余额，套餐只给开通状态。 */
export interface AiProviderQuota {
  id: string;
  label: string;
  kind: "subscription" | "balance" | "plan" | "unsupported";
  status: "ok" | "error" | "unconfigured" | "unsupported" | "unsynced";
  note: string;
  windows: AiProviderWindow[];
  balances: AiProviderBalance[];
  models: string[];
  capturedAt: number;
  /** 当前 Agent 正在使用的模型源（默认模型/已选服务商）。 */
  active?: boolean;
}

/** 额度同步状态：只有 Agent 运行时才按固定间隔刷新。 */
export interface AiProviderSync {
  live: boolean;
  intervalSeconds: number;
  lastSyncedAt: number | null;
}

export interface AiSession {
  id: string;
  tool: string;
  project: string;
  model: string | null;
  state: "running" | "asking" | "completed" | "ended" | "unknown";
  lastActivityAt: number;
  completedAt: number | null;
  total: number;
  usageAvailable?: boolean;
}
export interface AiDailyUsage {
  date: string;
  usage: AiUsageTokens;
}

export interface AiUsageItem {
  key: string;
  complete: boolean;
  usageAvailable: boolean;
  sessionAvailable?: boolean;
  inputIncludesCached?: boolean | null;
  dataSource: string;
  today: AiUsageTokens;
  week: AiUsageTokens;
  last7?: AiUsageTokens;
  last30?: AiUsageTokens;
  all: AiUsageTokens;
  account: AiAccountInfo;
  rate: AiRateLimit;
  /** DSH 模型源额度；其他工具为空。 */
  providers?: AiProviderQuota[];
  /** 模型源额度的同步状态；没有模型源时缺省。 */
  providerSync?: AiProviderSync;
  note: string;
  lastActivityTs: number | null;
  scannedAt: number;
  daily?: AiDailyUsage[];
  recentSessions?: AiSession[];
}

export interface AiSnapshot {
  items: AiUsageItem[];
}
export interface AiMonitorStatus {
  enabled: boolean;
  notices: { id: number; message: string; createdAt: number }[];
  summary: {
    todayTokens: number;
    running: number;
    scannedAt: number | null;
    partial?: boolean;
  };
  deliveryError: string | null;
}

export interface AiSwitchResult {
  ok: boolean;
  message: string;
}

export interface AiAccountEntry {
  legacy: boolean;
  which: string;
  email: string | null;
  label: string;
}

export const aiApi = {
  aiMigrateAccounts: () => call<AiSwitchResult>("ai_migrate_accounts"),
  aiUsageStatus: () => call<AiSnapshot>("ai_usage_status"),
  aiMonitorStatus: () => call<AiMonitorStatus>("ai_monitor_status"),
  aiMonitorSetEnabled: (enabled: boolean) =>
    call<void>("ai_monitor_set_enabled", { enabled }),
  aiMonitorDismiss: (id: number) => call<void>("ai_monitor_dismiss", { id }),
  aiSwitchAccount: (tool: string, which: string) =>
    call<AiSwitchResult>("ai_switch_account", { tool, which }),
  aiSnapshotAccount: (tool: string) =>
    call<AiSwitchResult>("ai_snapshot_account", { tool }),
  aiListAccounts: (tool: string) =>
    call<AiAccountEntry[]>("ai_list_accounts", { tool }),
};
