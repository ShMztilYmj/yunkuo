import type { LastScanSummary, LastScanSystem } from "../../services/api";

/** 历史结果超过这个时长只提示「时间较久」，不隐藏数据。 */
export const HISTORY_FRESH_MS = 6 * 60 * 60 * 1000;

/** 旧结论失效的原因；null 表示与当前环境看不出差别。 */
export type StaleReason = "config" | "system" | "age" | null;

export const STALE_TEXT: Record<Exclude<StaleReason, null>, string> = {
  config: "网络配置已变",
  system: "系统语言/时区已变",
  age: "时间较久",
};

/**
 * 只比较两边都读到的事实：拿不到当前系统状态时不下「已变」的结论，
 * 避免刚启动、面板还没读回 Windows 设置时闪一下「已失效」。
 */
export function systemChanged(
  last: LastScanSystem,
  now: LastScanSystem | null,
): boolean {
  if (!now) return false;
  if (last.timezone && now.timezone && last.timezone !== now.timezone)
    return true;
  if (last.culture && now.culture && last.culture !== now.culture) return true;
  return (
    last.languages.length > 0 &&
    now.languages.length > 0 &&
    last.languages.join("\u0000") !== now.languages.join("\u0000")
  );
}

/**
 * 历史结论的失效原因。优先级：配置 > 系统 > 时间。
 *
 * 这里刻意不看后端 `EnvironmentBasis`：它含进程内代次，重启后必然不相等，
 * 用它判定会让每次启动都显示「已失效」。
 */
export function staleReason(
  last: LastScanSummary,
  now: LastScanSystem | null,
  at: number = Date.now(),
): StaleReason {
  if (last.configChanged) return "config";
  if (systemChanged(last.system, now)) return "system";
  if (at - last.at > HISTORY_FRESH_MS) return "age";
  return null;
}

export function relativeTime(at: number, now: number = Date.now()): string {
  const minutes = Math.max(0, Math.floor((now - at) / 60_000));
  if (minutes < 1) return "刚刚";
  if (minutes < 60) return `${minutes} 分钟前`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} 小时前`;
  return `${Math.floor(hours / 24)} 天前`;
}
