import UsageCollectorStatus from "./UsageCollectorStatus";
import { useState } from "react";
import {
  agentIntegrations,
  type AgentConnection as Connection,
} from "../services/agentIntegrations";
import { aiToolCards } from "../services/aiToolCards";
import { useAppStore } from "../../../store";

export default function AgentIntegrations({
  onChanged,
}: {
  onChanged: () => void;
}) {
  const tools = aiToolCards(useAppStore((state) => state.config));
  const [open, setOpen] = useState(false);
  const [connections, setConnections] = useState<Connection[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState("");
  const load = async () => {
    try {
      setConnections(await agentIntegrations.status());
      setError("");
    } catch {
      setError("连接状态读取失败，请重新展开重试。");
    }
  };
  const change = async (key: string, enabled: boolean) => {
    setBusy(key);
    try {
      setConnections(await agentIntegrations.connect(key, enabled));
      setError("");
      onChanged();
    } catch (failure) {
      setError(String(failure));
    } finally {
      setBusy(null);
    }
  };
  return (
    <details
      className="ui-panel border p-4"
      onToggle={(event) => {
        const expanded = event.currentTarget.open;
        setOpen(expanded);
        if (expanded) void load();
      }}
    >
      <summary className="cursor-pointer text-sm font-medium">
        工具连接 · 已添加 {tools.length} 款 Agent
      </summary>
      <p className="mt-3 text-xs leading-relaxed text-zinc-500">
        工作状态、Token 和额度分别读取。缺少数据时显示
        —；工具归属依据会话来源，模型单独展示。
      </p>
      <UsageCollectorStatus onChanged={onChanged} enabled={open} />
      <div
        className="mt-3 grid gap-2 sm:grid-cols-2 lg:grid-cols-3"
        aria-label="Agent 连接列表"
      >
        {tools.map((tool) => {
          const connection = connections?.find((item) => item.key === tool.key);
          return (
            <div
              key={tool.key}
              className="rounded-xl border border-zinc-800/60 p-3"
              data-integration-tool={tool.key}
            >
              <div className="flex items-center justify-between gap-2 text-xs">
                <strong>{tool.label}</strong>
                <span className="text-zinc-500">
                  {!connection
                    ? "读取中"
                    : connection.method === "local"
                      ? "自动读取"
                      : connection.connected
                        ? "已配置"
                        : "待连接"}
                </span>
              </div>
              <p className="mt-2 text-[11px] leading-relaxed text-zinc-500">
                {connection?.note ?? "正在检查本地连接…"}
              </p>
              {connection?.method === "hooks" && (
                <button
                  className="btn-secondary mt-2 rounded-lg px-2 py-1 text-xs disabled:opacity-50"
                  disabled={busy != null}
                  onClick={() => void change(tool.key, !connection.connected)}
                >
                  {busy === tool.key
                    ? "处理中…"
                    : connection.connected
                      ? "断开状态连接"
                      : "启用状态连接"}
                </button>
              )}
            </div>
          );
        })}
      </div>
      {error && (
        <p role="alert" className="mt-3 text-xs text-amber-500">
          {error}
        </p>
      )}
      <p className="mt-3 text-[11px] text-zinc-500">
        连接会先备份原有
        hooks，并保留其他规则。断开仅移除本软件的连接；配置变更在新会话生效。
      </p>
    </details>
  );
}
