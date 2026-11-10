import { Plus, Sparkles, X } from "lucide-react";
import type { AiToolMeta } from "../../services/aiToolCards";
import { agentBotId } from "../../services/agentBots";
import AgentBot from "../../../../shared/ui/AgentBot";

/** 添加工具卡片：列出尚未加入卡组的工具。 */
export default function ToolPicker({
  tools,
  busy,
  onAdd,
  onClose,
}: {
  tools: readonly AiToolMeta[];
  busy: boolean;
  onAdd: (key: string) => void;
  onClose: () => void;
}) {
  return (
    <section
      id="ai-tool-picker"
      aria-label="添加工具卡片"
      className="ui-panel border p-4"
    >
      <div className="mb-3 flex items-center justify-between">
        <h2 className="text-sm font-medium">添加工具</h2>
        <button
          type="button"
          className="rounded-lg p-1.5 text-zinc-500 hover:bg-zinc-800"
          aria-label="关闭工具选择"
          onClick={onClose}
        >
          <X size={15} />
        </button>
      </div>
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
        {tools.map((tool) => (
          <button
            type="button"
            key={tool.key}
            aria-label={`添加 ${tool.label}`}
            disabled={busy}
            onClick={() => onAdd(tool.key)}
            className="flex items-center gap-2 rounded-xl border border-zinc-800 p-3 text-left text-xs hover:bg-zinc-800/50 disabled:opacity-40"
          >
            {agentBotId(tool.key) ? (
              <AgentBot bot={agentBotId(tool.key)} size={36} />
            ) : (
              <Sparkles size={24} className="m-1.5 text-blue-400" />
            )}
            <span className="flex-1">{tool.label}</span>
            <Plus size={13} className="text-zinc-500" />
          </button>
        ))}
      </div>
      {tools.length === 0 && (
        <p className="text-xs text-zinc-500">所有工具均已添加。</p>
      )}
    </section>
  );
}
