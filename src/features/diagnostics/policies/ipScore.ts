/**
 * IP 评分（IP Score）—— 针对单个出口 IP 的 0-100 评分，供出口通道卡顶部评分条使用。
 *
 * 与 Trust Score 的分工：信任分评「整套环境」，IP 分只评「这个出口 IP 本身」。
 * 所有因子都在本地由已有检测结果推断，不额外请求、不外发。
 */

import type { IpIntel } from "../../../services/api";
import type { ExitResult, Verdict } from "../signals/exit";

export interface IpScoreInput {
  /** 本轮检测是否已完成 */
  done: boolean;
  /** 后端已知的出口 IP（exit 层探测失败时仍可能拿到） */
  ip: string | null;
  exit: ExitResult | null;
  countryAllowed: boolean | null;
  /** 出口 IP 情报（RIR / RPKI / 代理画像），缺失时跳过这些因子 */
  intel?: IpIntel | null;
}

export interface IpScoreFactor {
  label: string;
  delta: number;
  tone: Verdict;
}

export type IpBand = "trusted" | "good" | "fair" | "risky";

export interface IpScoreResult {
  score: number;
  band: IpBand;
  factors: IpScoreFactor[];
}

/** 评分档位：文案 + 徽标配色（与参考图的 0 高危 → 100 可信 对齐） */
export const IP_BAND_META: Record<
  IpBand,
  { label: string; chip: string; text: string }
> = {
  trusted: {
    label: "可信",
    chip: "border-emerald-500/30 bg-emerald-500/10 text-emerald-400",
    text: "text-emerald-400",
  },
  good: {
    label: "良好",
    chip: "border-sky-500/30 bg-sky-500/10 text-sky-400",
    text: "text-sky-400",
  },
  fair: {
    label: "一般",
    chip: "border-amber-500/30 bg-amber-500/10 text-amber-400",
    text: "text-amber-400",
  },
  risky: {
    label: "高危",
    chip: "border-red-500/30 bg-red-500/10 text-red-400",
    text: "text-red-400",
  },
};

function bandOf(score: number): IpBand {
  return score >= 78
    ? "trusted"
    : score >= 58
      ? "good"
      : score >= 38
        ? "fair"
        : "risky";
}

export function computeIpScore(input: IpScoreInput): IpScoreResult | null {
  if (!input.done) return null;
  const exit = input.exit;
  // 出口探测层可能整个失败（地理库不可达 / 直连被劫持）。这时只要后端已经知道
  // 出口 IP，就退化成「已知信息打分」：不算 0 分，但起点更低并标出未确认项，
  // 避免界面只能显示「无法评分」。
  const geo = exit && exit.state === "ok" ? exit.primary : null;
  if (!geo && !input.ip) return null;

  const factors: IpScoreFactor[] = [];
  // 中性起点：干净的出口 + 单一时区错位大致落在「良好」区间。
  let score = geo ? 82 : 72;

  const push = (label: string, delta: number, tone: Verdict) => {
    if (!delta) return;
    factors.push({ label, delta, tone });
    score += delta;
  };

  if (!geo) push("出口归属未经地理库确认", -6, "n");

  // 1) ASN 属性：住宅/宽带加分，机房/托管扣分
  if (exit?.asnClass === "res") push("住宅 / 宽带 ASN", 16, "g");
  else if (exit?.asnClass === "dc") push("机房 / 托管 ASN", -8, "y");
  else push("ASN 属性未知", -3, "n");

  // 2) 出口地区
  const cc = (exit?.primary?.cc ?? "").toUpperCase();
  if (cc === "CN") push("出口落在中国大陆", -30, "r");
  if (input.countryAllowed === false) push("目标地区不可用", -22, "r");
  else if (input.countryAllowed == null) push("地区可用性未知", -6, "n");

  // 3) IP × 时区 × 语言 三角一致性（已含系统时区错位，不再单独扣一次）
  const tri = exit?.triangleVerdict ?? "n";
  if (tri === "g") push("三角一致", 12, "g");
  else if (tri === "y") push("时区与出口不符", -7, "y");
  else if (tri === "r") push("身份三角矛盾", -26, "r");
  else push("三角一致性未知", -1, "n");

  // 4) IPv6 出口
  const v6 = exit?.v6Verdict ?? "n";
  if (v6 === "g") push("无 IPv6 泄漏", 4, "g");
  else if (v6 === "y") push("IPv6 可能绕过代理", -6, "y");
  else if (v6 === "r") push("IPv6 落在封锁地区", -18, "r");

  // 5) 代理画像 / RPKI / 滥用风险（来自 ip_intel；缺项不猜，直接跳过）
  const intel = input.intel ?? null;
  if (intel) {
    if (intel.tor) push("Tor 出口", -12, "r");
    else if (intel.vpn) push("VPN 特征", -4, "y");
    else if (intel.proxy) push("代理特征", -4, "y");

    if (intel.rpki === "invalid") push("RPKI 校验无效", -2, "y");
    else if (intel.rpki === "valid") push("RPKI 校验有效", 2, "g");

    if (intel.riskScore != null) {
      if (intel.riskScore >= 75) push(`滥用风险 ${intel.riskScore}`, -6, "r");
      else if (intel.riskScore >= 35)
        push(`滥用风险 ${intel.riskScore}`, -1, "y");
      else push("滥用风险低", 3, "g");
    }

    // 实时黑名单命中比「风险分折算」更硬，单独扣一次
    if (intel.blacklistState === "listed")
      push(`黑名单命中 ${intel.blacklists.length} 个`, -3, "y");
  }

  // 留出上下限：既不给满分也不给 0 分，避免「看起来像占位值」。
  const clamped = Math.max(5, Math.min(95, Math.round(score)));
  return { score: clamped, band: bandOf(clamped), factors };
}
