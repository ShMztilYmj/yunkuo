import { configurationDomain } from "../../shared/state/configuration";
import { useAppStore } from "../../store";

export const AI_CONFIG_FIELDS = [
  "claude_shortcut",
  "chatgpt_shortcut",
  "grok_shortcut",
  "ai_tool_cards",
  "ai_tool_shortcuts",
  "ai_usage_scan",
] as const;
export const AI_PATCH_FIELDS = [
  "claude_shortcut",
  "chatgpt_shortcut",
  "grok_shortcut",
  "ai_tool_cards",
  "ai_tool_shortcuts",
  "ai_usage_scan",
] as const;
const domain = configurationDomain(AI_CONFIG_FIELDS, AI_PATCH_FIELDS);
export const aiConfiguration = {
  view: domain.view,
  patch: (patch: Parameters<typeof domain.patch>[0]) =>
    useAppStore.getState().patch(domain.patch(patch)),
};
