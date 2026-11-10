import { Loader2, RefreshCw, User } from "lucide-react";
import type { AiUsageItem } from "../../../../services/api";
import { AI_TOOL_CATALOG } from "../../services/aiToolCards";
import { localUsageSupported, quotaRows } from "../../services/aiStats";
import AgentQuotaMetrics from "../AgentQuotaMetrics";
import QuotaBar from "../QuotaBar";
import UsageCell from "./UsageCell";
import { planChipClass } from "./format";

// ================= 用量 / 套餐 / 账号面板（token-monitor 风格）=================

/** 详情页用量面板：来源、账号与套餐、额度快照、token 三格。 */
export default function UsagePanel({
  toolKey,
  item,
  loading,
  error,
  refreshing,
  onRefresh,
}: {
  toolKey: string;
  item?: AiUsageItem;
  loading: boolean;
  error: string | null;
  refreshing: boolean;
  onRefresh: () => void;
}) {
  if (!localUsageSupported(toolKey))
    return (
      <div className="rounded-lg border border-zinc-800 bg-zinc-950/40 p-3 text-xs">
        <span className="rounded bg-zinc-800 px-2 py-1 text-zinc-300">
          用量与额度未接入
        </span>
        <p className="mt-3 leading-relaxed text-zinc-500">
          保留快捷方式启动入口。当前没有已接入的账号、用量或剩余额度数据，请在官方应用查看。
        </p>
      </div>
    );
  const acc = item?.account;
  const rate = item?.rate;
  const scanned = item ? new Date(item.scannedAt * 1000) : null;

  return (
    <div className="tool-usage-panel rounded-lg border border-zinc-800 bg-zinc-950/40 p-2.5">
      {/* 标题行 */}
      <div className="mb-1.5 flex items-center gap-1.5">
        <span className="text-[10px] font-medium tracking-wide text-zinc-500">
          {toolKey === "chatgpt"
            ? "Codex 用量 · 套餐 · 账号"
            : toolKey === "deepseek"
              ? "DeepSeek Harness 本地用量"
              : "用量 · 套餐 · 账号"}
        </span>
        <span className="text-[9px] text-zinc-700">
          {scanned
            ? `本地数据 · ${scanned.toLocaleTimeString("zh-CN", { hour12: false, hour: "2-digit", minute: "2-digit" })}`
            : "本地数据"}
        </span>
        <button
          onClick={onRefresh}
          className="ml-auto rounded p-1 text-zinc-500 hover:bg-zinc-800 hover:text-zinc-200"
          data-tooltip="重新扫描本地用量"
          aria-label="重新扫描本地用量"
        >
          {refreshing ? (
            <Loader2 className="h-3 w-3 animate-spin" />
          ) : (
            <RefreshCw className="h-3 w-3" />
          )}
        </button>
      </div>

      {error ? (
        <div className="py-2 text-[11px] text-amber-400">
          {error}，请点击刷新重试。
        </div>
      ) : loading ? (
        <div className="flex items-center gap-2 py-2 text-[11px] text-zinc-500">
          <Loader2 className="h-3 w-3 animate-spin" />
          正在读取本地用量数据…
        </div>
      ) : !item || !item.usageAvailable ? (
        <div className="py-2 text-[11px] text-zinc-500">
          {item?.note || "暂无本地用量数据"}
          <AgentQuotaMetrics item={item} />
        </div>
      ) : (
        <>
          <p className="mb-2 text-[10px] leading-relaxed text-zinc-500">
            数据来源：
            {item.dataSource ||
              `${AI_TOOL_CATALOG.find((tool) => tool.key === toolKey)?.label ?? "工具"} 本地会话`}
            。
          </p>
          {/* 账号 + 套餐 */}
          <div className="mb-1.5 flex items-center gap-1.5 text-[11px]">
            <User className="h-3 w-3 shrink-0 text-zinc-500" />
            <span
              className="min-w-0 flex-1 truncate text-zinc-300"
              data-tooltip={acc?.email ?? undefined}
            >
              {acc?.email || "未发现本地账号数据"}
            </span>
            {acc?.planLabel && (
              <span
                className={`shrink-0 rounded px-1.5 py-0.5 text-[9px] font-medium ${planChipClass(acc?.plan ?? null)}`}
                data-tooltip={`套餐类型：${acc?.plan ?? "unknown"}`}
              >
                {acc.planLabel}
              </span>
            )}
          </div>

          {/* 本机日志快照，账号归属与有效性独立于当前登录账号。 */}
          {rate?.metrics?.length ? (
            <AgentQuotaMetrics item={item} />
          ) : (
            (rate?.primary || rate?.secondary) && (
              <div className="mb-1.5 space-y-1.5">
                <p className="text-[9px] text-zinc-500">
                  最近额度快照{rate?.accountVerified ? "" : " · 未核验账号"}
                </p>
                {quotaRows(item, Date.now() / 1000).map((row) => (
                  <QuotaBar key={row.id} variant="card" row={row} compact />
                ))}
              </div>
            )
          )}

          {/* token 用量三格 */}
          <div className="grid grid-cols-3 gap-1.5">
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
              sessions={item.all.sessions}
            />
          </div>

          <div
            className={`mt-1.5 text-[10px] ${item.complete ? "text-zinc-600" : "text-amber-400"}`}
            data-tooltip={item.note}
          >
            {item.note}
          </div>
        </>
      )}
    </div>
  );
}
