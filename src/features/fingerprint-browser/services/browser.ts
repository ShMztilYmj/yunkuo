import { call } from "../../../services/api/invoke";
import type { Region } from "../../../shared/types/environment";
export type { Region } from "../../../shared/types/environment";

export interface BrowserConfig {
  seed: number;
  language: string;
  timezone: string;
}
/** 由应用层从检测模块投影，浏览器只展示最近一次检测结果。 */
export interface BrowserDiagnostics {
  scanning: boolean;
  at: number | null;
  score: number | null;
  ip: string | null;
  country: string | null;
  timezone: string | null;
}
export interface BrowserEnvironment {
  installed: boolean;
  config: BrowserConfig;
  slot: string;
  running: boolean;
}
export interface BrowserRequests {
  pendingCount: number;
  notice: string | null;
}
export const browserEnvironment = {
  browser: () => call<BrowserEnvironment>("browser_environment_status"),
  browserRequests: () => call<BrowserRequests>("browser_request_status"),
  openPending: () => call<void>("browser_open_pending"),
  saveBrowser: (config: BrowserConfig, rotate = false) =>
    call<void>("browser_environment_save", { config, rotate }),
  launchBrowser: (openClaude = false) =>
    call<void>("browser_environment_launch", { openClaude }),
  clearBrowser: (all: boolean) =>
    call<void>("browser_environment_clear", { all, confirmed: true }),
  // 复用后端地区目录；浏览器页面只读取预设，不持有或修改系统环境。
  regions: async () =>
    (await call<{ presets: Region[] }>("computer_environment_status")).presets,
};
