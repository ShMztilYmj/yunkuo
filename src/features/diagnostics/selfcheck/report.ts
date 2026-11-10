import {
  SIGNAL_NAMES,
  SIGNAL_WEIGHTS,
  emojiStyleScore,
  scoreCnBrowser,
  scoreCnDevice,
  scoreEmojiVendor,
  scoreFontHits,
  scoreVendorFontHits,
  signalVerdict,
  type DetectOutcome,
  type SignalId,
  type SignalResult,
} from "../signals/web";

/**
 * 指纹浏览器回传的原始观测。字段一律宽松解析：页面在另一个进程里跑，
 * 任何一项缺失都只影响对应卡片，不能拖垮整页。
 */
export interface SelfcheckReport {
  at: number;
  ua: string;
  brands: string;
  platform: string;
  languages: string[];
  timezone: string;
  locale: string;
  offset: number;
  hardwareConcurrency: number | null;
  deviceMemory: number | null;
  screen: number[];
  fonts: { sc: string[]; tc: string[]; vendor: string[] };
  emojiFlag: boolean | null;
  webgl: string;
  webdriver: boolean;
}

function text(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function number(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function list(value: unknown, limit = 32): string[] {
  if (!Array.isArray(value)) return [];
  return value
    .filter((item): item is string => typeof item === "string")
    .slice(0, limit);
}

export function parseSelfcheckReport(value: unknown): SelfcheckReport | null {
  if (!value || typeof value !== "object") return null;
  const raw = value as Record<string, unknown>;
  const at = number(raw.at);
  if (at == null) return null;
  const fonts =
    raw.fonts && typeof raw.fonts === "object"
      ? (raw.fonts as Record<string, unknown>)
      : {};
  return {
    at,
    ua: text(raw.ua),
    brands: text(raw.brands),
    platform: text(raw.platform),
    languages: list(raw.languages),
    timezone: text(raw.timezone),
    locale: text(raw.locale),
    offset: number(raw.offset) ?? 0,
    hardwareConcurrency: number(raw.hardwareConcurrency),
    deviceMemory: number(raw.deviceMemory),
    screen: Array.isArray(raw.screen)
      ? raw.screen
          .filter((item): item is number => typeof item === "number")
          .slice(0, 4)
      : [],
    fonts: {
      sc: list(fonts.sc),
      tc: list(fonts.tc),
      vendor: list(fonts.vendor),
    },
    emojiFlag: typeof raw.emojiFlag === "boolean" ? raw.emojiFlag : null,
    webgl: text(raw.webgl),
    webdriver: raw.webdriver === true,
  };
}

function row(id: SignalId, outcome: DetectOutcome): SignalResult {
  const weight = SIGNAL_WEIGHTS.find((item) => item.id === id)?.weight ?? 0;
  return {
    id,
    name: SIGNAL_NAMES[id],
    raw: outcome.raw,
    score: outcome.score,
    contribution: Math.round(outcome.score * weight),
    band: signalVerdict(outcome.score),
  };
}

/**
 * 把回传的原始观测换算成与主机卡片同形的五行信号。
 *
 * 计分函数与 `signals/web.ts` 完全共用：字体、国产浏览器/设备、Emoji 都只
 * 换「观测来源」，不换规则，避免两栏各算一套。
 */
export function selfcheckSignals(report: SelfcheckReport): SignalResult[] {
  const probe = `${report.platform} ${report.ua}`.toLowerCase();
  const vendor = scoreEmojiVendor(probe);
  const flag = report.emojiFlag;
  const flagText = flag == null ? "" : flag ? " · 国旗可渲染" : " · 国旗不渲染";
  const browser = scoreCnBrowser(`${report.ua} ${report.brands}`);
  const device = scoreCnDevice(`${report.ua} ${report.platform}`);
  return [
    row("fonts", scoreFontHits(report.fonts.sc, report.fonts.tc)),
    row("vendorFonts", scoreVendorFontHits(report.fonts.vendor)),
    row("cnBrowser", { raw: browser.name ?? "未检测到", score: browser.score }),
    row("deviceVendor", {
      raw: device.name ?? "未检测到",
      score: device.score,
    }),
    row("emoji", {
      raw: `${vendor.vendor} 风格${flagText}`,
      score: emojiStyleScore(vendor.score, flag),
    }),
  ];
}

/** 一行环境摘要：语言 · 时区 · 核数 · 引擎版本 */
export function selfcheckSummary(report: SelfcheckReport): string {
  const language = report.languages[0] || "语言未知";
  const cores = report.hardwareConcurrency
    ? `${report.hardwareConcurrency} 核`
    : "核数未知";
  const engine = /(?:Edg|Chrome|Chromium)\/(\d+)/.exec(report.ua);
  return [
    language,
    report.timezone || "时区未知",
    cores,
    engine ? `Chrome ${engine[1]}` : "引擎未知",
  ].join(" · ");
}
