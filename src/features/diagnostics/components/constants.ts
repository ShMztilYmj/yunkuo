import type { SignalResult } from "../signals/web";

/** 信任评分档位配色：文字色与环形进度色 */
export const BAND_COLORS = {
  high: { text: "text-emerald-400", ring: "#34d399" },
  medium: { text: "text-amber-400", ring: "#fbbf24" },
  low: { text: "text-red-400", ring: "#f87171" },
};

/** 归入「系统级检测」卡片的信号 id */
export const SYSTEM_SIGNAL_IDS = new Set<SignalResult["id"]>([
  "language",
  "intlLocale",
  "timezone",
  "timezoneOffset",
]);
