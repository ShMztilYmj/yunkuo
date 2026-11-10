import type { TzSyncInfo } from "./timezone";
import { call } from "./invoke";

// ---------- 配置 ----------

export type NetworkMode = "daily" | "strict";
export type RoutingMode = "global" | "rule" | "direct";

export interface AppConfig {
  proxy_engine?: "mihomo";
  mixed_port: number;
  external_controller_port: number;
  secret: string;
  enable_system_proxy: boolean;
  enable_proxy_guard: boolean;
  proxy_guard_interval: number;
  proxy_bypass: string;
  enable_tun: boolean;
  mode: string;
  network_mode?: NetworkMode | null;
  daily_proxy?: string | null;
  log_level: string;
  allow_lan: boolean;
  ipv6: boolean;
  enable_auto_launch: boolean;
  enable_silent_start: boolean;
  close_to_tray: boolean;
  tz_auto_sync: boolean;
  tz_restore_on_proxy_off: boolean;
  claude_shortcut: string;
  chatgpt_shortcut: string;
  grok_shortcut: string;
  ai_tool_cards?: string[];
  ai_tool_shortcuts?: Record<string, string>;

  /** 本地用量扫描：关闭后不再读取各工具的会话文件。 */
  ai_usage_scan: boolean;
  gate_threshold: number;
  require_claude_reachable: boolean;
  blocked_countries: string[];
  /** ipapi.is 免费 API Key（可选）：填入后出口情报多一个独立源 */
  ipapiis_key: string;
  last_tz_sync: TzSyncInfo | null;
}

export const configApi = {
  getConfig: () => call<AppConfig>("get_config"),
  patchConfig: (patch: Partial<AppConfig>) =>
    call<AppConfig>("patch_config", { patch }),
};
