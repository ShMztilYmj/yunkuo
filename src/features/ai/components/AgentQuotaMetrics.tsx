import type { AiUsageItem } from "../../../services/api";
import { quotaRows } from "../services/aiStats";
import QuotaBar from "./QuotaBar";

/** 多账号额度指标：取数交给 quotaRows，逐行渲染交给 QuotaBar。 */
export default function AgentQuotaMetrics({
  item,
  compact = false,
}: {
  item?: AiUsageItem;
  compact?: boolean;
}) {
  // 只有官方账号查询才有多指标；旧时间窗走 QuotaBar 的 card 形态。
  if (!item?.rate.metrics?.length) return null;
  return (
    <div className={compact ? "space-y-2" : "space-y-3"}>
      {quotaRows(item, Date.now() / 1000).map((row) => (
        <QuotaBar key={row.id} variant="row" row={row} compact={compact} />
      ))}
    </div>
  );
}
