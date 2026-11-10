import type { AppConfig, NetworkMode, RoutingMode } from "./config";
import { call } from "./invoke";

// ---------- 系统代理 / TUN / 隐私保护 / 网络模式 ----------

export interface IpInfo {
  local_ip: string | null;
  exitIp: string | null;
  exitCountry: string | null;
  exitCountryName: string | null;
}

/** 严格保护的单个体检项；顺序与文案由安全服务决定。 */
export interface ProtectionCheck {
  key: string;
  title: string;
  ok: boolean;
  detail: string;
}

export interface PrivacyStatus {
  armed: boolean;
  healthy: boolean;
  tunnelOn: boolean;
  error: string | null;
  /** 最近一次逐项复核的结果；服务尚未复核时为空数组。 */
  checks?: ProtectionCheck[];
  /** 最近一次复核的 Unix 毫秒时间；从未复核时为 null。 */
  checkedAt?: number | null;
}

/** 混合代理端口的实际占用者；进入代理页时用来提示端口冲突。 */
export interface ProxyPortUsage {
  port: number;
  occupied: boolean;
  /** 占用者就是本应用内核，属于正常监听。 */
  ownedByCore: boolean;
  pid: number | null;
  process: string | null;
}

export const networkApi = {
  // 系统代理是否指向本软件读 core_status.system_proxy；
  // 详细的主机/端口/绕过列表由 get_ip_info 的诊断字段提供。
  setSysproxy: (enable: boolean) => call<AppConfig>("set_sysproxy", { enable }),
  getIpInfo: () => call<IpInfo>("get_ip_info"),
  setTun: (enable: boolean) => call<AppConfig>("set_tun", { enable }),
  privacyEnable: () => call<void>("privacy_enable"),
  privacyRelease: () => call<void>("privacy_release"),
  privacyStatus: () => call<PrivacyStatus>("privacy_status"),
  setNetworkMode: (mode: NetworkMode, allowRelease: boolean) =>
    call<AppConfig>("set_network_mode", { mode, allowRelease }),
  setRoutingMode: (mode: RoutingMode, allowRelease: boolean) =>
    call<AppConfig>("set_routing_mode", { mode, allowRelease }),
  restoreNetworkMode: (allowRelease: boolean) =>
    call<AppConfig>("restore_network_mode", { allowRelease }),
  networkModeBackupStatus: () =>
    call<{ available: boolean; createdAt: string | null }>(
      "network_mode_backup_status",
    ),
  proxyPortUsage: () => call<ProxyPortUsage>("proxy_port_usage"),
};
