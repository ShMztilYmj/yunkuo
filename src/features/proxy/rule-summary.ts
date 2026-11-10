import type { ProxyRule } from "./services/api";

/** 内核在 rules 末尾补的 Match 没有条件，承接“剩下的全部”，本身不构成分流。 */
export function isFallbackRule(rule: Pick<ProxyRule, "kind">): boolean {
  return rule.kind.trim().toLowerCase() === "match";
}

export interface RuleOverview {
  /** 表格里的总行数，包含兜底 Match。 */
  total: number;
  /** 真正带条件的规则条数。 */
  routing: number;
  /** 订阅没有 rules 段。 */
  empty: boolean;
  /** 只剩兜底 Match：所有流量落到同一个策略。 */
  onlyFallback: boolean;
  /** 两种都按“当前订阅没有规则”提示。 */
  missing: boolean;
  /** 兜底行的策略名，没有兜底行时为 null。 */
  fallbackTarget: string | null;
}

export function summarizeRules(rules: ProxyRule[]): RuleOverview {
  const routing = rules.filter((rule) => !isFallbackRule(rule)).length;
  const onlyFallback = rules.length > 0 && routing === 0;
  return {
    total: rules.length,
    routing,
    empty: rules.length === 0,
    onlyFallback,
    missing: rules.length === 0 || onlyFallback,
    fallbackTarget: rules.find(isFallbackRule)?.target.trim() || null,
  };
}

export interface MissingRulesNotice {
  title: string;
  detail: string;
}

/** 规则页顶部提示：没有分流规则时，说明规则模式等同全局模式。 */
export function missingRulesNotice(
  overview: RuleOverview,
): MissingRulesNotice | null {
  if (!overview.missing) return null;
  const advice =
    "要按域名或网段分流，请在概览里改用「日常分流」，或导入带规则的 Clash 配置。";
  if (overview.onlyFallback) {
    const target = overview.fallbackTarget ?? "未指定策略";
    return {
      title: "当前订阅没有规则",
      detail: `只有一条兜底 Match → ${target}，所有流量都走它，规则模式 ≈ 全局模式。${advice}`,
    };
  }
  return {
    title: "当前订阅没有规则",
    detail: `配置里没有 rules 段，规则模式下没有可匹配的条件。${advice}`,
  };
}
