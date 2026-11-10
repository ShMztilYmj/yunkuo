/**
 * 出口 IP 情报的展示策略：把 ip_intel 的原始字段折算成六面板要用的文案。
 *
 * 分工：
 * - 后端只给「可核验的原始字段」（ASN、RIR、RPKI、代理画像、风险分）；
 * - 这里做二次推断（人机占比、场景评分、带宽档位、蜜罐命中），
 *   全部带 `推导` 标记，UI 上以「本地推断」标注，不冒充第三方检测结果。
 */

import type { IpIntel } from "../../../services/api";
import type { Verdict } from "../signals/exit";

export interface Scaled {
  score: number;
  tone: Verdict;
}

function clamp(n: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, n));
}

/** 人机流量占比（推导）：机房段机器占比高，住宅段以人类为主。 */
export function trafficSplit(
  intel: IpIntel | null,
): { human: number; bot: number } | null {
  if (!intel) return null;
  const base =
    intel.networkClass === "dc" ? 42 : intel.networkClass === "res" ? 8 : 26;
  const risk = intel.riskScore ?? 20;
  const bot = clamp(base + risk * 0.26, 4, 94);
  const rounded = Math.round(bot * 10) / 10;
  return { human: Math.round((100 - rounded) * 10) / 10, bot: rounded };
}

export interface ScenarioRow {
  key: "tiktok" | "social" | "ai";
  label: string;
  score: number;
  tone: Verdict;
}

/** 场景评估（推导）：机房 / 代理 / 高风险出口在社交平台风控里普遍不推荐。 */
export function scenarioScores(intel: IpIntel | null): ScenarioRow[] {
  // 没有情报时不打满分：显示 0 分 + 未知，避免「查不到 = 很干净」的误读
  if (!intel) {
    return [
      { key: "tiktok", label: "TikTok", score: 0, tone: "n" },
      { key: "social", label: "社媒运营", score: 0, tone: "n" },
      { key: "ai", label: "AI 应用", score: 0, tone: "n" },
    ];
  }
  const dc = intel.networkClass === "dc";
  const flagged = intel?.proxy === true || intel?.vpn === true;
  const risky = (intel?.riskScore ?? 0) >= 60;
  const tor = intel?.tor === true;
  const aiPenalty = (dc ? 2 : 0) + (risky ? 1 : 0) + (tor ? 1 : 0);
  const socialPenalty =
    (dc ? 2 : 0) + (flagged ? 1 : 0) + (risky ? 1 : 0) + (tor ? 1 : 0);
  const tone = (s: number): Verdict => (s >= 3 ? "g" : s >= 2 ? "y" : "r");
  const rows: ScenarioRow[] = [
    {
      key: "tiktok",
      label: "TikTok",
      score: clamp(5 - socialPenalty, 0, 5),
      tone: "n",
    },
    {
      key: "social",
      label: "社媒运营",
      score: clamp(5 - socialPenalty, 0, 5),
      tone: "n",
    },
    {
      key: "ai",
      label: "AI 应用",
      score: clamp(5 - aiPenalty, 0, 5),
      tone: "n",
    },
  ];
  return rows.map((r) => ({ ...r, tone: tone(r.score) }));
}

/** 滥用风险档位：后端给 0-100 分，这里只做分档。 */
export function abuseLevel(risk: number | null): {
  label: string;
  tone: Verdict;
} {
  if (risk == null) return { label: "未知", tone: "n" };
  if (risk < 35) return { label: "低风险", tone: "g" };
  if (risk < 75) return { label: "中风险", tone: "y" };
  return { label: "高风险", tone: "r" };
}

/** 黑名单：StopForumSpam + DNSBL（UCEPROTECT L1 / DroneBL）实时查询结果。 */
export function blacklistView(intel: IpIntel | null): {
  label: string;
  tone: Verdict;
  detail: string;
} {
  if (!intel) return { label: "待检测", tone: "n", detail: "尚未取得情报" };
  if (intel.blacklistState === "listed") {
    const note = intel.blacklistNote ? `；${intel.blacklistNote}` : "";
    return {
      label: `命中 ${intel.blacklists.join(" / ")}`,
      tone: "r",
      detail: `实时滥用库 / DNSBL 命中${note}`,
    };
  }
  if (intel.blacklistState === "clean") {
    return {
      label: "未命中",
      tone: "g",
      detail: "StopForumSpam 与 DNSBL 均未收录该地址",
    };
  }
  return {
    label: "未查询",
    tone: "n",
    detail: "查询源不可达，本轮不下结论",
  };
}

/** ASN 规模 → 粗估带宽档位（按宣告 IPv4 地址量折算）。 */
export function bandwidthTier(count: number | null): string | null {
  if (count == null || count <= 0) return null;
  if (count <= 50_000) return "＜1 Tbps";
  if (count <= 200_000) return "1-5 Tbps";
  if (count <= 500_000) return "5-10 Tbps";
  return "10 Tbps 以上";
}

/** 带宽：优先用 PeeringDB 自报值，拿不到才退回按前缀量折算 */
export function bandwidthLabel(intel: IpIntel | null): string | null {
  if (intel?.peeringdbTraffic) return intel.peeringdbTraffic;
  return bandwidthTier(intel?.asnIpv4Count ?? null);
}

/** PeeringDB 明细，用于「预估带宽 / 运营商类型」的 tooltip */
export function peeringdbDetail(intel: IpIntel | null): string | null {
  if (!intel) return null;
  const parts = [
    intel.peeringdbType,
    intel.peeringdbRatio,
    intel.peeringdbScope,
  ].filter((x): x is string => !!x);
  if (!parts.length) return null;
  return `PeeringDB 自报：${parts.join(" · ")}`;
}

/** RIR 所在国的中文名，用于「广播 IP（美国）」这类文案 */
const RIR_COUNTRY_CN: Record<string, string> = {
  US: "美国",
  NL: "荷兰",
  AU: "澳大利亚",
  BR: "巴西",
  ZA: "南非",
};

function countryLabel(cc: string): string {
  return RIR_COUNTRY_CN[cc.toUpperCase()] ?? cc.toUpperCase();
}

/**
 * IP 原生性：IP 段注册机构所在国与出口地理国家不一致 → 广播 IP。
 * 例：ARIN 注册（美国）但实际出口在新加坡。
 */
export function nativeVerdict(
  intel: IpIntel | null,
  cc: string,
): { label: string; tone: Verdict; detail: string } {
  const reg = (intel?.rirCountry ?? "").toUpperCase();
  const exit = cc.toUpperCase();
  if (!reg) return { label: "未知", tone: "n", detail: "未取得注册机构信息" };
  if (!exit) return { label: reg, tone: "n", detail: "未取得出口国家" };
  if (reg === exit) {
    return {
      label: `${exit} 原生`,
      tone: "g",
      detail: `${intel?.rir ?? "RIR"} 注册国与出口国家一致`,
    };
  }
  return {
    label: `广播 IP（${countryLabel(reg)}）`,
    tone: "y",
    detail: `${intel?.rir ?? "RIR"} 注册国 ${reg}，实际出口 ${exit}`,
  };
}

export interface DeepRow {
  label: string;
  text: string;
  tone: Verdict;
  hint: string;
}

/** 风险深度检测：代理画像来自 proxycheck.io，缺失时如实显示未知。 */
export function deepChecks(intel: IpIntel | null): DeepRow[] {
  const tri = (
    flag: boolean | null | undefined,
    onText: string,
    offText: string,
  ): { text: string; tone: Verdict } =>
    flag === true
      ? { text: onText, tone: "r" }
      : flag === false
        ? { text: offText, tone: "g" }
        : { text: "未知", tone: "n" };

  const vpn = tri(intel?.vpn, "已检测到", "未检测到");
  const proxy = tri(intel?.proxy, "已检测到", "未检测到");
  // Tor 优先看出口节点名单；名单不可用时才退回代理画像库的类型字段
  const tor = tri(intel?.torExit ?? intel?.tor, "已检测到", "未检测到");
  const split = trafficSplit(intel);
  const bot = split?.bot ?? 0;
  const crawler: { text: string; tone: Verdict } =
    intel?.isCrawler === true
      ? { text: "已检测到", tone: "r" }
      : intel === null
        ? { text: "未知", tone: "n" }
        : bot >= 55
          ? { text: "疑似机器流量", tone: "y" }
          : { text: "未检测到", tone: "g" };

  return [
    { label: "VPN", ...vpn, hint: "代理画像库中的 VPN 标记" },
    { label: "代理（Proxy）", ...proxy, hint: "代理画像库中的开放代理标记" },
    {
      label: "Tor",
      ...tor,
      hint: intel?.torSource
        ? `来源：${intel.torSource}`
        : "Tor 出口节点名单 / 代理画像库",
    },
    {
      label: "爬虫 / 机器人",
      ...crawler,
      hint: "按机器流量占比推导，非第三方爬虫库",
    },
  ];
}

/** 威胁画像综合结论：命中哪些代理特征。 */
export function threatHeadline(intel: IpIntel | null): DeepRow {
  if (!intel)
    return {
      label: "风险标记",
      text: "待检测",
      tone: "n",
      hint: "尚未取得情报",
    };
  if (intel.tor)
    return {
      label: "风险标记",
      text: "Tor 出口",
      tone: "r",
      hint: "该 IP 被标记为 Tor 出口节点",
    };
  const hits = [intel.proxy && "代理", intel.vpn && "VPN"].filter(Boolean);
  if (hits.length)
    return {
      label: "风险标记",
      text: `存在${hits.join(" / ")}特征`,
      tone: "y",
      hint: "代理画像库命中",
    };
  return {
    label: "风险标记",
    text: "未发现明显威胁",
    tone: "g",
    hint: "公开代理画像库未命中",
  };
}

export function formatCount(n: number | null): string {
  if (n == null || n <= 0) return "—";
  return n.toLocaleString("en-US");
}

export function rpkiView(rpki: IpIntel["rpki"]): {
  label: string;
  tone: Verdict;
} {
  if (rpki === "valid") return { label: "Valid", tone: "g" };
  if (rpki === "invalid") return { label: "Invalid", tone: "r" };
  if (rpki === "unknown") return { label: "未登记 ROA", tone: "n" };
  return { label: "未知", tone: "n" };
}
