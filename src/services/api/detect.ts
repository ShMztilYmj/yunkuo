import { call } from "./invoke";

// ---------- 环境检测 / ipmap / DNS 深度探测 ----------

export interface NativeDetect {
  coreRunning: boolean;
  sysproxyOn: boolean;
  sysproxyHost: string | null;
  sysproxyPort: number | null;
  tunOn: boolean;
  globalOn: boolean;
  proxyNodeSelected: boolean;
  privacyArmed: boolean;
  privacyHealthy: boolean;
  privacyError: string | null;
  exitIp: string | null;
  exitCountry: string | null;
  exitCountryName: string | null;
  countryAllowed: boolean | null;
  claudeReachable: boolean | null;
  claudeStatus: number | null;
  claudeError: string | null;
  currentTz: string;
  currentTzIana: string;
  exitTimezone: string | null;
  timezoneMatches: boolean | null;
  tzIsCn: boolean;
}

export interface TraceExit {
  platform: string;
  ip: string | null;
  region: string | null;
  error: string | null;
}

export interface GeoRow {
  source: string;
  country: string;
  city: string;
  cc: string;
  error: string | null;
}

/**
 * 出口 IP 情报（后端 ip_intel 命令）。
 * 全部字段来自公开 IP 库 / RIR 数据，缺失时一律为 null，前端不自行编造。
 */
export interface IpIntel {
  ip: string;
  /** ok = 主数据源齐全；partial = 部分失败；fail = 全部失败 */
  state: "ok" | "partial" | "fail";
  error: string | null;
  asn: number | null;
  /** ASN 归属（AS 名称），如 FDCSERVERS */
  asnOrg: string | null;
  /** ASN 持有人全称 */
  asnName: string | null;
  isp: string | null;
  org: string | null;
  /** 运营商类型：hosting / isp / unknown */
  asnType: "hosting" | "isp" | "unknown" | null;
  /** IP 标记：dc 机房 / res 住宅 / unknown */
  networkClass: "dc" | "res" | "unknown" | null;
  /** IP 段注册机构：ARIN / RIPE NCC / APNIC / LACNIC / AFRINIC */
  rir: string | null;
  /** 注册机构所在国家码（原生 / 广播判断） */
  rirCountry: string | null;
  prefix: string | null;
  asnIpv4Count: number | null;
  asnRegistered: string | null;
  isBogon: boolean | null;
  reverseDns: string | null;
  mobile: boolean | null;
  hosting: boolean | null;
  proxy: boolean | null;
  vpn: boolean | null;
  tor: boolean | null;
  proxyType: string | null;
  /** 滥用风险 0-100 */
  riskScore: number | null;
  /** RPKI 路由校验：valid / invalid / unknown */
  rpki: "valid" | "invalid" | "unknown" | null;
  /** PeeringDB 自报线路容量，如 5-10Tbps */
  peeringdbTraffic: string | null;
  /** PeeringDB 自报网络类型：Content / NSP / Cable/DSL/ISP / Enterprise */
  peeringdbType: string | null;
  /** PeeringDB 流量比例：Balanced / Heavy Inbound / Heavy Outbound */
  peeringdbRatio: string | null;
  /** PeeringDB 覆盖范围：Global / Regional / ... */
  peeringdbScope: string | null;
  peeringdbPrefixes4: number | null;
  peeringdbName: string | null;
  /** Tor 出口名单命中（null = 名单没拉到，不代表不是 Tor） */
  torExit: boolean | null;
  torSource: string | null;
  /** 黑名单结论：listed / clean / unavailable */
  blacklistState: "listed" | "clean" | "unavailable";
  blacklists: string[];
  blacklistNote: string | null;
  /** ipapi.is 的滥用分（形如 0.0605 (High)），仅在配置 Key 时存在 */
  abuserScore: string | null;
  isAbuser: boolean | null;
  isCrawler: boolean | null;
  abuseEmail: string | null;
  /** ipapi.is 分支状态：ok / failed（配了 Key 但到不了）/ off */
  ipapiisState: "ok" | "failed" | "off";
  sources: string[];
}

export interface DnsLeakRow {
  ip: string;
  country: string;
  cc: string;
  isp: string;
  leaked: boolean | null;
  via?: string;
}

export interface DnsDeepChallenge {
  id: string;
  hostnames: string[];
  rounds: number;
  perRound: number;
  provider: string;
}
export interface DnsDeepResolver {
  ip: string;
  family: "IPv4" | "IPv6";
  country: string;
  cc: string;
  city: string;
  operator: string;
  asn: string;
}
export interface DnsDeepReport {
  state: "observed" | "review" | "unknown";
  detail: string;
  resolvers: DnsDeepResolver[];
  filtered: number;
}

export interface DiagnosticsReportStatus {
  revision: number;
  valid: boolean;
  ageSeconds: number | null;
  lifetimeSeconds: number;
}

/**
 * 上次检测摘要（`environment/last-scan.json`）。
 *
 * 只用于展示「上次差什么」，故意不含 `finished` / `native` / `webResults`：
 * 启动门控仍然只认本轮检测，历史数据在类型上就喂不进 `computeGate`。
 */
export interface LastScanSystem {
  /** Windows 时区键名，与 NativeDetect.currentTz 同一表示 */
  timezone: string;
  languages: string[];
  culture: string;
}

export interface LastScanSummary {
  /** unix ms */
  at: number;
  score: number;
  /** 上次未通过的项（人话） */
  failures: string[];
  system: LastScanSystem;
  /** 当前配置语义摘要与上次不同 → 旧结论已不代表当前网络策略 */
  configChanged: boolean;
}

/** 指纹浏览器自检：本地页面地址与一次性票据 */
export interface SelfcheckStart {
  ticket: string;
  url: string;
}

export interface SelfcheckStatus {
  ready: boolean;
  /** 未解析的原始 JSON，由 selfcheck/report.ts 宽松解析 */
  report: unknown;
  /** 剩余有效秒数；0 表示本次自检已结束 */
  expiresIn: number;
}

export const detectApi = {
  detectNative: () => call<NativeDetect>("detect_native"),
  recordWebScan: (webScore: number | null) =>
    call<void>("record_web_scan", { webScore }),
  beginEnvironmentScan: () => call<string>("begin_environment_scan"),
  finishEnvironmentScan: (scanId: string, webScore: number) =>
    call<void>("finish_environment_scan", { scanId, webScore }),
  diagnosticsReportStatus: () =>
    call<DiagnosticsReportStatus>("get_diagnostics_report_status"),

  /** 上次检测摘要；没有或已损坏时返回 null，不作为任何判定依据。 */
  lastScan: () => call<LastScanSummary | null>("get_last_scan"),
  saveLastScan: (input: {
    score: number;
    failures: string[];
    system: LastScanSystem;
  }) => call<void>("save_last_scan", { ...input }),

  /** 在指纹浏览器里打开一次性自检页；脚本由 selfcheck/probe.ts 生成。 */
  browserSelfcheckStart: (script: string) =>
    call<SelfcheckStart>("browser_selfcheck_start", { script }),
  browserSelfcheckStatus: (ticket: string) =>
    call<SelfcheckStatus>("browser_selfcheck_status", { ticket }),

  traceExit: (platform: string) => call<TraceExit>("trace_exit", { platform }),
  multiGeo: (ip: string | null) =>
    call<GeoRow[]>("multi_geo", { ip: ip === null ? null : ip }),
  ipIntel: (ip: string | null) =>
    call<IpIntel>("ip_intel", { ip: ip === null ? null : ip }),
  scanPorts: (ip: string) => call<number[]>("scan_ports", { ip }),
  dnsLeak: (rounds: number | null) =>
    call<DnsLeakRow[]>("dns_leak", { rounds }),
  dnsDeepStart: () => call<DnsDeepChallenge>("dns_deep_start"),
  dnsDeepCollect: (id: string) =>
    call<DnsDeepReport>("dns_deep_collect", { id }),
  dnsDeepCancel: (id: string) => call<void>("dns_deep_cancel", { id }),
};
