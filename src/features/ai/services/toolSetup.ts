import { call } from "../../../services/api/invoke";

import type { ToolConnection } from "../../../services/api/ai";
export type { ToolConnection };
export interface ToolSetup {
  key: string;
  installed: boolean | null;
  running: boolean | null;
  configured: boolean;
  shortcutValid: boolean;
  candidates: string[];
  connection: ToolConnection;
}
export interface ToolSetupSnapshot {
  tools: ToolSetup[];
  checkedAt: number;
}
export interface ToolInspection {
  candidates: string[];
  version: string | null;
  note: string;
}
export interface ToolConnectivity {
  reachable: boolean;
  endpoint: string;
  status: number | null;
  note: string;
}
/** 自动定位结果：`path` 为已有文件时才可直接保存为启动入口。 */
export interface ToolAutodetect {
  path: string | null;
  candidates: string[];
  note: string;
}
let cached: ToolSetupSnapshot | null = null,
  expires = 0,
  generation = 0;
let pending: Promise<ToolSetupSnapshot> | null = null;
export function invalidateToolSetup() {
  cached = null;
  expires = 0;
  generation++;
  window.dispatchEvent(new Event("tool-setup-refresh"));
}
export function readToolSetup(): Promise<ToolSetupSnapshot> {
  if (cached && Date.now() < expires) return Promise.resolve(cached);
  if (pending) return pending.then(() => readToolSetup());
  const current = generation;
  pending = call<ToolSetupSnapshot>("tool_setup_status")
    .then((value) => {
      if (generation === current) {
        cached = value;
        expires = Date.now() + 5000;
      }
      return value;
    })
    .finally(() => {
      pending = null;
    });
  return pending.then((value) =>
    generation === current ? value : readToolSetup(),
  );
}
export const toolSetup = {
  inspect: (tool: string) =>
    call<ToolInspection>("tool_setup_inspect", { tool }),
  test: (tool: string) => call<ToolConnectivity>("tool_setup_test", { tool }),
  autodetect: (tool: string) =>
    call<ToolAutodetect>("tool_setup_autodetect", { tool }),
};
export const connectionSource = (connection?: ToolConnection) =>
  ({
    native: "官方直连",
    external: "自定义接口",
    tool: "工具自行管理",
    unknown: "连接未知",
  })[connection?.source ?? "unknown"] ?? "连接未知";
export const connectionModel = (connection: ToolConnection) =>
  connection.model ??
  (connection.source === "unknown"
    ? "模型待确认"
    : connection.source === "tool"
      ? "在工具中选模型"
      : "沿用工具默认模型");
export const connectionLine = (connection?: ToolConnection) =>
  connection
    ? `${connectionSource(connection)} · ${connectionModel(connection)}`
    : "正在读取用户配置…";
