import { call } from "./invoke";
import type { PrivacyStatus } from "./network";

// ---------- 原生代理状态 / 流量 / 连接 ----------

/**
 * 引擎能力描述。嵌套结构由后端自带 camelCase，
 * 所以外层状态统一 snake_case 时这些键名仍保持原样。
 */
export interface CoreCapabilities {
  engine: string;
  hosting: "managed-process" | "embedded";
  tcpDnsListener: boolean;
  embedded: boolean;
  tunAvailable: boolean;
  fullReloadPreservesConnections: boolean;
}

export interface CoreStatus {
  running: boolean;
  adopted: boolean;
  version: string | null;
  started_at: number | null;
  elevated: boolean;
  state?:
    "stopped" | "starting" | "ready" | "reconfiguring" | "stopping" | "failed";
  service_managed?: boolean;
  system_proxy?: boolean;
  tun?: boolean;
  last_error?: string | null;
  /** 与内核状态在同一次读取中返回，避免另发服务状态请求。 */
  privacy?: PrivacyStatus | null;
  // ---- 原 proxy_snapshot 合并进来的观察字段，沿用 core_status 的 snake_case ----
  /** 控制器已就绪；与 running 同源，按语义命名读取。 */
  controller_ready: boolean;
  /** 内核代际与网络修订号：读取期间是否发生过替换或网络变更。 */
  generation: number;
  network_revision: number;
  observed_at: string;
  /** 配置意图（是否要求 TUN），与表示网卡实际已起的 tun 区分。 */
  tun_requested: boolean;
  /** 严格保护的布尔投影；privacy 未查询时为 null（未知）。 */
  protection: boolean | null;
  /** 纯常量能力描述，读取不产生额外 IPC。 */
  capabilities: CoreCapabilities;
  /** 最近一次成功应用配置的方式：hot 表示没换进程、连接保持。 */
  last_apply?: { hot: boolean; at: string; note: string } | null;
}

export interface RuntimeInfo {
  controller: string;
  secret: string;
  mixed_port: number;
  data_dir: string;
}

export interface TrafficInfo {
  upload: number;
  download: number;
  total: number;
  /** Unix 秒 */
  expire: number;
}

/** 消息等级：与后端 Notice.level 一一对应，消息中心据此分组与排序。 */
export type NoticeLevel = "error" | "warn" | "info";

/** 内核与联动产生的未处理提示。 */
export interface CoreNotice {
  id: number;
  level: NoticeLevel;
  message: string;
}

export const coreApi = {
  coreStatus: () => call<CoreStatus>("core_status"),
  coreErrors: () => call<CoreNotice[]>("get_core_errors"),
  dismissCoreError: (id: number) => call<void>("dismiss_core_error", { id }),
  connectionTotals: () =>
    call<{ uploadTotal: number; downloadTotal: number }>("connection_totals"),
  coreStart: () => call<void>("core_start"),
  coreRestart: () => call<void>("core_restart"),
  coreStop: () => call<void>("core_stop"),
  runtimeInfo: () => call<RuntimeInfo>("get_runtime_info"),
  // 两条控制器转发通道命令名不同（旧客户端别名），页面只应通过 controller.ts 取数据。
  nativeRequest: (method: string, path: string, body?: unknown) =>
    call<unknown>("native_proxy_request", { method, path, body: body ?? null }),

  mihomoRequest: (method: string, path: string, body?: unknown) =>
    call<unknown>("mihomo_request", { method, path, body: body ?? null }),
};
