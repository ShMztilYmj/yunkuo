import type { AppConfig } from "../../../services/api";
import roster from "../../../../src-tauri/agents.json";

/// 存储键是已持久化的契约，聊天记录、卡片与启动入口都按它落盘。
export type AiToolKey =
  | "claude"
  | "chatgpt"
  | "cursor"
  | "antigravity"
  | "opencode"
  | "kimi"
  | "grok"
  | "glm"
  | "deepseek";

/// 名单的一项，字段与 src-tauri/agents.json 一一对应。
export interface RosterAgent {
  key: AiToolKey;
  runtime: string | null;
  avatar: string | null;
  name: string;
  botName: string;
  hudName: string;
  short: string;
  hint: string;
  protocol: string | null;
  protocolShort: string | null;
  retired: boolean;
  aliases: string[];
  usageClients: string[];
}

const rosterAgents = (roster as unknown as { agents: RosterAgent[] }).agents;

export const AGENTS: readonly RosterAgent[] = rosterAgents;

export function agentFor(key: string): RosterAgent | undefined {
  return rosterAgents.find((agent) => agent.key === key);
}

/**
 * 「Claude / Codex」这类双 CLI 场景的统一显示名。
 *
 * 续跑、账号迁移等旧入口只服务这两个 CLI；此前各页面各写一次三元表达式，
 * 还出现过给 Codex 加 CLI 后缀的措辞漂移。需要全名时传 `full`。
 */
export function cliToolName(key: string, full = false): string {
  if (key === "claude") return full ? "Claude Code" : "Claude";
  return "Codex";
}

/**
 * 旧用量快照缺少 `inputIncludesCached` 时的兜底名单：这些工具的 input 不含缓存，
 * 计算缓存占比前要先补上 cached + cacheWrite。新快照一律以后端字段为准。
 */
export const CACHE_EXCLUSIVE_INPUT: ReadonlySet<string> = new Set([
  "claude",
  "deepseek",
  "kimi",
  "opencode",
]);

/// 界面样式按键维护；除颜色之外的元数据全部来自唯一名单。
const STYLES: Record<AiToolKey, { ring: string; chip: string }> = {
  claude: {
    ring: "from-orange-500 to-red-600 shadow-orange-900/40",
    chip: "bg-orange-500/15 text-orange-300",
  },
  chatgpt: {
    ring: "from-emerald-500 to-teal-600 shadow-emerald-900/40",
    chip: "bg-emerald-500/15 text-emerald-300",
  },
  cursor: {
    ring: "from-zinc-500 to-zinc-700 shadow-zinc-900/40",
    chip: "bg-zinc-500/15 text-zinc-300",
  },
  antigravity: {
    ring: "from-violet-500 to-blue-600 shadow-violet-900/40",
    chip: "bg-violet-500/15 text-violet-300",
  },
  opencode: {
    ring: "from-zinc-500 to-zinc-700 shadow-zinc-900/40",
    chip: "bg-zinc-500/15 text-zinc-300",
  },
  kimi: {
    ring: "from-zinc-500 to-zinc-700 shadow-zinc-900/40",
    chip: "bg-zinc-500/15 text-zinc-300",
  },
  grok: {
    ring: "from-zinc-500 to-zinc-700 shadow-zinc-900/40",
    chip: "bg-zinc-500/15 text-zinc-300",
  },
  glm: {
    ring: "from-zinc-500 to-zinc-700 shadow-zinc-900/40",
    chip: "bg-zinc-500/15 text-zinc-300",
  },
  deepseek: {
    ring: "from-blue-500 to-indigo-600 shadow-blue-900/40",
    chip: "bg-blue-500/15 text-blue-300",
  },
};

export interface AiToolMeta {
  key: AiToolKey;
  label: string;
  hudLabel: string;
  short: string;
  hint: string;
  runtime: string | null;
  protocolShort: string | null;
  ring: string;
  chip: string;
}

function card(agent: RosterAgent): AiToolMeta {
  return {
    key: agent.key,
    label: agent.name,
    hudLabel: agent.hudName,
    short: agent.short,
    hint: agent.hint,
    runtime: agent.runtime,
    protocolShort: agent.protocolShort,
    ...STYLES[agent.key],
  };
}

// 新添加列表只列未退役的工具。
const entries = rosterAgents.map((agent) => ({
  retired: agent.retired,
  meta: card(agent),
}));
export const AI_TOOL_METADATA: readonly AiToolMeta[] = entries.map(
  (entry) => entry.meta,
);
export const AI_TOOL_CATALOG: readonly AiToolMeta[] = entries
  .filter((entry) => !entry.retired)
  .map((entry) => entry.meta);
export const DEFAULT_AI_CARDS: readonly AiToolKey[] = ["claude", "chatgpt"];
const LEGACY_SHORTCUTS = {
  claude: "claude_shortcut",
  chatgpt: "chatgpt_shortcut",
  grok: "grok_shortcut",
} as const;

export function aiToolCards(config: AppConfig | null): AiToolMeta[] {
  const keys = [
    ...new Set([...DEFAULT_AI_CARDS, ...(config?.ai_tool_cards ?? [])]),
  ];
  return keys.flatMap((key) =>
    AI_TOOL_METADATA.filter((tool) => tool.key === key),
  );
}

export function toolShortcut(config: AppConfig | null, key: AiToolKey): string {
  if (key in LEGACY_SHORTCUTS)
    return (
      config?.[LEGACY_SHORTCUTS[key as keyof typeof LEGACY_SHORTCUTS]] ?? ""
    );
  return config?.ai_tool_shortcuts?.[key] ?? "";
}

export function shortcutPatch(
  config: AppConfig | null,
  key: AiToolKey,
  path: string,
): Partial<AppConfig> {
  if (key in LEGACY_SHORTCUTS)
    return { [LEGACY_SHORTCUTS[key as keyof typeof LEGACY_SHORTCUTS]]: path };
  return { ai_tool_shortcuts: { ...config?.ai_tool_shortcuts, [key]: path } };
}
