import type { NativeDetect } from "../../services/api";

/** 启动门控的输入（由检测页产出，写入 store 供启动页判定使用） */
export interface GateInput {
  finished: boolean;
  scanning: boolean;
  native: NativeDetect | null;
  /** 浏览器信号总分（0-100） */
  webTotal: number;
}

/** 门控项标识：检测页据此把「未通过项」映射成可执行的修复条目。 */
export type GateCode =
  | "unscanned"
  | "core"
  | "tun"
  | "sysproxy"
  | "global"
  | "node"
  | "country"
  | "exit-ip"
  | "claude"
  | "tz-cn"
  | "tz-mismatch"
  | "tz-unknown"
  | "score-invalid"
  | "score-high";

export interface GateIssue {
  code: GateCode;
  message: string;
}

export interface GateVerdict {
  pass: boolean;
  /** 未通过项文案，供启动页逐条展示；与 issues 一一对应 */
  failures: string[];
  /** 结构化版本：修复清单按 code 生成，避免文案改动导致漏项 */
  issues: GateIssue[];
}

export const GATE_DEFAULT_THRESHOLD = 31;

/**
 * 门控策略只在这一处解析。
 * 检测页、AI 卡片页、首页、维护页、侧边栏都必须取同一组阈值与连通性要求，
 * 否则同一份检测快照会被算出两种结论。
 */
export function gateLimits(
  config:
    | {
        gate_threshold?: number | null;
        require_claude_reachable?: boolean | null;
      }
    | null
    | undefined,
): { threshold: number; requireClaudeReachable: boolean } {
  return {
    threshold: config?.gate_threshold ?? GATE_DEFAULT_THRESHOLD,
    requireClaudeReachable: config?.require_claude_reachable ?? true,
  };
}

export function computeGate(
  d: GateInput | null,
  threshold: number,
  requireClaudeReachable: boolean = true,
): GateVerdict {
  if (!d || d.scanning || !d.finished || !d.native) {
    const issue = { code: "unscanned" as const, message: "请先完成环境检测" };
    return { pass: false, failures: [issue.message], issues: [issue] };
  }
  const n = d.native;
  const issues: GateIssue[] = [];
  const add = (code: GateCode, message: string) =>
    issues.push({ code, message });
  if (!n.coreRunning) add("core", "代理内核未运行");
  if (!n.tunOn) add("tun", "TUN 未实际开启");
  if (!n.sysproxyOn) add("sysproxy", "系统代理未开启或未指向本软件");
  if (!n.globalOn) add("global", "必须使用全局代理模式");
  if (!n.proxyNodeSelected)
    add("node", "全局代理未选择有效节点（禁止 DIRECT）");
  if (n.countryAllowed === false)
    add("country", `出口 IP 在受限地区 ${n.exitCountry ?? ""}`.trim());
  if (n.countryAllowed == null)
    add("exit-ip", "无法获取出口 IP（代理可能未生效）");
  if (requireClaudeReachable && n.claudeReachable !== true)
    add(
      "claude",
      n.claudeError ?? "claude.ai 连通性未通过，请检查代理节点后重新检测",
    );
  if (n.tzIsCn)
    add(
      "tz-cn",
      "系统时区为 Asia/Shanghai / Asia/Urumqi，请先按出口 IP 同步时区",
    );
  else if (n.timezoneMatches === false)
    add("tz-mismatch", "系统时区与出口不匹配，请同步时区");
  else if (n.timezoneMatches !== true)
    add("tz-unknown", "无法确认系统时区与出口一致，请重新检测");
  if (!Number.isInteger(d.webTotal) || d.webTotal < 0 || d.webTotal > 100)
    add("score-invalid", "环境评分无效，请重新检测");
  else if (d.webTotal >= threshold)
    add("score-high", `环境评分 ${d.webTotal} ≥ 阈值 ${threshold}，风险偏高`);
  return {
    pass: issues.length === 0,
    failures: issues.map((issue) => issue.message),
    issues,
  };
}
