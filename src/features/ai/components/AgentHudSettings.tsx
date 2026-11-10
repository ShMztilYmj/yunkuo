import { useEffect, useState } from "react";
import { Loader2, PanelTop, RotateCcw } from "lucide-react";
import { useTauriEvent } from "../../../shared/hooks/useTauriEvent";
import { agentHud, type AgentHudStatus } from "../services/agentHud";
import AgentBot from "../../../shared/ui/AgentBot";
import { aiToolCards } from "../services/aiToolCards";
import { useAppStore } from "../../../store";
import { agentBotId } from "../services/agentBots";

export default function AgentHudSettings() {
  const config = useAppStore((state) => state.config);
  const [status, setStatus] = useState<AgentHudStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useTauriEvent<AgentHudStatus>("agent-hud-changed", (payload) =>
    setStatus(payload),
  );
  useEffect(() => {
    let active = true;
    void agentHud
      .status()
      .then((value) => {
        if (active) setStatus(value);
      })
      .catch((error) => {
        if (active) setError(String(error));
      });
    return () => {
      active = false;
    };
  }, []);
  const run = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError("");
    try {
      await action();
      setStatus(await agentHud.status());
    } catch (error) {
      setError(String(error));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section
      className="ui-panel space-y-5 border p-5"
      aria-label="Agent 悬浮条设置"
    >
      <div>
        <h2 className="flex items-center gap-2 text-sm font-semibold">
          <PanelTop size={17} />
          Agent 悬浮条
        </h2>
        <p className="mt-2 text-xs leading-6 text-zinc-500">
          在其他软件上方查看 Agent
          状态，鼠标移上去展开今日用量、额度和最近会话。主界面收起后仍可使用。
        </p>
      </div>
      <div
        className="flex w-fit max-w-full items-center rounded-2xl border border-zinc-800 bg-zinc-950/40 px-2 py-1"
        aria-hidden="true"
      >
        {aiToolCards(config).map((tool) => (
          <AgentBot key={tool.key} bot={agentBotId(tool.key)} size={40} />
        ))}
      </div>
      {!status && !error && (
        <p
          role="status"
          className="flex items-center gap-2 text-xs text-zinc-500"
        >
          <Loader2 size={13} className="animate-spin" />
          正在读取悬浮条设置…
        </p>
      )}
      <div className="flex items-center justify-between gap-4">
        <div>
          <p className="text-sm">显示悬浮条</p>
          <p className="mt-1 text-xs text-zinc-500">
            拖动左侧手柄调整位置，靠近屏幕边缘时自动贴边。
          </p>
        </div>
        <button
          role="switch"
          aria-label="显示 Agent 悬浮条"
          aria-checked={status?.enabled ?? false}
          disabled={busy || !status}
          onClick={() => void run(() => agentHud.enabled(!status!.enabled))}
          className={`relative h-5 w-9 shrink-0 rounded-full disabled:opacity-40 ${status?.enabled ? "bg-emerald-500" : "bg-zinc-700"}`}
        >
          <span
            className={`absolute top-0.5 h-4 w-4 rounded-full bg-white shadow-sm ${status?.enabled ? "right-0.5" : "left-0.5"}`}
          />
        </button>
      </div>
      <div className="flex items-center justify-between gap-4 border-t border-zinc-800 pt-4">
        <div>
          <p className="text-sm">始终置顶</p>
          <p className="mt-1 text-xs text-zinc-500">
            保持在普通窗口上方，不抢占输入焦点。
          </p>
        </div>
        <button
          role="switch"
          aria-label="悬浮条始终置顶"
          aria-checked={status?.alwaysOnTop ?? true}
          disabled={busy || !status}
          onClick={() => void run(() => agentHud.top(!status!.alwaysOnTop))}
          className={`relative h-5 w-9 shrink-0 rounded-full disabled:opacity-40 ${status?.alwaysOnTop ? "bg-emerald-500" : "bg-zinc-700"}`}
        >
          <span
            className={`absolute top-0.5 h-4 w-4 rounded-full bg-white shadow-sm ${status?.alwaysOnTop ? "right-0.5" : "left-0.5"}`}
          />
        </button>
      </div>
      <div className="flex items-center justify-between gap-4 border-t border-zinc-800 pt-4">
        <div>
          <p className="text-sm">自动吸边</p>
          <p className="mt-1 text-xs leading-6 text-zinc-500">
            拖到屏幕顶部、左侧或右侧后自动停靠。左右停靠时竖排显示，详情向屏幕内侧展开；关闭后自由放置。
          </p>
        </div>
        <button
          role="switch"
          aria-label="悬浮条自动吸边"
          aria-checked={status?.autoDockTop ?? true}
          disabled={busy || !status}
          onClick={() =>
            void run(() =>
              agentHud.options({ autoDockTop: !(status?.autoDockTop ?? true) }),
            )
          }
          className={`relative h-5 w-9 shrink-0 rounded-full disabled:opacity-40 ${status?.autoDockTop !== false ? "bg-emerald-500" : "bg-zinc-700"}`}
        >
          <span
            className={`absolute top-0.5 h-4 w-4 rounded-full bg-white shadow-sm ${status?.autoDockTop !== false ? "right-0.5" : "left-0.5"}`}
          />
        </button>
      </div>
      <div className="flex items-center justify-between gap-4 border-t border-zinc-800 pt-4">
        <div>
          <label htmlFor="hud-tool-scope" className="text-sm">
            图标显示范围
          </label>
          <p className="mt-1 text-xs leading-6 text-zinc-500">
            “已添加工具”与 AI
            启动工具中的卡片一致；“当前在线”仅显示其中已检测到运行进程的工具。Agent
            工作状态由会话事件判断。
          </p>
        </div>
        <select
          id="hud-tool-scope"
          aria-label="悬浮条显示范围"
          value={status?.toolScope ?? "installed"}
          disabled={busy || !status}
          onChange={(event) =>
            void run(() =>
              agentHud.options({
                toolScope: event.target.value as "online" | "installed",
              }),
            )
          }
          className="ui-input shrink-0 rounded-lg border px-3 py-2 text-xs disabled:opacity-40"
        >
          <option value="online">仅显示当前在线</option>
          <option value="installed">显示所有已添加工具</option>
        </select>
      </div>
      <button
        className="btn-secondary flex items-center gap-2 rounded-lg px-3 py-2 text-xs disabled:opacity-40"
        disabled={busy || !status?.enabled}
        onClick={() => void run(agentHud.reset)}
      >
        <RotateCcw size={13} />
        恢复到主屏顶部
      </button>
      <p className="text-[11px] leading-6 text-zinc-500">
        显示范围、吸附方向、位置与开关保存在本机，下次启动自动恢复；也可在悬浮窗展开后的顶部切换。关闭悬浮条会停止刷新。Claude
        Code、Codex
        的工作状态来自现有会话记录；其他工具只识别安装和运行状态，不推测用量、登录或审批状态。
      </p>
      {error && (
        <div role="alert" className="text-xs text-red-400">
          {error}
          <button
            className="ml-2 underline"
            onClick={() => void run(agentHud.status)}
          >
            重试
          </button>
        </div>
      )}
    </section>
  );
}
