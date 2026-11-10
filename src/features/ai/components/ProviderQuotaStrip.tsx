import type { AiUsageItem } from "../../../services/api";
import { useRemovedProviders } from "../hooks/useRemovedProviders";
import { toolQuotaView } from "../services/providerAccounts";
import type { ToolSetup } from "../services/toolSetup";
import ProviderQuotaPanel, { QuotaProviderLine } from "./ProviderQuotaPanel";

/**
 * 工具卡上的服务商额度条：Codex、Claude 与其它 Agent 共用这一份。
 *
 * 取数交给 `toolQuotaView`（账号额度优先，其次当前 / 上次使用的模型源），
 * 渲染交给 `ProviderQuotaPanel` 的 compact 形态；这里只补「还没有读数」的说法，
 * 让两种状态在卡面上的位置、字号和徽标完全一致。
 */
export default function ProviderQuotaStrip({
  toolKey,
  item,
  entry,
  emptyNote,
}: {
  toolKey: string;
  item?: AiUsageItem;
  entry?: ToolSetup;
  /** 没有额度读数时补一行数据来源，避免区块空着。 */
  emptyNote?: string;
}) {
  const removed = useRemovedProviders();
  const view = toolQuotaView(toolKey, item, entry, removed);
  const badge =
    view.state === "active"
      ? "使用中"
      : view.state === "last"
        ? "上次使用"
        : undefined;
  if (!view.quota)
    return (
      <>
        <QuotaProviderLine
          provider={view.provider}
          badge={badge}
          note="暂无额度快照"
        />
        {emptyNote && <p className="tool-card-overview-note">{emptyNote}</p>}
      </>
    );
  return (
    <ProviderQuotaPanel
      compact
      quota={view.quota}
      title={view.provider}
      badge={badge}
    />
  );
}
