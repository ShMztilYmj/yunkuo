import { Sparkles } from "lucide-react";
import type { AiUsageItem } from "../../../../services/api";
import {
  agentBotId,
  botStateForUsage,
  type AgentBotState,
} from "../../services/agentBots";
import type { AiToolMeta } from "../../services/aiToolCards";
import { connectionLine, type ToolSetup } from "../../services/toolSetup";
import AgentBot from "../../../../shared/ui/AgentBot";
import CardOverview from "./CardOverview";

/** 卡面：机器人形象 + 出品方 + 名称 + 连接线说明 + 用量概览。 */
export default function ToolCardFace({
  meta,
  maker,
  busy,
  launchState,
  item,
  entry,
  setupFailure,
  showDetails,
  usageLoading,
  usageError,
}: {
  meta: AiToolMeta;
  maker: string;
  busy: boolean;
  launchState?: AgentBotState;
  item?: AiUsageItem;
  entry?: ToolSetup;
  setupFailure: string;
  showDetails: boolean;
  usageLoading: boolean;
  usageError: string | null;
}) {
  return (
    <div className="tool-card-face" hidden={showDetails}>
      <div className="tool-card-hero">
        <div className="tool-card-bot-stage">
          {agentBotId(meta.key) ? (
            <AgentBot
              bot={agentBotId(meta.key)}
              size={100}
              state={
                busy
                  ? "working"
                  : launchState === "asking"
                    ? "asking"
                    : botStateForUsage(item, Date.now() / 1000)
              }
            />
          ) : (
            <Sparkles size={52} strokeWidth={1.3} />
          )}
        </div>
        <p className="tool-card-maker">{maker}</p>
        <h2>{meta.label}</h2>
        <p
          className="tool-card-tagline"
          data-tooltip={
            entry ? entry.connection.note : setupFailure || meta.hint
          }
        >
          {setupFailure
            ? "配置读取失败 · 详情中重试"
            : connectionLine(entry?.connection)}
        </p>
      </div>
      <CardOverview
        toolKey={meta.key}
        item={item}
        entry={entry}
        loading={usageLoading}
        error={usageError}
      />
    </div>
  );
}
