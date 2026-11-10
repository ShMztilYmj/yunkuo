import { Sparkles } from "lucide-react";
import type { AiUsageItem } from "../../../../services/api";
import { localUsageSupported } from "../../services/aiStats";
import type { ToolSetup } from "../../services/toolSetup";
import ProviderQuotaStrip from "../ProviderQuotaStrip";
import UsageCell from "./UsageCell";

/** 卡面概览：未接入 / 读取中 / 无用量 / 三格 token + 服务商额度。 */
export default function CardOverview({
  toolKey,
  item,
  entry,
  loading,
  error,
}: {
  toolKey: string;
  item?: AiUsageItem;
  entry?: ToolSetup;
  loading: boolean;
  error: string | null;
}) {
  if (!localUsageSupported(toolKey))
    return (
      <div className="tool-card-overview tool-card-empty">
        <Sparkles size={17} />
        <span>为你的工具留一个位置</span>
        <p>
          配置启动入口后即可使用
          <br />
          用量与额度暂未接入
        </p>
      </div>
    );
  if (!item || loading || error)
    return (
      <div className="tool-card-overview tool-card-empty">
        <span>
          {error
            ? "本地数据暂不可用"
            : loading
              ? "正在读取本地用量…"
              : "等待下一次灵感"}
        </span>
        <p>{error ? "在详情中刷新重试" : "使用后，在这里查看本地用量"}</p>
      </div>
    );
  if (!item.usageAvailable)
    return (
      <div className="tool-card-overview tool-card-empty">
        <span>
          {item.sessionAvailable ? "工作状态已连接 · 用量 —" : "暂无本地用量"}
        </span>
        <p>{item.note || "使用后刷新，查看可核验的本地记录。"}</p>
        <ProviderQuotaStrip toolKey={toolKey} item={item} entry={entry} />
      </div>
    );
  return (
    <div className="tool-card-overview">
      <div className="tool-card-metrics">
        <UsageCell
          label={item.complete ? "今日" : "今日（部分）"}
          tokens={item.today.total}
        />
        <UsageCell
          label={item.complete ? "本周" : "本周（部分）"}
          tokens={item.week.total}
        />
        <UsageCell
          label={item.complete ? "累计" : "累计（部分）"}
          tokens={item.all.total}
        />
      </div>
      <div className="tool-card-quotas">
        <ProviderQuotaStrip
          toolKey={toolKey}
          item={item}
          entry={entry}
          emptyNote={
            item.complete ? item.dataSource : "部分记录未能完整统计 · 详情查看"
          }
        />
      </div>
    </div>
  );
}
