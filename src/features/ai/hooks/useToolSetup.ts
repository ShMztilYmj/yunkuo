import { useEffect, useState } from "react";
import { usePageActive } from "../../../shared/hooks/usePageActive";
import { usePolling } from "../../../shared/hooks/usePolling";
import { useTauriEvent } from "../../../shared/hooks/useTauriEvent";
import {
  invalidateToolSetup,
  readToolSetup,
  type ToolSetupSnapshot,
} from "../services/toolSetup";

export function useToolSetup() {
  const active = usePageActive();
  const [snapshot, setSnapshot] = useState<ToolSetupSnapshot | null>(null);
  const [error, setError] = useState("");
  const [revision, setRevision] = useState(0);
  useTauriEvent("config-changed", invalidateToolSetup, active);
  useTauriEvent("ai-tool-config-changed", invalidateToolSetup, active);
  // 页面内的手动刷新事件不是 Tauri 事件，仍在 effect 里挂载。
  useEffect(() => {
    if (!active) return;
    const changed = () => setRevision((value) => value + 1);
    window.addEventListener("tool-setup-refresh", changed);
    return () => window.removeEventListener("tool-setup-refresh", changed);
  }, [active]);
  usePolling(
    async (current) => {
      try {
        const next = await readToolSetup();
        if (current()) {
          setSnapshot(next);
          setError("");
        }
      } catch (reason) {
        if (current()) {
          setSnapshot(null);
          setError(`工具状态读取失败：${String(reason)}`);
        }
      }
    },
    15_000,
    true,
    true,
    revision,
  );
  return { snapshot, error, refresh: invalidateToolSetup };
}
