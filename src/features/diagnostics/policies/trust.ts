/**
 * Claude 信任评分（Trust Score）—— 把所有检测结果汇成 0-100 的信任分。
 *
 * 打分模型参考 fourierwang66666/claude-ban-risk-check（9 维度权重汇总）
 * 与 Hype-yolo/claude-ban-check（三角矛盾/机房/IPv6 扣分）的思路：
 * 先算综合风险分（0-100，封顶），再取 trust = 100 - risk。
 * 风险越低 → 信任越高 → 越像"北美默认用户"。
 */

import type { ExitResult, Verdict } from "../signals/exit";
import { domesticResolverCount } from "../signals/dnsDeep";
import type { DnsDeepReport } from "../../../services/api";
import type { TurnItem } from "../signals/turnstile";

export interface TrustInput {
  /** 浏览器环境信号风险总分（0-100，与检测页同一套权重口径） */
  webTotal: number;
  native: {
    countryAllowed: boolean | null;
    tzIsCn: boolean;
    timezoneMatches?: boolean | null;
  } | null;
  exit: ExitResult | null;
  /** 深度 DNS 测试结果：与 DNS 卡片共用同一份结论 */
  dnsDeep: DnsDeepReport | null;
  turnstile: TurnItem[] | null;
}

export interface TrustResult {
  /** 0-100，越高越值得信任 */
  score: number;
  risk: number;
  band: "high" | "medium" | "low";
  hits: string[];
}

function verdictRisk(v: Verdict): number {
  return v === "r" ? 1 : v === "y" ? 0.5 : v === "g" ? 0 : 0.25;
}

export function computeTrust(input: TrustInput): TrustResult {
  const hits: string[] = [];
  let risk = 0;

  // 1) 浏览器环境信号（时区/语言/字体/Emoji 等 9 项）
  risk += input.webTotal * 0.35;
  if (input.webTotal >= 31)
    hits.push(`浏览器环境信号 ${input.webTotal}/100 偏高`);

  // 2) 出口 IP 地区
  const nat = input.native;
  if (nat) {
    if (nat.countryAllowed === false) {
      risk += 30;
      hits.push("出口 IP 在受限地区");
    } else if (nat.countryAllowed == null) {
      risk += 15;
      hits.push("出口 IP 未知（代理可能未生效）");
    }
    if (nat.tzIsCn) {
      risk += 10;
      hits.push("系统时区仍为中国大陆时区");
    } else if (nat.timezoneMatches !== true) {
      risk += 10;
      hits.push(
        nat.timezoneMatches === false
          ? "系统时区与出口不匹配"
          : "系统时区与出口一致性未知",
      );
    }
  }

  // 3) 出口一致性层（三角矛盾 / 机房 / IPv6）
  const exit = input.exit;
  if (exit && exit.state === "ok") {
    // 只有「确实对不上」才扣分：n 表示浏览器探测线路与系统时区不同源，
    // 属于「系统代理 / TUN 没开」那条提示的范围，不在这里重复计分。
    if (exit.triangleVerdict === "y" || exit.triangleVerdict === "r") {
      risk += exit.triangleVerdict === "r" ? 18 : 9;
      hits.push(exit.triangleMsg);
    }
    if (exit.asnClass === "dc") {
      risk += 8;
      hits.push("疑似机房 / VPN 出口（推断，供参考）");
    }
    const v6 = verdictRisk(exit.v6Verdict);
    if (v6 > 0) {
      risk += exit.v6Verdict === "r" ? 12 : 4;
      hits.push(exit.v6Msg);
    }
  }

  // 4) DNS 区域泄露：只认深度测试（与 DNS 卡片同一份结论）
  const dnsDeep = input.dnsDeep;
  if (dnsDeep) {
    const domestic = domesticResolverCount(dnsDeep);
    if (domestic > 0) {
      risk += 12;
      hits.push(`DNS 深度测试发现 ${domestic} 个境内解析器`);
    } else if (dnsDeep.resolvers.length === 0) {
      risk += 6;
      hits.push("DNS 深度测试未取得解析器，尚未完成核验");
    }
  }

  // Turnstile 使用恒成功测试密钥，只检查组件加载，不参与风险评分。

  risk = Math.min(100, Math.round(risk));
  const score = 100 - risk;
  const band: TrustResult["band"] =
    score >= 75 ? "high" : score >= 45 ? "medium" : "low";
  return { score, risk, band, hits };
}

export const TRUST_BAND_TEXT: Record<TrustResult["band"], string> = {
  high: "高信任",
  medium: "中信任",
  low: "低信任",
};
