import { call } from "../../../services/api/invoke";

export type AgentHudScope = "online" | "installed";
export type AgentHudDock = "top" | "left" | "right" | "free";
export interface AgentHudStatus {
  enabled: boolean;
  alwaysOnTop: boolean;
  autoDockTop: boolean;
  dockEdge?: AgentHudDock;
  stripOffsetX?: number;
  stripOffsetY?: number;
  toolScope: AgentHudScope;
  expanded: boolean;
  compactWidth: number;
  sideHeight: number;
}
export interface AgentHudTool {
  key: string;
  installed: boolean;
  online: boolean;
}
export interface AgentHudOptions {
  autoDockTop?: boolean;
  toolScope?: AgentHudScope;
}
export const agentHud = {
  status: () => call<AgentHudStatus>("agent_hud_status"),
  ready: () => call<AgentHudStatus>("agent_hud_ready"),
  enabled: (enabled: boolean) =>
    call<AgentHudStatus>("agent_hud_set_enabled", { enabled }),
  top: (alwaysOnTop: boolean) =>
    call<AgentHudStatus>("agent_hud_set_top", { alwaysOnTop }),
  options: (options: AgentHudOptions) =>
    call<AgentHudStatus>("agent_hud_set_options", { ...options }),
  tools: () => call<AgentHudTool[]>("agent_hud_tools"),
  expand: (expanded: boolean, toolCount?: number) =>
    call<AgentHudStatus | undefined>("agent_hud_expand", {
      expanded,
      toolCount,
    }),
  snap: () => call<boolean>("agent_hud_snap"),
  reset: () => call<void>("agent_hud_reset"),
  open: (page: "stats" | "settings" | "environment" | "tools" | "detect") =>
    call<void>("agent_hud_open_main", { page }),
};
