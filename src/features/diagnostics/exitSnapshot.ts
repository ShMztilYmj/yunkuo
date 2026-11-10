/**
 * 出口通道快照：把上次测到的出口 IP 与 IP 情报留在本机。
 *
 * 为什么要留：IP 情报要打 RIR / PeeringDB / 代理画像库，单轮 10-20 秒且经常部分失败；
 * 重启软件后卡片又会从「待检测」重新来一遍。这里只缓存展示字段，首屏直接给上次结果，
 * 等本轮检测回来再覆盖。
 *
 * 存 localStorage 而不是 environment/ 目录：它是界面缓存，过期就该丢，丢了也只影响
 * 首屏显示。启动门控仍然只认本轮检测（与 last-scan.json 同一原则），这里没有
 * finished / native / 任何放行凭据，喂不进 `computeGate`。
 */

import type { IpIntel } from "../../services/api";
import type { ExitResult, GeoInfo } from "./signals/exit";

/** v1：字段或语义变化时换键，旧快照自然失效。 */
const SNAPSHOT_VERSION = 1;
const STORAGE_KEY = "yunkuo.exit-snapshot.v1";

/** 超过一天就当作过期：旧 IP 不再冒充当前出口，避免误判。 */
export const EXIT_SNAPSHOT_TTL_MS = 24 * 60 * 60 * 1000;
/** 允许的时钟回拨：本地时间被调早时不要把快照判成「未来」。 */
const CLOCK_SKEW_MS = 5 * 60 * 1000;
/** 粗上限：情报字段多，但不该让一条快照撑爆 localStorage。 */
const MAX_CHARS = 32 * 1024;

export interface ExitSnapshot {
  /** unix ms */
  at: number;
  /** 快照对应的出口 IP；与当前出口不同时不再复用 */
  ip: string;
  countryAllowed: boolean | null;
  exit: ExitResult | null;
  intel: IpIntel | null;
}

const VERDICTS = ["g", "y", "r", "n"] as const;
const EXIT_STATES = ["ok", "fail", "loading"] as const;
const ASN_CLASSES = ["dc", "res", "unknown"] as const;
const INTEL_STATES = ["ok", "partial", "fail"] as const;
const ASN_TYPES = ["hosting", "isp", "unknown"] as const;
const NETWORK_CLASSES = ["dc", "res", "unknown"] as const;
const RPKI_VALUES = ["valid", "invalid", "unknown"] as const;
const BLACKLIST_STATES = ["listed", "clean", "unavailable"] as const;
const IPAPIIS_STATES = ["ok", "failed", "off"] as const;

type Raw = Record<string, unknown>;

function raw(value: unknown): Raw | null {
  return typeof value === "object" && value !== null ? (value as Raw) : null;
}

function str(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function num(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function bool(value: unknown): boolean | null {
  return typeof value === "boolean" ? value : null;
}

function oneOf<T extends string>(
  value: unknown,
  allowed: readonly T[],
): T | null {
  if (typeof value !== "string") return null;
  return (allowed as readonly string[]).includes(value) ? (value as T) : null;
}

function textList(value: unknown, limit: number): string[] {
  if (!Array.isArray(value)) return [];
  return value
    .filter((item): item is string => typeof item === "string" && !!item.trim())
    .slice(0, limit);
}

function normalizeGeo(value: unknown): GeoInfo | null {
  const r = raw(value);
  const ip = r ? str(r.ip) : null;
  if (!r || !ip) return null;
  return {
    ip,
    cc: (str(r.cc) ?? "").toUpperCase(),
    country: str(r.country) ?? "",
    city: str(r.city) ?? "",
    asn: str(r.asn) ?? "",
    isp: str(r.isp) ?? "",
    tzId: str(r.tzId) ?? "",
    src: str(r.src) ?? "",
  };
}

/** 逐字段重建，不信任存储里的类型：坏字段退化成 null，不让 UI 读到 "undefined"。 */
export function normalizeExit(value: unknown): ExitResult | null {
  const r = raw(value);
  const state = r ? oneOf(r.state, EXIT_STATES) : null;
  const at = r ? num(r.at) : null;
  if (!r || !state || at === null) return null;
  return {
    state,
    primary: normalizeGeo(r.primary),
    ip6: str(r.ip6),
    v6Verdict: oneOf(r.v6Verdict, VERDICTS) ?? "n",
    v6Msg: str(r.v6Msg) ?? "",
    triangleVerdict: oneOf(r.triangleVerdict, VERDICTS) ?? "n",
    triangleMsg: str(r.triangleMsg) ?? "",
    asnClass: oneOf(r.asnClass, ASN_CLASSES) ?? "unknown",
    at,
  };
}

export function normalizeIntel(value: unknown): IpIntel | null {
  const r = raw(value);
  const ip = r ? str(r.ip) : null;
  if (!r || !ip) return null;
  return {
    ip,
    state: oneOf(r.state, INTEL_STATES) ?? "partial",
    error: str(r.error),
    asn: num(r.asn),
    asnOrg: str(r.asnOrg),
    asnName: str(r.asnName),
    isp: str(r.isp),
    org: str(r.org),
    asnType: oneOf(r.asnType, ASN_TYPES),
    networkClass: oneOf(r.networkClass, NETWORK_CLASSES),
    rir: str(r.rir),
    rirCountry: str(r.rirCountry),
    prefix: str(r.prefix),
    asnIpv4Count: num(r.asnIpv4Count),
    asnRegistered: str(r.asnRegistered),
    isBogon: bool(r.isBogon),
    reverseDns: str(r.reverseDns),
    mobile: bool(r.mobile),
    hosting: bool(r.hosting),
    proxy: bool(r.proxy),
    vpn: bool(r.vpn),
    tor: bool(r.tor),
    proxyType: str(r.proxyType),
    riskScore: num(r.riskScore),
    rpki: oneOf(r.rpki, RPKI_VALUES),
    peeringdbTraffic: str(r.peeringdbTraffic),
    peeringdbType: str(r.peeringdbType),
    peeringdbRatio: str(r.peeringdbRatio),
    peeringdbScope: str(r.peeringdbScope),
    peeringdbPrefixes4: num(r.peeringdbPrefixes4),
    peeringdbName: str(r.peeringdbName),
    torExit: bool(r.torExit),
    torSource: str(r.torSource),
    blacklistState: oneOf(r.blacklistState, BLACKLIST_STATES) ?? "unavailable",
    blacklists: textList(r.blacklists, 8),
    blacklistNote: str(r.blacklistNote),
    abuserScore: str(r.abuserScore),
    isAbuser: bool(r.isAbuser),
    isCrawler: bool(r.isCrawler),
    abuseEmail: str(r.abuseEmail),
    ipapiisState: oneOf(r.ipapiisState, IPAPIIS_STATES) ?? "off",
    sources: textList(r.sources, 8),
  };
}

/**
 * 解析落盘内容：版本、时间、IP 任一不对就当没有快照。
 * 情报与出口的 IP 必须与快照 IP 一致，避免换节点后拿旧画像描述新出口。
 */
export function parseExitSnapshot(
  value: unknown,
  now: number = Date.now(),
): ExitSnapshot | null {
  const r = raw(value);
  if (!r || num(r.v) !== SNAPSHOT_VERSION) return null;
  const at = num(r.at);
  const ip = str(r.ip);
  if (at === null || !ip) return null;
  if (at > now + CLOCK_SKEW_MS) return null;
  if (now - at > EXIT_SNAPSHOT_TTL_MS) return null;
  const exit = normalizeExit(r.exit);
  const intel = normalizeIntel(r.intel);
  if (intel && intel.ip !== ip) return null;
  if (exit?.primary && exit.primary.ip !== ip) return null;
  if (!exit && !intel) return null;
  return {
    at,
    ip,
    countryAllowed: bool(r.countryAllowed),
    exit,
    intel,
  };
}

function storage(): Storage | null {
  try {
    if (typeof window === "undefined") return null;
    return window.localStorage;
  } catch {
    return null;
  }
}

/** 读上次快照；过期、损坏、读不到一律返回 null。 */
export function readExitSnapshot(
  now: number = Date.now(),
): ExitSnapshot | null {
  const store = storage();
  if (!store) return null;
  try {
    const rawText = store.getItem(STORAGE_KEY);
    if (!rawText) return null;
    return parseExitSnapshot(JSON.parse(rawText) as unknown, now);
  } catch {
    return null;
  }
}

/**
 * 覆盖快照。没有 IP 或体积超限时放弃写入，并保留上一条：
 * 一次失败的情报查询不该把上一轮的好数据抹掉。
 */
export function saveExitSnapshot(
  input: Omit<ExitSnapshot, "at"> & { at?: number },
): ExitSnapshot | null {
  const ip = input.ip.trim();
  if (!ip) return null;
  const snapshot: ExitSnapshot = {
    at: input.at ?? Date.now(),
    ip,
    countryAllowed: input.countryAllowed,
    exit: input.exit,
    intel: input.intel,
  };
  const payload = JSON.stringify({ v: SNAPSHOT_VERSION, ...snapshot });
  if (payload.length > MAX_CHARS) return null;
  const store = storage();
  if (!store) return null;
  try {
    store.setItem(STORAGE_KEY, payload);
  } catch {
    // 存储被禁用时只影响首屏复用，不影响本轮结果。
    return null;
  }
  return snapshot;
}

export function clearExitSnapshot(): void {
  try {
    storage()?.removeItem(STORAGE_KEY);
  } catch {
    // 清不掉也不影响下一轮覆盖。
  }
}
