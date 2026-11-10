import type {
  AiAccountEntry,
  AiAccountInfo,
  AiProviderQuota,
  AiUsageItem,
} from "../../../services/api";
import { providerEntryKey, visibleProviders } from "./providerVisibility";
import type { ToolSetup } from "./toolSetup";

/**
 * 「服务商与 Agent」的展示数据：每个已添加的 Agent 一列，列内按登记顺序列出
 * 服务商与账号。数据全部来自已有的只读快照，不在这里发请求，也不读取密钥。
 */
export type ProviderBindingState =
  "active" | "configured" | "empty" | "error" | "managed" | "unsynced";

/** 额度窗口：模型源查询或官方账号快照里的已用百分比。 */
export interface ProviderQuotaWindow {
  label: string;
  usedPercent: number;
  resetsAt: number | null;
  /** 后端给出的剩余百分比；缺省时由 100 - 已用 兜底。 */
  remainingPercent?: number | null;
}

export interface ProviderQuotaBalance {
  label: string;
  amount: string;
  currency: string;
}

/** 计费方式：订阅制看窗口，按量付费看余额，编程套餐只看开通状态。 */
export type ProviderBilling =
  "subscription" | "balance" | "plan" | "unsupported";

/**
 * 一条登记能展示的额度。
 *
 * - `provider`：模型源额度，只有 Agent 运行中才按固定间隔同步；
 * - `account`：账号额度来自官方快照（Codex 本地日志、Claude 官方用量接口），
 *   不随 Agent 进程同步。
 */
export interface ProviderQuotaView {
  /** 计费方式决定展示形态：窗口条还是余额。 */
  billing: ProviderBilling;
  /** 额度查询状态：决定「套餐已开通 / 暂不支持 / 未同步」的说法。 */
  status: AiProviderQuota["status"];
  windows: ProviderQuotaWindow[];
  balances: ProviderQuotaBalance[];
  /** 最近一次读数时间；0 表示还没同步过。 */
  capturedAt: number;
  from: "provider" | "account";
  /** 模型源额度的同步状态；账号快照类额度为 null。 */
  sync: {
    live: boolean;
    intervalSeconds: number;
    lastSyncedAt: number | null;
  } | null;
}

export interface ProviderBinding {
  id: string;
  /** 服务商显示名：官方、自定义接口主机名或工具自管。 */
  provider: string;
  /** 账号显示名：邮箱、快照名称或凭据状态。 */
  account: string;
  /** 补充说明：模型、套餐或失败原因；没有就不占行高。 */
  detail: string | null;
  state: ProviderBindingState;
  source: "login" | "snapshot" | "config" | "tool";
  /** 用户添加的模型源才可移除；官方来源、账号条目与工具自管只读。 */
  deletable: boolean;
  /** 当前 Agent 正在使用的服务商（默认模型/已选服务商）。 */
  active: boolean;
  /** 可展示的额度；没有额度来源时为 null。 */
  quota: ProviderQuotaView | null;
  /** 占位行不是真实登记：仅在读不到数据时提示，不计入汇总。 */
  placeholder?: boolean;
}

/** 状态与来源的完整措辞：矩阵单元格、详情卡片与图例共用同一份。 */
export const PROVIDER_STATE_LABEL: Record<ProviderBindingState, string> = {
  active: "已核验",
  configured: "已保存",
  empty: "未配置",
  error: "需处理",
  managed: "工具自管",
  unsynced: "待同步",
};
export const PROVIDER_SOURCE_LABEL: Record<ProviderBinding["source"], string> =
  {
    login: "本机登录",
    snapshot: "本地加密快照",
    config: "用户级配置",
    tool: "工具自身维护",
  };

export interface AgentProviderColumn {
  key: string;
  label: string;
  short: string;
  hint: string;
  bindings: ProviderBinding[];
}

export interface ProviderAgentMeta {
  key: string;
  label: string;
  short: string;
  hint: string;
}

export interface NamedIdentity {
  name: string;
  email: string | null;
}

export interface ProviderAgentInput {
  meta: ProviderAgentMeta;
  item?: AiUsageItem;
  entry?: ToolSetup;
  codexAccounts?: readonly AiAccountEntry[];
  claudeIdentities?: readonly NamedIdentity[];
}

const MANAGED_PROVIDER = "由工具自身管理";
const UNKNOWN_PROVIDER = "连接配置未读取";

function endpointHost(endpoint: string): string {
  try {
    return new URL(endpoint).host;
  } catch {
    return endpoint.replace(/^[a-z]+:\/\//i, "").split("/")[0] ?? "";
  }
}

/** 用户级配置里的连接方式：官方直连、自定义接口或工具自管。 */
export function providerFrom(
  entry: ToolSetup | undefined,
  official: string,
): { name: string; known: boolean; official: boolean } {
  const connection = entry?.connection;
  if (!connection || connection.source === "unknown")
    return { name: UNKNOWN_PROVIDER, known: false, official: false };
  if (connection.source === "native")
    return { name: official, known: true, official: true };
  if (connection.source === "external") {
    const host = connection.endpoint ? endpointHost(connection.endpoint) : "";
    return {
      name: host ? `自定义接口 · ${host}` : "自定义接口",
      known: true,
      official: false,
    };
  }
  return { name: MANAGED_PROVIDER, known: true, official: false };
}

function accountName(account: AiAccountInfo | undefined): string | null {
  if (!account) return null;
  return account.email ?? account.name ?? null;
}

function accountDetail(account: AiAccountInfo | undefined): string | null {
  if (!account) return null;
  const parts = [account.planLabel, account.workspace].filter(
    (value): value is string => Boolean(value),
  );
  return parts.length ? parts.join(" · ") : null;
}

/** 窗口名：只按窗口长度说话，猜不出长度时用「主要 / 次要」。 */
function rateWindowLabel(seconds: number, slot: string): string {
  if (seconds === 604800) return "每周";
  if (seconds >= 86400 && seconds % 86400 === 0) return `${seconds / 86400} 天`;
  if (seconds > 0 && seconds % 3600 === 0) return `${seconds / 3600} 小时`;
  if (seconds > 0 && seconds % 60 === 0) return `${seconds / 60} 分钟`;
  return slot;
}

/** 账号额度：最新的官方窗口快照，不随 Agent 进程同步。 */
export function accountQuotaView(
  item: AiUsageItem | undefined,
): ProviderQuotaView | null {
  const rate = item?.rate;
  if (!rate) return null;
  const scannedAt = item?.scannedAt ?? 0;
  const windows: ProviderQuotaWindow[] = [];
  if (rate.metrics?.length) {
    for (const metric of rate.metrics)
      windows.push({
        label: metric.label || "额度窗口",
        usedPercent: metric.usedPercent,
        resetsAt: metric.resetAt,
        remainingPercent: metric.remainingPercent,
      });
  } else {
    ([rate.primary, rate.secondary] as const).forEach((window, index) => {
      if (!window) return;
      windows.push({
        label: rateWindowLabel(
          window.limitWindowSeconds,
          index === 0 ? "主要" : "次要",
        ),
        usedPercent: window.utilization * 100,
        resetsAt:
          window.resetAt ??
          (window.resetAfterSeconds > 0
            ? scannedAt + window.resetAfterSeconds
            : null),
      });
    });
  }
  if (!windows.length) return null;
  return {
    billing: "subscription",
    status: "ok",
    windows,
    balances: [],
    capturedAt: rate.capturedAt ?? 0,
    from: "account",
    sync: null,
  };
}

/** 额度色阶：先看已用百分比，和统计页的阈值保持一致。 */
export function quotaTone(percent: number): "good" | "warning" | "danger" {
  return percent >= 90 ? "danger" : percent >= 70 ? "warning" : "good";
}

function money(amount: string, currency: string): string {
  const symbol = currency.toUpperCase() === "USD" ? "$" : "¥";
  return `${symbol}${amount}`;
}

/** 余额展示：保留平台返回的原始金额字符串，只补币种符号。 */
export function formatAmount(amount: string, currency: string): string {
  return money(amount, currency);
}

/** 余额标签：后台返回「充值余额 / 赠金」，列表里充值余额直接叫「余额」。 */
export function balanceLabel(label: string): string {
  return label === "充值余额" ? "余额" : label;
}

/** 单元格一行的额度摘要；完整读数在详情卡片里。 */
export function quotaSummary(quota: ProviderQuotaView): string | null {
  if (quota.billing === "balance") {
    // 计费方式已经写在账号行（按量付费），这里只列余额，避免同一句说两遍。
    const parts = quota.balances.map(
      (balance) =>
        `${balanceLabel(balance.label)} ${money(balance.amount, balance.currency)}`,
    );
    return parts.length ? parts.join(" · ") : null;
  }
  if (quota.billing === "plan") return null;
  const parts: string[] = [];
  for (const window of quota.windows)
    parts.push(`${window.label} ${Math.round(window.usedPercent)}%`);
  for (const balance of quota.balances)
    parts.push(
      `${balanceLabel(balance.label)} ${money(balance.amount, balance.currency)}`,
    );
  return parts.length ? parts.join(" · ") : null;
}

/** 额度同步状态文案：实时同步 / 已暂停 / 尚未同步。 */
export function quotaSyncText(
  quota: ProviderQuotaView,
  readAt: string | null = null,
): string {
  const clock = (at: number | null | undefined) =>
    at
      ? new Date(at * 1000).toLocaleTimeString("zh-CN", { hour12: false })
      : null;
  if (quota.from === "account") {
    const at = clock(quota.capturedAt) ?? readAt;
    return at ? `官方额度快照 · 采集于 ${at}` : "官方额度快照";
  }
  // 同步状态看这一行自己的读数：别的服务商同步过，不代表这一行有额度。
  const rowAt = clock(quota.capturedAt);
  const interval = quota.sync?.intervalSeconds
    ? `每 ${quota.sync.intervalSeconds} 秒`
    : "按固定间隔";
  if (quota.sync?.live)
    return rowAt
      ? `实时同步 · ${interval} · 最近 ${rowAt}`
      : `实时同步 · ${interval} · 等待首次读数`;
  if (rowAt) return `Agent 未运行，同步已暂停 · 最后同步 ${rowAt}`;
  return "尚未同步 · 启动 Agent 后自动同步";
}

function dedupe(bindings: ProviderBinding[]): ProviderBinding[] {
  const seen = new Set<string>();
  return bindings.filter((binding) => {
    const key = `${binding.provider}\u0000${binding.account}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

function placeholder(
  id: string,
  provider: string,
  account: string,
  detail: string,
): ProviderBinding {
  return {
    id,
    provider,
    account,
    detail,
    state: "empty",
    source: "config",
    deletable: false,
    active: false,
    quota: null,
    placeholder: true,
  };
}

/** Claude 只有一套服务端连接，账号却可能是当前登录 + 多份本地身份备份。 */
function claudeColumn(input: ProviderAgentInput): ProviderBinding[] {
  const provider = providerFrom(input.entry, "Anthropic 官方");
  const bindings: ProviderBinding[] = [];
  const account = input.item?.account;
  const current = accountName(account);
  if (!input.item)
    bindings.push(
      placeholder(
        "claude:unread",
        provider.name,
        "当前账号未读取",
        "本地用量扫描未开启或读取失败；下方是已保存的本地身份备份。",
      ),
    );
  if (current) {
    bindings.push({
      id: "claude:current",
      provider: provider.name,
      account: current,
      detail: accountDetail(account) ?? "当前登录账号",
      state: "active",
      source: "login",
      deletable: false,
      active: true,
      quota: accountQuotaView(input.item),
    });
  }
  (input.claudeIdentities ?? []).forEach((identity, index) => {
    if (!identity.name && !identity.email) return;
    bindings.push({
      id: `claude:identity:${index}`,
      provider: provider.name,
      account: identity.name || identity.email || "本地身份备份",
      detail: identity.email ? `身份备份 · ${identity.email}` : "本地身份备份",
      state: "configured",
      source: "snapshot",
      deletable: false,
      active: false,
      quota: null,
    });
  });
  const rows = dedupe(bindings);
  return rows.length
    ? rows
    : [
        placeholder(
          "claude:empty",
          provider.name,
          "尚未登录",
          "未读取到本机登录账号；在 Claude Code 登录后显示。",
        ),
      ];
}

/** Codex 的账号来自本地加密快照；ChatGPT 桌面账号不在管理范围内。 */
function codexColumn(input: ProviderAgentInput): ProviderBinding[] {
  const provider = providerFrom(input.entry, "OpenAI 官方");
  const bindings: ProviderBinding[] = [];
  const account = input.item?.account;
  const current = accountName(account);
  if (!input.item)
    bindings.push(
      placeholder(
        "codex:unread",
        provider.name,
        "当前账号未读取",
        "本地用量扫描未开启或读取失败；下方是已保存的本地账号快照。",
      ),
    );
  if (current) {
    bindings.push({
      id: "codex:current",
      provider: provider.name,
      account: current,
      detail: accountDetail(account) ?? "当前登录账号",
      state: "active",
      source: "login",
      deletable: false,
      active: true,
      quota: accountQuotaView(input.item),
    });
  }
  (input.codexAccounts ?? []).forEach((snapshot, index) => {
    bindings.push({
      id: `codex:snapshot:${index}`,
      provider: provider.name,
      account: snapshot.label,
      detail: snapshot.legacy
        ? "旧版快照 · 需要在账号面板加密迁移"
        : "本地加密账号快照",
      state: snapshot.legacy ? "error" : "configured",
      source: "snapshot",
      deletable: false,
      active: false,
      quota: null,
    });
  });
  const rows = dedupe(bindings);
  return rows.length
    ? rows
    : [
        placeholder(
          "codex:empty",
          provider.name,
          "尚未保存账号",
          "在 Codex 登录后可保存本地账号快照；桌面端账号需在官方应用中切换。",
        ),
      ];
}

/** 模型源额度行同时说明账号凭据状态，不能把查询失败当成未配置。 */
function quotaAccount(provider: AiProviderQuota): {
  account: string;
  detail: string | null;
  state: ProviderBindingState;
} {
  const models = provider.models.length ? provider.models.join(" · ") : null;
  const note = provider.note.trim() || null;
  if (provider.status === "ok") {
    const account =
      provider.kind === "balance"
        ? "按量付费"
        : provider.kind === "subscription"
          ? "API 密钥"
          : provider.kind === "plan"
            ? "编程套餐"
            : "已接入";
    return { account, detail: models ?? note, state: "active" };
  }
  if (provider.status === "error") {
    return {
      account:
        provider.kind === "subscription" ? "密钥或网络异常" : "账号查询失败",
      detail: note ?? models,
      state: "error",
    };
  }
  if (provider.status === "unsynced") {
    // 还没同步过：不给假的 0，也不把「未同步」写成「未配置」。
    return { account: "等待首次同步", detail: models, state: "unsynced" };
  }
  if (provider.status === "unconfigured") {
    const account =
      provider.kind === "plan"
        ? "套餐未开通或状态未知"
        : provider.kind === "balance"
          ? "未登录官方账号"
          : "未配置凭据";
    return { account, detail: note ?? models, state: "empty" };
  }
  return {
    account: "由服务商控制台管理",
    detail: models ?? note,
    state: "managed",
  };
}

/**
 * 官方来源：官方账号类条目——余额制的平台账号，以及名称里明写「官方」的服务商。
 * 其余模型源都是用户在工具里自己添加的，可以从对照表移除。
 */
export function officialProvider(provider: AiProviderQuota): boolean {
  return provider.kind === "balance" || provider.label.includes("官方");
}

/** 模型源额度的展示视图：服务商页详情卡片与悬浮条共用同一份。 */
export function providerQuotaView(
  provider: AiProviderQuota,
  sync?: AiUsageItem["providerSync"],
): ProviderQuotaView {
  return {
    billing: provider.kind,
    status: provider.status,
    windows: provider.windows.map((window) => ({
      label: window.label,
      usedPercent: window.usedPercent,
      resetsAt: window.resetsAt,
    })),
    balances: provider.balances.map((balance) => ({
      label: balance.label,
      amount: balance.amount,
      currency: balance.currency,
    })),
    capturedAt: provider.capturedAt,
    from: "provider",
    sync: sync
      ? {
          live: sync.live,
          intervalSeconds: sync.intervalSeconds,
          lastSyncedAt: sync.lastSyncedAt,
        }
      : null,
  };
}

/** 当前正在使用的服务商：后端标记优先，其次取第一个有读数的来源。 */
export function activeProviderOf(
  providers: readonly AiProviderQuota[],
): AiProviderQuota | null {
  return (
    providers.find((provider) => provider.active === true) ??
    providers.find(
      (provider) => provider.windows.length > 0 || provider.balances.length > 0,
    ) ??
    providers.find((provider) => provider.status === "ok") ??
    providers[0] ??
    null
  );
}

/** 各 Agent 的官方服务商：账号额度（官方快照）只属于这一家。 */
export const OFFICIAL_PROVIDER: Readonly<Record<string, string>> = {
  claude: "Anthropic 官方",
  chatgpt: "OpenAI 官方",
  deepseek: "DSH 模型源",
  glm: "ZCode 服务商",
};

/**
 * 工具卡上的服务商额度：这组读数属于哪一家，以及还算不算「使用中」。
 *
 * - `active`：Agent 当前就在用这家，读数是活的；
 * - `last`：连接已经指向别处，读数是上一次使用留下的；
 * - `unknown`：还没有任何读数，只有服务商名字。
 */
export interface ToolQuotaView {
  /** 服务商显示名：官方、自定义接口主机名或模型源名称。 */
  provider: string;
  state: "active" | "last" | "unknown";
  /** 订阅窗口或余额；没有可用读数时为 null。 */
  quota: ProviderQuotaView | null;
}

/**
 * 取「使用中或上次使用」的服务商额度，工具卡与详情面板共用这一份判定。
 *
 * 账号额度优先：它来自官方额度快照，能说明当前登录账号的真实剩余；
 * 只有官方直连时才标「使用中」，走自定义接口时这份快照属于上一次的使用记录。
 * 没有账号额度才回退到 Agent 当前在用的模型源（按量付费就展示余额）。
 */
export function toolQuotaView(
  toolKey: string,
  item: AiUsageItem | undefined,
  entry: ToolSetup | undefined,
  removedKeys: ReadonlySet<string> = new Set<string>(),
): ToolQuotaView {
  const official = OFFICIAL_PROVIDER[toolKey] || "官方";
  const connection = providerFrom(entry, official);
  // 只有自定义接口才算真的换了服务商：官方直连与工具自管都仍在这家账号下。
  const custom = entry?.connection.source === "external";
  const account = accountQuotaView(item);
  if (account)
    return {
      provider: official,
      state: custom ? "last" : "active",
      quota: account,
    };
  const current = activeProviderOf(
    visibleProviders(toolKey, item?.providers ?? [], removedKeys),
  );
  if (current)
    return {
      provider: current.label || official,
      state: current.active === true ? "active" : "last",
      quota: providerQuotaView(current, item?.providerSync),
    };
  return {
    // 换过接口就点名主机名；其余情况直接说官方，别写成「连接配置未读取」。
    provider: custom ? connection.name : official,
    state: "unknown",
    quota: null,
  };
}

function quotaColumn(
  input: ProviderAgentInput,
  fallback: string,
  hint: string,
): ProviderBinding[] {
  const providers = input.item?.providers ?? [];
  if (!providers.length)
    return [
      placeholder(`${input.meta.key}:empty`, fallback, "未读取到服务商", hint),
    ];
  return providers.map((provider, index) => {
    const view = quotaAccount(provider);
    // 身份为空的服务商无法跨刷新定位，只展示、不提供移除。
    const identified = providerEntryKey(input.meta.key, provider);
    return {
      id: identified || `${input.meta.key}:provider:#${index}`,
      provider: provider.label || provider.id || "未命名服务商",
      account: view.account,
      detail: view.detail,
      state: view.state,
      source: "config" as const,
      deletable: Boolean(identified) && !officialProvider(provider),
      active: provider.active === true,
      quota: providerQuotaView(provider, input.item?.providerSync),
    };
  });
}

/** 其余 Agent 的服务商与登录由工具自身维护，只展示能读到的账号。 */
function managedColumn(input: ProviderAgentInput): ProviderBinding[] {
  const account = input.item?.account;
  const name = accountName(account);
  return [
    {
      id: `${input.meta.key}:managed`,
      provider: MANAGED_PROVIDER,
      account: name ?? "在工具内登录",
      detail: name
        ? (accountDetail(account) ?? "由工具自身维护")
        : `${input.meta.label} 的服务商与登录由工具自身维护。`,
      state: name ? "configured" : "managed",
      source: "tool",
      deletable: false,
      active: false,
      quota: null,
    },
  ];
}

function bindingsFor(input: ProviderAgentInput): ProviderBinding[] {
  switch (input.meta.key) {
    case "claude":
      return claudeColumn(input);
    case "chatgpt":
      return codexColumn(input);
    case "deepseek":
      return quotaColumn(
        input,
        "DSH 模型源",
        "开启本地用量扫描后读取 DSH 模型源；也可以在 DSH 中配置后刷新。",
      );
    case "glm":
      return quotaColumn(
        input,
        "ZCode 服务商",
        "开启本地用量扫描后读取 ZCode 服务商；也可以在 ZCode 中配置后刷新。",
      );
    default:
      return managedColumn(input);
  }
}

export function buildAgentColumns(
  inputs: readonly ProviderAgentInput[],
): AgentProviderColumn[] {
  return inputs.map((input) => ({
    key: input.meta.key,
    label: input.meta.label,
    short: input.meta.short,
    hint: input.meta.hint,
    bindings: bindingsFor(input),
  }));
}

/** 表头只统计已登记的条目；占位与工具自管不算。 */
export function registeredCount(column: AgentProviderColumn): number {
  return column.bindings.filter(
    (binding) =>
      binding.state !== "empty" &&
      !(binding.state === "managed" && binding.source === "tool"),
  ).length;
}

/**
 * 移除过的服务商不再进入对照表。整列都被移除时保留空列（显示为未登记），
 * 不退化成「未读取到服务商」——读取是成功的，只是用户把它移除了。
 */
export function withoutRemovedProviders(
  columns: readonly AgentProviderColumn[],
  removedKeys: ReadonlySet<string>,
): AgentProviderColumn[] {
  if (!removedKeys.size) return [...columns];
  return columns.map((column) => ({
    ...column,
    bindings: column.bindings.filter(
      (binding) => !(binding.deletable && removedKeys.has(binding.id)),
    ),
  }));
}

export function columnRows(columns: readonly AgentProviderColumn[]): number {
  return Math.max(1, ...columns.map((column) => column.bindings.length));
}

export interface ProviderAccountSummary {
  agents: number;
  providers: number;
  accounts: number;
}

export function summarizeColumns(
  columns: readonly AgentProviderColumn[],
): ProviderAccountSummary {
  const providers = new Set<string>();
  let accounts = 0;
  for (const column of columns)
    for (const binding of column.bindings) {
      if (binding.placeholder) continue;
      // 服务商按真实配置计数（含暂不支持查询的来源）；账号只统计有凭据或登录的条目。
      if (binding.provider !== MANAGED_PROVIDER)
        providers.add(binding.provider);
      if (
        binding.state === "active" ||
        binding.state === "configured" ||
        binding.state === "error"
      )
        accounts += 1;
    }
  return { agents: columns.length, providers: providers.size, accounts };
}
