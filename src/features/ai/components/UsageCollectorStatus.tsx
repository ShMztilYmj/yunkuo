import { useRef, useState } from "react";
import { usePolling } from "../../../shared/hooks/usePolling";
import {
  aiUsageEngine,
  type AiUsageEngineStatus as Status,
} from "../services/aiUsageEngine";
import { aiToolCards } from "../services/aiToolCards";
import { useAppStore } from "../../../store";

export default function UsageCollectorStatus({
  onChanged,
  enabled,
}: {
  onChanged: () => void;
  enabled: boolean;
}) {
  const tools = aiToolCards(useAppStore((state) => state.config));
  const [status, setStatus] = useState<Status | null>(null);
  const [error, setError] = useState("");
  const previous = useRef<Status | null>(null);
  usePolling(
    async (isCurrent) => {
      try {
        const next = await aiUsageEngine.status();
        if (!isCurrent() || !next) return;
        const before = previous.current;
        previous.current = next;
        setStatus(next);
        if (
          before &&
          (before.lastSuccessAt !== next.lastSuccessAt ||
            (before.running && !next.running))
        )
          onChanged();
      } catch {
        /* Older preview hosts can still show lifecycle connections. */
      }
    },
    5000,
    true,
    enabled,
    tools.map((tool) => tool.key).join(","),
  );
  const sync = async () => {
    setError("");
    try {
      const next = await aiUsageEngine.refresh();
      previous.current = next;
      setStatus(next);
    } catch {
      setError("同步未启动，请检查采集引擎安装后重试。");
    }
  };
  return (
    <div
      className="mt-3 rounded-xl bg-zinc-500/5 p-3"
      aria-label="统一用量采集"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <strong className="text-xs">
          统一用量采集 · 已添加 {tools.length} 款 Agent
        </strong>
        <button
          className="btn-secondary rounded-lg px-2 py-1 text-xs disabled:opacity-50"
          disabled={status?.running || status?.available === false}
          onClick={() => void sync()}
        >
          {status?.running ? "同步中…" : "同步用量与额度"}
        </button>
      </div>
      <p className="mt-2 text-[11px] leading-relaxed text-zinc-500">
        {error ||
          status?.issue ||
          (status?.available === false
            ? "采集引擎未安装，请修复软件安装。"
            : status?.running
              ? "正在后台读取，完成后自动更新；首次云端同步可能需要一些时间。"
              : status?.lastSuccessAt
                ? `最近同步 ${new Date(status.lastSuccessAt * 1000).toLocaleString("zh-CN")}`
                : "等待后台首次采集。")}
      </p>
      <p className="mt-1 text-[11px] leading-relaxed text-zinc-500">
        仅监测已添加工具，移除后停止采集和提醒。
        {tools.some((tool) => tool.key === "cursor") &&
          "Cursor 读取账号云端用量，可包含其他设备。"}
        {tools.some((tool) => tool.key === "antigravity") &&
          "Antigravity 读取本地会话并同步运行中的服务。"}
        额度使用官方登录查询，有数据才显示。
      </p>
    </div>
  );
}
