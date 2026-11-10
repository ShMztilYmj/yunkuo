import type {
  AiRateLimit,
  AiRateWindow,
  AiSession,
  AiUsageItem,
  AiUsageTokens,
} from "../../../services/api";
import { agentFor, CACHE_EXCLUSIVE_INPUT } from "./aiToolCards";

export type AiPeriod = "today" | "week" | "last7" | "last30" | "all";
export const AI_PERIODS = [
  { key: "today", label: "今日" },
  { key: "week", label: "本周" },
  { key: "last7", label: "近 7 天" },
  { key: "last30", label: "近 30 天" },
  { key: "all", label: "累计" },
] as const;
export const localUsageSupported = (key: string) =>
  (agentFor(key)?.usageClients.length ?? 0) > 0;
export function formatTokens(value: number) {
  if (value >= 1e9) return `${(value / 1e9).toFixed(2)}B`;
  if (value >= 1e6) return `${(value / 1e6).toFixed(2)}M`;
  if (value >= 1e3) return `${(value / 1e3).toFixed(1)}K`;
  return new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 0 }).format(
    value,
  );
}
export function usageFor(
  item: AiUsageItem | undefined,
  period: AiPeriod,
): AiUsageTokens | null {
  return item?.usageAvailable ? (item[period] ?? null) : null;
}
/** Claude / DSH 的 input 不含缓存；Codex 的 input 已含缓存。分母先统一口径。 */
export function cacheShare(item: AiUsageItem, period: AiPeriod) {
  const usage = usageFor(item, period);
  if (!usage) return null;
  const exclusive =
    item.inputIncludesCached === false ||
    (item.inputIncludesCached == null && CACHE_EXCLUSIVE_INPUT.has(item.key));
  const input = exclusive
    ? usage.input + usage.cached + usage.cacheWrite
    : usage.input;
  return input > 0
    ? {
        cached: usage.cached,
        input,
        percent: Math.min(100, (usage.cached / input) * 100),
      }
    : null;
}
export function localDate(date: Date) {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}
export function trendDays(
  items: AiUsageItem[],
  count: 7 | 30,
  now = new Date(),
) {
  return Array.from({ length: count }, (_, index) => {
    const date = new Date(
      now.getFullYear(),
      now.getMonth(),
      now.getDate() - count + index + 1,
    );
    const key = localDate(date);
    const values = items
      .filter((item) => item.usageAvailable)
      .map((item) => ({
        key: item.key,
        total: item.daily?.find((day) => day.date === key)?.usage.total ?? 0,
      }));
    return {
      date: key,
      label: `${date.getMonth() + 1}/${date.getDate()}`,
      values,
      total: values.reduce((sum, value) => sum + value.total, 0),
    };
  });
}
export function quotaView(
  quota: AiRateWindow,
  rate: AiRateLimit,
  scannedAt: number,
  now: number,
) {
  const used = Number.isFinite(quota.utilization)
    ? Math.max(0, quota.utilization * 100)
    : null;
  // 兼容旧快照时使用采样时刻，不用页面刷新时刻重启倒计时。
  const resetAt =
    quota.resetAt === undefined
      ? quota.resetAfterSeconds > 0
        ? scannedAt + quota.resetAfterSeconds
        : 0
      : quota.resetAt;
  const resetPassed =
    (resetAt != null && resetAt > 0 && resetAt <= now) ||
    (quota.resetAt === undefined && quota.resetAfterSeconds === 0);
  const stale =
    !rate.capturedAt ||
    now - rate.capturedAt > 1800 ||
    rate.capturedAt > now + 60;
  const tone: QuotaRow["tone"] =
    stale || resetPassed || used === null
      ? "muted"
      : used >= 90
        ? "danger"
        : used >= 70
          ? "warning"
          : "good";
  return {
    used,
    remaining: used === null ? null : Math.max(0, 100 - used),
    seconds: resetAt ? Math.max(0, resetAt - now) : null,
    resetAt,
    stale,
    resetPassed,
    tone,
  };
}
/** 窗口标签：row 供多账号/旧时间窗统一列表用，card 供单窗口卡片用。 */
export function quotaWindowLabel(
  window: AiRateWindow,
  slot: string,
  style: "row" | "card",
): string {
  const seconds = window.limitWindowSeconds;
  // row 的 slot 是兜底标签（如“额度窗口 1”），card 的 slot 是窗口名（如“主要”“次要”）。
  // 只有恰好 7 天才算“周”：更长的窗口按天报，否则 14 天窗口会被显示成“周额度”。
  if (seconds === 604800) return style === "card" ? "周额度" : "每周";
  if (seconds >= 86400 && seconds % 86400 === 0)
    return `${seconds / 86400} 天${style === "card" ? "额度" : ""}`;
  if (seconds > 0 && seconds % 3600 === 0)
    return style === "card"
      ? `${seconds / 3600}h 额度`
      : `${seconds / 3600} 小时`;
  if (style === "card") return `${slot}额度`;
  if (seconds > 0 && seconds % 60 === 0) return `${seconds / 60} 分钟`;
  return slot;
}
/** 额度条的一行数据；渲染一律走 QuotaBar，取数只在这里。 */
export interface QuotaRow {
  id: string;
  /** 列表风格窗口名：多账号指标与旧时间窗，HUD 与指标行使用。 */
  label: string;
  /** 卡片风格窗口名：统计页单窗口卡片（含「主要 / 次要」兜底）。 */
  cardLabel: string;
  /** 紧凑条窗口名：工具卡不显示窗口名，回退成「额度」。 */
  barLabel: string;
  /** 窗口名兜底：card 的 aria 文本用「主要 / 次要」。 */
  slot: string;
  account: string;
  plan: string;
  capturedAt: number;
  remainingLabel: string | null;
  /** 已解析的重置时刻；0 表示未知，null 表示后端明确没有。 */
  resetAt: number | null;
  /** 原始使用率百分比：可能超过 100（超额），也可能非有限（未知）。 */
  percent: number;
  used: number | null;
  remaining: number | null;
  seconds: number | null;
  stale: boolean;
  resetPassed: boolean;
  tone: "good" | "warning" | "danger" | "muted";
}
/** 多账号指标与旧版时间窗使用同一套采样、重置和告警规则。 */
export function quotaRows(
  item: AiUsageItem | undefined,
  now: number,
): QuotaRow[] {
  if (!item) return [];
  if (item.rate.metrics?.length)
    return item.rate.metrics.map((metric, index) => {
      const view = quotaView(
        {
          utilization: metric.usedPercent / 100,
          resetAt: metric.resetAt,
          resetAfterSeconds: 0,
          limitWindowSeconds: 0,
        },
        { ...item.rate, capturedAt: metric.capturedAt },
        item.scannedAt,
        now,
      );
      const label = metric.label || "额度窗口";
      return {
        id: `${metric.accountLabel}:${metric.label}:${index}`,
        label,
        cardLabel: label,
        barLabel: label,
        slot: label,
        account: metric.accountLabel,
        plan: metric.plan,
        capturedAt: metric.capturedAt,
        remainingLabel: metric.remainingLabel,
        percent: metric.usedPercent,
        ...view,
        // 后端给出剩余百分比时以后端为准，避免和工具卡片的读数不一致。
        remaining: metric.remainingPercent ?? view.remaining,
      };
    });
  return [item.rate.primary, item.rate.secondary].flatMap((window, index) => {
    if (!window) return [];
    const slot = index === 0 ? "主要" : "次要";
    return [
      {
        id: `window:${index}`,
        label: quotaWindowLabel(window, `额度窗口 ${index + 1}`, "row"),
        cardLabel: quotaWindowLabel(window, slot, "card"),
        barLabel: quotaWindowLabel(window, "", "card"),
        slot,
        account: "",
        plan: "",
        capturedAt: item.rate.capturedAt ?? 0,
        remainingLabel: null,
        percent: window.utilization * 100,
        ...quotaView(window, item.rate, item.scannedAt, now),
      },
    ];
  });
}
/** 重置倒计时文案：long 用于 HUD/统计页，card 用于统计页单窗口卡片，short 用于紧凑条。 */
export function quotaResetText(
  seconds: number | null,
  resetPassed: boolean,
  style: "long" | "card" | "short",
): string {
  // 三种风格共用“已到期 / 未知 / 已过点”的判定，只有正常倒计时的措辞不同。
  if (resetPassed) return "重置时间已到，待新快照";
  if (seconds == null) return "重置时间未知";
  if (seconds <= 0) return "等待更新重置时间";
  if (style === "short") {
    // 分钟向下取整；不足一分钟仍显示 1m，避免出现“0m后重置”。
    const hours = Math.floor(seconds / 3600),
      minutes = Math.floor((seconds % 3600) / 60);
    return hours > 0
      ? `${hours}h${minutes > 0 ? `${minutes}m` : ""}后重置`
      : `${Math.max(1, minutes)}m后重置`;
  }
  const minutes = Math.max(1, Math.ceil(seconds / 60)),
    hours = Math.floor(minutes / 60),
    days = Math.floor(hours / 24);
  // card：与 HUD 一样给出“天”档，否则 7 天窗口会显示成“168 小时”。
  if (style === "card")
    return `${[
      days ? `${days} 天` : "",
      hours % 24 ? `${hours % 24} 小时` : "",
      minutes % 60 ? `${minutes % 60} 分钟` : "",
    ]
      .filter(Boolean)
      .join(" ")}后重置`;
  return days
    ? `${days} 天${hours % 24 ? ` ${hours % 24} 小时` : ""}后重置`
    : hours
      ? `${hours} 小时${minutes % 60 ? ` ${minutes % 60} 分钟` : ""}后重置`
      : `${minutes} 分钟后重置`;
}
/** HUD 只关心“多久后重置”，不看 resetPassed，保留旧入口。 */
export function resetCountdown(seconds: number | null) {
  return quotaResetText(seconds, false, "long");
}
export function sessionState(session: AiSession, now: number) {
  return ["running", "asking"].includes(session.state) &&
    now - session.lastActivityAt >= 1800
    ? "unknown"
    : session.state;
}
export function recentSessions(items: AiUsageItem[], now: number) {
  const unique = new Map<string, AiSession>();
  for (const item of items)
    for (const session of item.recentSessions ?? []) {
      // 数据源决定工具归属；模型名、同名项目和重用的会话 ID 都不能改变来源。
      const tool = item.key ?? session.tool;
      const id =
        tool && !session.id.startsWith(`${tool}:`)
          ? `${tool}:${session.id}`
          : session.id;
      const old = unique.get(id);
      if (
        session.lastActivityAt >= now - 30 * 86400 &&
        (!old || old.lastActivityAt < session.lastActivityAt)
      )
        unique.set(id, { ...session, id, tool });
    }
  return [...unique.values()].sort(
    (a, b) => b.lastActivityAt - a.lastActivityAt || a.id.localeCompare(b.id),
  );
}
