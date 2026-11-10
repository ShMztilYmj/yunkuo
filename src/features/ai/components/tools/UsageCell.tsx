import { formatTokens } from "../../services/aiStats";

/** 单个 token 用量格（今日 / 本周 / 累计）。 */
export default function UsageCell({
  label,
  tokens,
  sessions,
}: {
  label: string;
  tokens: number;
  sessions?: number;
}) {
  return (
    <div
      className="min-w-0 rounded-md bg-zinc-900/80 px-2 py-1.5"
      data-tooltip={`${label} tokens：${tokens}`}
    >
      <div className="text-[10px] text-zinc-600">{label}</div>
      <div className="flex items-baseline gap-1 text-[11px] font-medium text-zinc-200">
        <span className="truncate">
          {tokens > 0 ? formatTokens(tokens) : "—"}
        </span>
        {sessions != null && sessions > 0 && (
          <span className="text-[9px] font-normal text-zinc-600">
            {sessions} 会话
          </span>
        )}
      </div>
    </div>
  );
}
