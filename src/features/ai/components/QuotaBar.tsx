import {
  quotaResetText,
  resetCountdown,
  type QuotaRow,
} from "../services/aiStats";

export type QuotaBarVariant = "card" | "row" | "hud";

/**
 * 额度条的唯一渲染入口。
 *
 * 数据一律来自 `quotaRows()`：`variant` 只决定排版，不含任何取数逻辑。
 * - `card`：统计页单窗口卡片；`compact` 为工具卡上的紧凑条（原 RateBar）。
 * - `row`：多账号指标行（原 AgentQuotaMetrics 的单行）；`compact` 用于卡面。
 * - `hud`：悬浮条详情（原 HudAgentDetails 的额度块）。
 *
 * className、aria-*、data-* 与合并前逐字一致，便于截图对比与 a11y 回归。
 */
export default function QuotaBar({
  variant,
  row,
  name,
  compact = false,
}: {
  variant: QuotaBarVariant;
  row: QuotaRow;
  /// 工具显示名，用于拼 aria 文本；card / hud 需要。
  name?: string;
  /// card / row 的紧凑形态。
  compact?: boolean;
}) {
  if (variant === "row") return <MetricRow row={row} compact={compact} />;
  if (variant === "hud") return <HudBar row={row} name={name} />;
  return compact ? <CompactBar row={row} /> : <CardBar row={row} name={name} />;
}

/// 统计页的单窗口卡片：剩余百分比 + 进度条 + 已用/重置/采样三行信息。
function CardBar({ row, name }: { row: QuotaRow; name?: string }) {
  const shown = row.used == null ? null : Math.round(row.used);
  const label = row.cardLabel;
  return (
    <div className={`ai-quota ${row.tone}`}>
      <div className="mb-1.5 flex flex-wrap items-center justify-between gap-1 text-[11px]">
        <span className="text-zinc-500">{label}</span>
        <strong className="font-semibold">
          {row.remaining == null
            ? "额度未知"
            : `剩余 ${Math.round(row.remaining)}%`}
          {row.stale ? " · 快照已过期" : ""}
        </strong>
      </div>
      <div
        role="progressbar"
        aria-label={`${name ?? ""}${row.slot}${label}已用`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={shown == null ? undefined : Math.min(100, shown)}
        aria-valuetext={
          shown == null
            ? "未知"
            : `已用 ${shown}%${row.stale ? "，快照已过期" : ""}`
        }
        className="ai-quota-track"
      >
        <div style={{ width: `${Math.min(100, row.used ?? 0)}%` }} />
      </div>
      <div className="mt-1 flex flex-wrap justify-between gap-1 text-[10px] text-zinc-500">
        <span>
          {shown == null
            ? "暂无读数"
            : (row.used ?? 0) > 100
              ? `已超额 · ${shown}%`
              : `已用 ${shown}%`}{" "}
          · {quotaResetText(row.seconds, row.resetPassed, "card")}
        </span>
        <span>
          {row.capturedAt
            ? `采样 ${new Date(row.capturedAt * 1000).toLocaleString("zh-CN", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" })}`
            : "采样时间未知"}
        </span>
      </div>
    </div>
  );
}

/// 工具卡上的紧凑条：只有一行标题和一条细进度条。
function CompactBar({ row }: { row: QuotaRow }) {
  const percent = row.percent;
  const shown = Math.min(100, Math.round(percent));
  // 非有限利用率落到最后一档，仍按绿色渲染：保留旧 RateBar 的观感。
  const color = !Number.isFinite(percent)
    ? "bg-emerald-400"
    : row.tone === "muted"
      ? "bg-zinc-500"
      : row.tone === "danger"
        ? percent > 100
          ? "bg-red-500"
          : "bg-red-400"
        : row.tone === "warning"
          ? "bg-amber-400"
          : "bg-emerald-400";
  return (
    <div className="tool-rate-bar">
      <div className="tool-rate-heading flex items-center justify-between text-[10px] text-zinc-500">
        <span>
          {row.barLabel}
          {row.stale ? " · 快照已过期" : ""}
        </span>
        <span>{`${percent > 100 ? "已超额" : `${shown}%`} · ${quotaResetText(row.seconds, row.resetPassed, "short")}`}</span>
      </div>
      <div className="tool-rate-track mt-0.5 h-1 overflow-hidden rounded-full bg-zinc-800">
        <div
          className={`h-full rounded-full ${color}`}
          style={{ width: `${Math.min(100, Math.max(2, percent))}%` }}
        />
      </div>
    </div>
  );
}

/// 多账号指标行：窗口名 + 剩余/重置结论 + 细进度条 + 采样时刻。
function MetricRow({ row, compact }: { row: QuotaRow; compact: boolean }) {
  const used = row.used;
  const remaining =
    row.remainingLabel ||
    (row.remaining == null ? "额度未知" : `剩余 ${Math.round(row.remaining)}%`);
  return (
    <div data-quota-metric={row.label}>
      <div className="mb-1 flex items-center justify-between gap-2 text-[11px]">
        <span
          className="min-w-0 truncate text-zinc-500"
          title={[row.account, row.plan].filter(Boolean).join(" · ")}
        >
          {row.label}
          {row.account && !compact ? ` · ${row.account}` : ""}
        </span>
        <strong className="shrink-0 font-medium">
          {row.stale
            ? "快照已过期"
            : row.resetPassed
              ? "重置时间已到，待新快照"
              : remaining}
        </strong>
      </div>
      <div
        className="h-1.5 overflow-hidden rounded-full bg-zinc-500/10"
        role="progressbar"
        aria-label={`${row.label} 已用额度`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={
          row.stale || used == null ? undefined : Math.min(100, used)
        }
      >
        <div
          className={`h-full rounded-full transition-[width] duration-500 ${row.stale ? "bg-zinc-400/40" : (used ?? 0) >= 90 ? "bg-amber-500" : "bg-emerald-500/80"}`}
          style={{ width: `${Math.min(100, Math.max(0, used ?? 0))}%` }}
        />
      </div>
      {!compact && (
        <p className="mt-1 text-[10px] text-zinc-500">
          {row.resetAt
            ? `${new Date(row.resetAt * 1000).toLocaleString("zh-CN", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" })} 重置 · `
            : ""}
          采样{" "}
          {new Date(row.capturedAt * 1000).toLocaleTimeString("zh-CN", {
            hour: "2-digit",
            minute: "2-digit",
          })}
        </p>
      )}
    </div>
  );
}

/// 悬浮条的额度块：只关心“多久后重置”，过期的读数降级成历史值。
function HudBar({ row, name }: { row: QuotaRow; name?: string }) {
  const fresh = !row.stale && !row.resetPassed && row.used != null;
  return (
    <div className={`hud-quota ${row.tone}`} data-quota={row.label}>
      <div className="hud-quota-title">
        <span>{row.label}</span>
        {row.account && <small title={row.account}>{row.account}</small>}
      </div>
      <div className="hud-quota-value">
        <strong>{row.used == null ? "—" : `${Math.round(row.used)}%`}</strong>
        <span>
          {fresh ? "已用" : row.used == null ? "额度未知" : "历史已用"}
        </span>
        <small>
          {fresh ? resetCountdown(row.seconds) : "额度快照已过期，等待更新"}
        </small>
      </div>
      <div
        className="hud-quota-track"
        role={fresh ? "meter" : "img"}
        aria-label={`${name ?? ""} ${row.label}额度已用`}
        aria-valuenow={fresh ? Math.min(100, Math.round(row.used!)) : undefined}
        aria-valuemin={fresh ? 0 : undefined}
        aria-valuemax={fresh ? 100 : undefined}
      >
        <i style={{ width: `${Math.min(100, row.used ?? 0)}%` }} />
      </div>
      {row.remainingLabel && (
        <p className="hud-quota-balance">{row.remainingLabel}</p>
      )}
    </div>
  );
}
