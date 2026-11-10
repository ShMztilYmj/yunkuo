/**
 * 节点页的展示逻辑：延迟结果带时效、按地区归类、按需排序。
 *
 * 测速结果只放在内存里会随刷新丢失，用户切一次页面就要重测；
 * 这里落到 localStorage，并给每个样本一个时间戳，超过 TTL 视为未测。
 */

export const DELAY_TTL_MS = 10 * 60 * 1000;
const STORAGE_KEY = "yunkuo.proxy.delays.v1";

export interface DelaySample {
  delay: number;
  /** 测速时刻（epoch ms） */
  at: number;
}

export type DelayMap = Record<string, DelaySample>;

export type SortMode = "default" | "delay" | "name";

export type RegionKey =
  "HK" | "TW" | "JP" | "SG" | "US" | "KR" | "GB" | "DE" | "CA" | "AU" | "OTHER";

export interface Region {
  key: RegionKey;
  label: string;
}

/** 展示顺序：常见机房地区在前，其余归入「其他」。 */
export const REGIONS: Region[] = [
  { key: "HK", label: "香港" },
  { key: "TW", label: "台湾" },
  { key: "JP", label: "日本" },
  { key: "SG", label: "新加坡" },
  { key: "US", label: "美国" },
  { key: "KR", label: "韩国" },
  { key: "GB", label: "英国" },
  { key: "DE", label: "德国" },
  { key: "CA", label: "加拿大" },
  { key: "AU", label: "澳大利亚" },
  { key: "OTHER", label: "其他" },
];

/** 只有出现过的地区才进筛选器，避免固定列出十个空选项。 */
export function presentRegions(names: string[]): Region[] {
  const keys = new Set(names.map(regionOf));
  return REGIONS.filter((region) => keys.has(region.key));
}

export function regionLabel(key: RegionKey): string {
  return REGIONS.find((region) => region.key === key)?.label ?? "其他";
}

const REGION_RULES: [RegionKey, RegExp][] = [
  ["HK", /香港|港区|港區|hong\s*kong|(^|[^a-z])hk([^a-z]|$)/i],
  ["TW", /台湾|台灣|台北|taiwan|(^|[^a-z])tw([^a-z]|$)/i],
  ["JP", /日本|东京|東京|大阪|japan|tokyo|osaka|(^|[^a-z])jp([^a-z]|$)/i],
  ["SG", /新加坡|狮城|獅城|singapore|(^|[^a-z])sg([^a-z]|$)/i],
  [
    "US",
    /美国|美國|洛杉矶|洛杉磯|圣何塞|聖荷西|西雅图|西雅圖|纽约|紐約|硅谷|矽谷|united\s*states|america|los\s*angeles|san\s*jose|seattle|new\s*york|(^|[^a-z])(us|usa)([^a-z]|$)/i,
  ],
  ["KR", /韩国|韓國|首尔|首爾|korea|seoul|(^|[^a-z])kr([^a-z]|$)/i],
  [
    "GB",
    /英国|英國|伦敦|倫敦|united\s*kingdom|britain|london|(^|[^a-z])(uk|gb)([^a-z]|$)/i,
  ],
  [
    "DE",
    /德国|德國|法兰克福|法蘭克福|germany|frankfurt|(^|[^a-z])de([^a-z]|$)/i,
  ],
  ["CA", /加拿大|canada|toronto|vancouver|(^|[^a-z])ca([^a-z]|$)/i],
  [
    "AU",
    /澳大利亚|澳洲|悉尼|墨尔本|布里斯班|australia|sydney|melbourne|brisbane|(^|[^a-z])au([^a-z]|$)/i,
  ],
];

/** 从节点名猜地区；猜不出统一归入「其他」，不做二次推断。 */
export function regionOf(name: string): RegionKey {
  for (const [key, pattern] of REGION_RULES) {
    if (pattern.test(name)) return key;
  }
  return "OTHER";
}

/** 未过期才返回延迟；0 或负数按未测处理。 */
export function freshDelay(
  map: DelayMap,
  name: string,
  now = Date.now(),
): number | null {
  const sample = map[name];
  if (!sample || sample.delay <= 0) return null;
  if (now - sample.at > DELAY_TTL_MS) return null;
  return sample.delay;
}

export function sampleAge(
  map: DelayMap,
  name: string,
  now = Date.now(),
): number | null {
  const sample = map[name];
  return sample ? now - sample.at : null;
}

/** 丢弃过期样本，避免存储无限增长。 */
export function pruneDelays(map: DelayMap, now = Date.now()): DelayMap {
  const next: DelayMap = {};
  for (const [name, sample] of Object.entries(map)) {
    if (sample.delay > 0 && now - sample.at <= DELAY_TTL_MS)
      next[name] = sample;
  }
  return next;
}

export function loadDelays(): DelayMap {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) return {};
    const map: DelayMap = {};
    for (const [name, value] of Object.entries(
      parsed as Record<string, unknown>,
    )) {
      if (
        typeof value === "object" &&
        value !== null &&
        typeof (value as DelaySample).delay === "number" &&
        typeof (value as DelaySample).at === "number"
      ) {
        map[name] = {
          delay: (value as DelaySample).delay,
          at: (value as DelaySample).at,
        };
      }
    }
    return pruneDelays(map);
  } catch {
    return {};
  }
}

export function saveDelays(map: DelayMap): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(pruneDelays(map)));
  } catch {
    // 存储被禁用时只影响跨刷新保留，不影响当前会话。
  }
}

/** 默认保持内核顺序；延迟排序把未测/过期的放最后，名称排序按中文习惯。 */
export function sortNodes(
  names: string[],
  delays: DelayMap,
  mode: SortMode,
  now = Date.now(),
): string[] {
  const list = [...names];
  if (mode === "name")
    return list.sort((a, b) => a.localeCompare(b, "zh-Hans-CN"));
  if (mode === "delay") {
    return list.sort((a, b) => {
      const left = freshDelay(delays, a, now);
      const right = freshDelay(delays, b, now);
      if (left === null && right === null) return 0;
      if (left === null) return 1;
      if (right === null) return -1;
      return left - right;
    });
  }
  return list;
}

export function filterNodes(
  names: string[],
  region: RegionKey | "all",
): string[] {
  if (region === "all") return names;
  return names.filter((name) => regionOf(name) === region);
}
