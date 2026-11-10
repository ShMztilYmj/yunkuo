import { call } from "../../../services/api/invoke";
import { coreApi } from "../../../services/api/core";

export type ProxyLifecycle =
  "stopped" | "starting" | "ready" | "reconfiguring" | "stopping" | "failed";

export interface ProxyNode {
  id: string;
  protocol: string;
  udp: boolean | null;
  samples: { time: string; delay: number }[];
  selected: string | null;
  choices: string[];
}

export interface ProxyRule {
  kind: string;
  pattern: string;
  target: string;
  size: number;
}

export interface ProxyConnection {
  id: string;
  network: string;
  inbound: string;
  sourceIp: string;
  destinationIp: string;
  sourcePort: string;
  destinationPort: string;
  host: string;
  processPath: string | null;
  process: string | null;
  uploaded: number;
  downloaded: number;
  startedAt: string;
  chain: string[];
  rule: string;
  rulePattern: string;
}

export interface ProxyConnections {
  downloaded: number;
  uploaded: number;
  items: ProxyConnection[];
}

/** 规则命中解释；跳过的规则说明为什么无法离线判定。 */
export interface ProxyExplanation {
  host: string;
  matched: {
    index: number;
    kind: string;
    payload: string;
    target: string;
  } | null;
  skipped: { index: number; kind: string; reason: string }[];
  skippedCount: number;
  chain: string[];
  finalNode: string | null;
  finalType: string | null;
  fallback: boolean;
  unrouted: boolean;
}

/** 订阅覆写：原文与解析后的条目摘要。 */
export interface ProxyOverlay {
  text: string;
  rules: number;
  groups: string[];
  dnsPolicy: string[];
}

export interface ProxyFailure {
  module: "proxy";
  code: string;
  message: string;
  retryable: boolean;
  operationId: string;
}

/** 保留错误码和操作标识，现有页面 String(error) 仍可展示可读原因。 */
export class ProxyOperationError extends Error {
  readonly details: ProxyFailure;
  constructor(details: ProxyFailure) {
    super(details.message);
    this.name = "ProxyOperationError";
    this.details = details;
  }
}

async function request<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return await call<T>(command, args);
  } catch (reason) {
    if (
      typeof reason === "object" &&
      reason !== null &&
      "module" in reason &&
      reason.module === "proxy" &&
      "message" in reason &&
      typeof reason.message === "string"
    ) {
      throw new ProxyOperationError(reason as ProxyFailure);
    }
    throw reason;
  }
}

/** 调用方只提交业务参数，控制器路径与内核响应由后端适配。 */
export const proxyApi = {
  // 内核与网络状态统一读取 core_status（coreApi.coreStatus），不再保留第二份快照命令。
  nodes: () => request<Record<string, ProxyNode>>("proxy_nodes"),
  rules: () => request<ProxyRule[]>("proxy_rules"),
  /** 内核实际应用的配置；mode 与 TUN 设备名用于判定真实出站路径。 */
  kernelConfigs: () =>
    coreApi.nativeRequest("GET", "/configs") as Promise<{
      mode?: string;
      tun?: { enable?: boolean; device?: string };
    }>,
  connections: () => request<ProxyConnections>("proxy_connections"),
  selectNode: (group: string, node: string) =>
    request<void>("proxy_select_node", { group, node }),
  closeConnections: (id: string | null = null) =>
    request<void>("proxy_close_connections", { id }),
  delay: (
    node: string,
    group = false,
    /** 住宅/中转链路一次往返常到 4—6 秒，5 秒会把可用节点也判成超时。 */
    timeout = 8000,
    url = "https://www.gstatic.com/generate_204",
  ) =>
    request<Record<string, number | Record<string, unknown>>>("proxy_delay", {
      node,
      group,
      timeout,
      url,
    }),
  /** 只读两次控制器快照，回答「这个域名会命中哪条规则」。 */
  explain: (host: string) =>
    request<ProxyExplanation>("proxy_explain", { host }),
  overlay: () => request<ProxyOverlay>("proxy_overlay"),
  overlaySave: (text: string) =>
    request<ProxyOverlay>("proxy_overlay_save", { text }),
  overlayReset: () => request<ProxyOverlay>("proxy_overlay_reset"),
};
