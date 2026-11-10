import { useCallback, useEffect, useRef, useState } from "react";
import {
  api,
  type AiAccountEntry,
  type AiUsageItem,
} from "../../../services/api";
import { readAiUsage } from "../services/aiUsage";
import { identityEnvironment, type Identity } from "../services/identities";
import { usePolling } from "../../../shared/hooks/usePolling";

/**
 * 服务商页的组合读取：用量快照提供模型源，账号快照与 Claude 身份提供本地账号。
 * 任一来源失败只降级该部分，不把整页清空。
 */
export function useProviderAccounts(keys: readonly string[], enabled = true) {
  const scope = keys.join(",");
  const [items, setItems] = useState<AiUsageItem[] | null>(null);
  const [codexAccounts, setCodexAccounts] = useState<AiAccountEntry[]>([]);
  const [claudeIdentities, setClaudeIdentities] = useState<Identity[]>([]);
  const [warnings, setWarnings] = useState<string[]>([]);
  const [readAt, setReadAt] = useState<number | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const mounted = useRef(true);
  const sequence = useRef(0);
  const needsCodex = keys.includes("chatgpt");
  const needsClaude = keys.includes("claude");

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const load = useCallback(
    async (force = false) => {
      const request = ++sequence.current;
      setRefreshing(true);
      try {
        const [usage, codex, claude] = await Promise.allSettled([
          readAiUsage(force),
          needsCodex
            ? api.aiListAccounts("chatgpt")
            : Promise.resolve([] as AiAccountEntry[]),
          needsClaude
            ? identityEnvironment.identities()
            : Promise.resolve(null),
        ]);
        if (!mounted.current || request !== sequence.current) return;
        const next: string[] = [];
        if (usage.status === "fulfilled") setItems(usage.value.items);
        else next.push(`用量与模型源读取失败：${String(usage.reason)}`);
        if (codex.status === "fulfilled") setCodexAccounts(codex.value);
        else next.push(`Codex 账号快照读取失败：${String(codex.reason)}`);
        if (claude.status === "fulfilled" && claude.value)
          setClaudeIdentities(claude.value.catalog.records);
        else if (claude.status === "rejected")
          next.push(`Claude 身份备份读取失败：${String(claude.reason)}`);
        setWarnings(next);
        setReadAt(Date.now());
        setLoaded(true);
      } finally {
        if (mounted.current && request === sequence.current)
          setRefreshing(false);
      }
    },
    [needsCodex, needsClaude],
  );

  usePolling(() => load(false), 60_000, true, enabled, scope);

  return {
    items,
    codexAccounts,
    claudeIdentities,
    warnings,
    readAt,
    refreshing,
    loaded,
    refresh: () => load(true),
  };
}
