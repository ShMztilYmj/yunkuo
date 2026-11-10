import { AGENTS } from "./aiToolCards";
import type { AiSession, AiUsageItem } from "../../../services/api";
import { sessionState } from "./aiStats";
import type { AgentBotId, AgentBotState } from "../../../shared/ui/AgentBot";

export type { AgentBotId, AgentBotState };

/// 存储键、运行时 ID、进程名与头像 ID 都能认出同一款工具。
const aliases: Record<string, AgentBotId> = {};
for (const agent of AGENTS) {
  if (!agent.avatar) continue;
  const id = agent.avatar as AgentBotId;
  for (const name of [
    agent.key,
    agent.runtime,
    agent.avatar,
    ...agent.aliases,
  ]) {
    if (name) aliases[name.toLowerCase()] = id;
  }
}
export function agentBotId(tool: string): AgentBotId | undefined {
  const key = tool.trim().toLowerCase();
  return Object.prototype.hasOwnProperty.call(aliases, key)
    ? aliases[key]
    : undefined;
}

export function botStateForSession(
  session: AiSession,
  now: number,
): AgentBotState {
  const state = sessionState(session, now);
  if (state === "running") return "working";
  if (state === "asking") return "asking";
  return state === "completed" && now - session.lastActivityAt < 1800
    ? "done"
    : "idle";
}

/** 只根据会话事件表现状态，不把历史 Token 用量当成正在工作。 */
export function botStateForUsage(
  item: AiUsageItem | undefined,
  now: number,
): AgentBotState {
  if (!item) return "idle";
  const sessions = item.recentSessions ?? [];
  if (sessions.some((session) => botStateForSession(session, now) === "asking"))
    return "asking";
  if (
    sessions.some((session) => botStateForSession(session, now) === "working")
  )
    return "working";
  const latest = sessions.reduce<AiSession | undefined>(
    (value, session) =>
      !value || session.lastActivityAt > value.lastActivityAt ? session : value,
    undefined,
  );
  return latest ? botStateForSession(latest, now) : "idle";
}
