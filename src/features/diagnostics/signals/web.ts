/**
 * 环境风险信号定义 + 打分逻辑。
 *
 * 权重比例沿用既定的检测口径，主要改动：去掉站点专用部分；
 * 移除 WebRTC 出口测试项，其权重按比例摊给其余信号。
 */
export type SignalId =
  | "timezone"
  | "timezoneOffset"
  | "language"
  | "intlLocale"
  | "fonts"
  | "vendorFonts"
  | "cnBrowser"
  | "deviceVendor"
  | "emoji";

export interface DetectOutcome {
  raw: string;
  score: number; // 0..1
  status?: "unknown" | "candidate";
}

export interface SignalResult {
  id: SignalId;
  name: string;
  raw: string;
  score: number; // 0..1
  contribution: number; // = round(score * weight)
  band: RiskBand;
  status?: DetectOutcome["status"];
}

export type RiskBand = "low" | "medium" | "high";

export const SIGNAL_NAMES: Record<SignalId, string> = {
  timezone: "系统时区",
  language: "浏览器语言",
  fonts: "中文字体",
  vendorFonts: "国产字体",
  cnBrowser: "国产浏览器",
  deviceVendor: "国产设备",
  intlLocale: "Intl 区域",
  timezoneOffset: "时区偏移",
  emoji: "Emoji 渲染",
};

export const CN_TIMEZONES = [
  "Asia/Shanghai",
  "Asia/Urumqi",
  "Asia/Chongqing",
  "Asia/Chungking",
  "Asia/Harbin",
  "Asia/Kashgar",
];
export const CLAUDE_TIMEZONES = ["Asia/Shanghai", "Asia/Urumqi"];
/** 港澳在 Anthropic 受限名单内 → 部分风险；台湾完全支持 → 不计分 */
export const GREATER_CN_TIMEZONES = ["Asia/Hong_Kong", "Asia/Macau"];

export const FONTS_SC = [
  "Microsoft YaHei",
  "Microsoft YaHei UI",
  "SimSun",
  "NSimSun",
  "SimHei",
  "KaiTi",
  "FangSong",
  "DengXian",
  "PingFang SC",
  "Hiragino Sans GB",
  "STHeiti",
  "STSong",
  "Songti SC",
  "Source Han Sans CN",
  "Source Han Sans SC",
  "Noto Sans CJK SC",
  "Noto Serif CJK SC",
  "WenQuanYi Micro Hei",
  "WenQuanYi Zen Hei",
];
export const FONTS_TC = [
  "Microsoft JhengHei",
  "PMingLiU",
  "MingLiU",
  "DFKai-SB",
  "PingFang TC",
  "PingFang HK",
  "Source Han Sans TW",
  "Noto Sans CJK TC",
];

/** 国产厂商/软件字体，命中即为强信号 */
export const FONTS_CN_VENDOR = [
  "MiSans",
  "MIUI",
  "HarmonyOS Sans SC",
  "HarmonyOS Sans",
  "HONOR Sans",
  "OPPO Sans",
  "vivo Sans",
  "Alibaba PuHuiTi",
  "Alibaba Sans",
  "DingTalk JinBuTi",
  "Douyin Sans",
  "HYQiHei",
  "FZShuSong-Z01S",
  "FZKai-Z03S",
  "FZHei-B01S",
  "FZFangSong-Z02S",
];

const CN_BROWSER_PATTERNS: Array<[RegExp, string]> = [
  [/micromessenger|wxwork/i, "WeChat"],
  [/mqqbrowser|qqbrowser|\bqq\//i, "QQ Browser"],
  [/quark/i, "Quark"],
  [/ucbrowser|ucweb/i, "UC Browser"],
  [/baiduboxapp|bidubrowser|baidubrowser/i, "Baidu"],
  [/miuibrowser|xiaomi\/|mibrowser/i, "Mi Browser"],
  [/huaweibrowser/i, "Huawei Browser"],
  [/heytapbrowser|oppobrowser/i, "HeyTap (OPPO)"],
  [/vivobrowser/i, "vivo Browser"],
  [/sogoumobilebrowser|\bmetasr\b|\bse 2\.x\b/i, "Sogou"],
  [/maxthon/i, "Maxthon"],
  [/360se|360ee|qihoobrowser|\bqhbrowser\b/i, "360 Browser"],
  [/2345explorer|2345browser/i, "2345"],
  [/lbbrowser/i, "Liebao"],
  [/theworld/i, "TheWorld"],
  [/aweme|bytedancewebview|newsarticle|toutiaomicroapp/i, "Douyin / Toutiao"],
  [/alipayclient/i, "Alipay"],
  [/dingtalk/i, "DingTalk"],
  [/weibo/i, "Weibo"],
  [/xiaohongshu|xhsminiapp/i, "Xiaohongshu"],
  [/\bbilibili\b/i, "Bilibili"],
];

const CN_DEVICE_PATTERNS: Array<[RegExp, string, number]> = [
  [/harmonyos|openharmony/i, "HarmonyOS", 1],
  [/huawei|\bhonor\b/i, "Huawei / Honor", 0.8],
  [/meizu/i, "Meizu", 0.8],
  [/nubia|\bzte\b/i, "ZTE / nubia", 0.7],
  [/xiaomi|redmi|\bpoco\b|\bm2\d{3}[a-z0-9]+\b/i, "Xiaomi", 0.6],
  [/oppo|\bpd[a-z]m\d{2}\b/i, "OPPO", 0.6],
  [/vivo|\bv2\d{3}[a-z]{1,2}\b/i, "vivo", 0.6],
  [/realme|\brmx\d{4}\b/i, "realme", 0.6],
  [/oneplus/i, "OnePlus", 0.6],
  [/\blenovo\b|\bzuk\b/i, "Lenovo", 0.5],
];

function getTimezone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "";
  } catch {
    return "";
  }
}

export function scoreTimezone(tz: string): number {
  if (CLAUDE_TIMEZONES.includes(tz) || CN_TIMEZONES.includes(tz)) return 1;
  if (GREATER_CN_TIMEZONES.includes(tz)) return 0.6;
  return 0;
}

function normLangs(): string[] {
  const list =
    navigator.languages && navigator.languages.length
      ? navigator.languages
      : [navigator.language];
  return list.map((l) => (l || "").toLowerCase());
}

export function scoreLanguages(langs: string[]): number {
  const list = langs.map((l) => (l || "").toLowerCase()).filter(Boolean);
  const isTW = (l: string) =>
    l.startsWith("zh-tw") || (l.includes("hant") && l.includes("tw"));
  const isHKMO = (l: string) => l.startsWith("zh-hk") || l.startsWith("zh-mo");
  const isTrad = (l: string) => isTW(l) || isHKMO(l) || l.includes("hant");
  const firstTrad = list.findIndex(isTrad);
  const isHansCN = (l: string, i: number) =>
    l.startsWith("zh-cn") ||
    l.includes("hans") ||
    (l === "zh" && (firstTrad === -1 || i < firstTrad));

  const kept = list
    .map((l, i) => ({ l, i }))
    .filter(
      ({ l, i }) =>
        !isTW(l) && !(l === "zh" && firstTrad !== -1 && i > firstTrad),
    );

  const primary = kept[0];
  if (!primary) return 0;
  if (isHansCN(primary.l, primary.i)) return 1;
  if (isHKMO(primary.l) || primary.l.includes("hant")) return 0.5;
  if (kept.some(({ l, i }) => isHansCN(l, i))) return 0.7;
  if (kept.some(({ l }) => l.startsWith("zh"))) return 0.4;
  return 0;
}

/**
 * 字体可用性探针：同一段文字在「基准字体」和「候选字体 + 基准」下量宽。
 * 只引用参数，可整体序列化到指纹浏览器自检页里执行（见 selfcheck/probe.ts）。
 */
export function isFontAvailable(
  font: string,
  ctx: CanvasRenderingContext2D,
): boolean {
  const testString = "中文字体检测ABCabc012";
  const size = "72px";
  const bases = ["monospace", "sans-serif", "serif"];
  return bases.some((base) => {
    ctx.font = `${size} ${base}`;
    const baseWidth = ctx.measureText(testString).width;
    ctx.font = `${size} "${font}", ${base}`;
    const testWidth = ctx.measureText(testString).width;
    return Math.abs(testWidth - baseWidth) > 0.5;
  });
}

function detectTimezone(): DetectOutcome {
  const tz = getTimezone();
  return { raw: tz || "unknown", score: scoreTimezone(tz) };
}

function detectTimezoneOffset(): DetectOutcome {
  const offset = new Date().getTimezoneOffset();
  const utcHours = -offset / 60;
  const sign = utcHours >= 0 ? "+" : "-";
  const raw = `UTC${sign}${Math.abs(utcHours)}`;
  return { raw, score: offset === -480 ? 0.7 : 0 };
}

function detectLanguage(): DetectOutcome {
  const langs = normLangs();
  return { raw: langs.join(", ") || "unknown", score: scoreLanguages(langs) };
}

function detectIntlLocale(): DetectOutcome {
  let locale = "";
  try {
    locale = Intl.DateTimeFormat().resolvedOptions().locale || "";
  } catch {
    locale = "";
  }
  const l = locale.toLowerCase();
  let score = 0;
  if (l.startsWith("zh-cn") || l.includes("hans") || l === "zh") score = 1;
  else if (l.startsWith("zh") && !l.startsWith("zh-tw")) score = 0.5;
  return { raw: locale || "unknown", score };
}

/** 中文字体命中计分；自检页把命中的字体名回传后复用同一套计分 */
export function scoreFontHits(sc: string[], tc: string[]): DetectOutcome {
  let score = 0;
  if (sc.length >= 1) score = Math.min(1, 0.75 + 0.08 * sc.length);
  else if (tc.length >= 1) score = 0.2;

  const hit = [...sc, ...tc];
  const raw = hit.length
    ? hit.slice(0, 4).join(", ") + (hit.length > 4 ? "…" : "")
    : "未检测到";
  return { raw, score };
}

function detectFonts(): DetectOutcome {
  const canvas = document.createElement("canvas");
  const ctx = canvas.getContext("2d");
  if (!ctx) return { raw: "canvas 不可用", score: 0 };

  return scoreFontHits(
    FONTS_SC.filter((f) => isFontAvailable(f, ctx)),
    FONTS_TC.filter((f) => isFontAvailable(f, ctx)),
  );
}

/** 国产厂商字体命中计分 */
export function scoreVendorFontHits(hits: string[]): DetectOutcome {
  const score = hits.length >= 2 ? 1 : hits.length === 1 ? 0.8 : 0;
  const raw = hits.length
    ? hits.slice(0, 3).join(", ") + (hits.length > 3 ? "…" : "")
    : "未检测到";
  return { raw, score };
}

function detectVendorFonts(): DetectOutcome {
  const canvas = document.createElement("canvas");
  const ctx = canvas.getContext("2d");
  if (!ctx) return { raw: "canvas 不可用", score: 0 };

  return scoreVendorFontHits(
    FONTS_CN_VENDOR.filter((f) => isFontAvailable(f, ctx)),
  );
}

type UADataBrand = { brand: string; version: string };
interface UAData {
  brands?: UADataBrand[];
  getHighEntropyValues?: (hints: string[]) => Promise<Record<string, unknown>>;
}

function uaData(): UAData | undefined {
  return (navigator as Navigator & { userAgentData?: UAData }).userAgentData;
}

export function scoreCnBrowser(probe: string): {
  name: string | null;
  score: number;
} {
  for (const [re, name] of CN_BROWSER_PATTERNS) {
    if (re.test(probe)) return { name, score: 1 };
  }
  return { name: null, score: 0 };
}

function detectCnBrowser(): DetectOutcome {
  const brands = (uaData()?.brands ?? []).map((b) => b.brand).join(" ");
  const { name, score } = scoreCnBrowser(`${navigator.userAgent} ${brands}`);
  return { raw: name ?? "未检测到", score };
}

export function scoreCnDevice(probe: string): {
  name: string | null;
  score: number;
} {
  for (const [re, name, score] of CN_DEVICE_PATTERNS) {
    if (re.test(probe)) return { name, score };
  }
  return { name: null, score: 0 };
}

async function detectDeviceVendor(): Promise<DetectOutcome> {
  let extra = "";
  try {
    const high = await uaData()?.getHighEntropyValues?.([
      "model",
      "platform",
      "platformVersion",
    ]);
    if (high)
      extra = ` ${String(high.model ?? "")} ${String(high.platform ?? "")}`;
  } catch {
    /* hints 被拒绝时回退纯 UA */
  }
  const { name, score } = scoreCnDevice(`${navigator.userAgent}${extra}`);
  return { raw: name ?? "未检测到", score };
}

export function scoreEmojiVendor(probe: string): {
  vendor: string;
  score: number;
} {
  const p = probe.toLowerCase();

  let vendor = "Unknown";
  if (/iphone|ipad|ipod|mac/.test(p)) vendor = "Apple";
  else if (/android/.test(p)) vendor = "Google";
  else if (/win/.test(p)) vendor = "Microsoft";
  else if (/cros/.test(p)) vendor = "Google";
  else if (/linux/.test(p)) vendor = "Linux / Other";

  const vendorScore: Record<string, number> = {
    Apple: 0.25,
    Microsoft: 0.4,
    Google: 0.35,
    "Linux / Other": 0.5,
    Unknown: 0.4,
  };

  return { vendor, score: vendorScore[vendor] ?? 0.4 };
}

/**
 * 国旗 emoji 像素探针：Windows 的 Segoe UI Emoji 不支持国旗渲染（显示字母组合），
 * Apple / Google / Noto 风格则渲染出彩色旗帜。
 * 只引用 DOM，可整体序列化到指纹浏览器自检页里执行（见 selfcheck/probe.ts）。
 */
export function emojiFlagProbe(): boolean | null {
  try {
    const c = document.createElement("canvas");
    c.width = c.height = 22;
    const x = c.getContext("2d", { willReadFrequently: true });
    if (!x) return null;
    x.textBaseline = "top";
    x.font = "18px sans-serif";
    x.fillText("\u{1F1E8}\u{1F1F3}", 0, 0); // 🇨🇳
    const d = x.getImageData(0, 0, 22, 22).data;
    let red = 0;
    let colored = 0;
    for (let i = 0; i < d.length; i += 4) {
      const r = d[i],
        g = d[i + 1],
        b = d[i + 2],
        a = d[i + 3];
      if (a > 10) {
        if (r > 140 && g < 110 && b < 110) red++;
        if (Math.max(r, g, b) - Math.min(r, g, b) > 45) colored++;
      }
    }
    return red > 2 || colored > 8;
  } catch {
    return null;
  }
}

/** 国旗是否渲染 + 厂商分 → Emoji 风格风险；两边共用同一套判定 */
export function emojiStyleScore(
  vendorScore: number,
  flagRendered: boolean | null,
): number {
  return flagRendered === null ? vendorScore : flagRendered ? 0.25 : 0.4;
}

export function scoreEmojiStyle(probe: string): {
  vendor: string;
  flagRendered: boolean | null;
  score: number;
} {
  const { vendor, score: vendorScore } = scoreEmojiVendor(probe);
  const flagRendered = emojiFlagProbe();
  // 国旗不渲染（Windows 风格）是轻微风险；渲染（彩色风格）为低风险
  return {
    vendor,
    flagRendered,
    score: emojiStyleScore(vendorScore, flagRendered),
  };
}

function detectEmoji(): DetectOutcome {
  const ua = (navigator.userAgent || "").toLowerCase();
  const platform = (navigator.platform || "").toLowerCase();
  const { vendor, flagRendered, score } = scoreEmojiStyle(`${platform} ${ua}`);
  const flag =
    flagRendered == null
      ? ""
      : flagRendered
        ? " · 国旗可渲染"
        : " · 国旗不渲染";
  return { raw: `${vendor} 风格${flag}`, score };
}

// 权重定义（总分为 100；原 WebRTC 项已移除，其 10 分按比例摊回其余信号）
export const SIGNAL_WEIGHTS: Array<{ id: SignalId; weight: number }> = [
  { id: "timezone", weight: 27 },
  { id: "language", weight: 20 },
  { id: "fonts", weight: 16 },
  { id: "vendorFonts", weight: 11 },
  { id: "cnBrowser", weight: 9 },
  { id: "deviceVendor", weight: 7 },
  { id: "intlLocale", weight: 4 },
  { id: "timezoneOffset", weight: 3 },
  { id: "emoji", weight: 3 },
];

const DETECTORS: Record<
  SignalId,
  () => DetectOutcome | Promise<DetectOutcome>
> = {
  timezone: detectTimezone,
  language: detectLanguage,
  fonts: detectFonts,
  vendorFonts: detectVendorFonts,
  cnBrowser: detectCnBrowser,
  deviceVendor: detectDeviceVendor,
  intlLocale: detectIntlLocale,
  timezoneOffset: detectTimezoneOffset,
  emoji: detectEmoji,
};

export function signalVerdict(score: number): RiskBand {
  if (score >= 0.6) return "high";
  if (score >= 0.25) return "medium";
  return "low";
}

/** 顺序运行全部信号并汇总 */
export async function runAllSignals(): Promise<{
  results: SignalResult[];
  total: number;
}> {
  const results: SignalResult[] = [];
  let total = 0;
  for (const { id, weight } of SIGNAL_WEIGHTS) {
    let outcome: DetectOutcome;
    try {
      outcome = await DETECTORS[id]();
    } catch {
      outcome = { raw: "—", score: 0 };
    }
    const contribution = Math.round(outcome.score * weight);
    const band = signalVerdict(outcome.score);
    total += contribution;
    results.push({
      id,
      name: SIGNAL_NAMES[id],
      raw: outcome.raw,
      score: outcome.score,
      contribution,
      band,
      ...(outcome.status ? { status: outcome.status } : {}),
    });
  }
  return { results, total: Math.min(100, total) };
}
