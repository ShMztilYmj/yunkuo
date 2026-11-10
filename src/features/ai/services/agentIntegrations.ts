import { call } from "../../../services/api/invoke";

/** 已添加 Agent 的本地接入方式：local 为本地配置，hooks 为官方 hooks。 */
export interface AgentConnection {
  key: string;
  connected: boolean;
  method: "local" | "hooks";
  note: string;
}

export const agentIntegrations = {
  status: () => call<AgentConnection[]>("agent_integration_status"),
  connect: (key: string, enabled: boolean) =>
    call<AgentConnection[]>("agent_integration_connect", { key, enabled }),
};
