import type { SignalResult } from "../signals/web";

/** 未检测时的信号占位行 */
export function PendingSignalRow({ name }: { name: string }) {
  return (
    <div className="flex items-center gap-2.5 rounded-lg border border-zinc-800/70 bg-zinc-900/40 px-2.5 py-1.5">
      <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-zinc-500" />
      <span className="w-[74px] shrink-0 text-[11.5px] text-zinc-300">
        {name}
      </span>
      <span className="flex-1 text-[10.5px] text-zinc-500">待检测</span>
    </div>
  );
}

/**
 * 单项浏览器环境信号。
 *
 * 按「状态点 / 名称 / 实测值 / 计分」四列排一行，值可截断、完整内容走 tooltip；
 * 卡片里用两列自适应网格摆放，避免整行横跨时中间留出大片空白。
 */
export function SignalRow({ result }: { result: SignalResult }) {
  const unknown = result.status === "unknown";
  return (
    <div className="flex min-w-0 items-center gap-2.5 rounded-lg border border-zinc-800/70 bg-zinc-900/40 px-2.5 py-1.5">
      <div
        className={`h-1.5 w-1.5 shrink-0 rounded-full ${
          unknown
            ? "bg-amber-500"
            : result.band === "high"
              ? "bg-red-500"
              : result.band === "medium"
                ? "bg-amber-500"
                : "bg-emerald-500"
        }`}
      />
      <div className="w-[74px] shrink-0 text-[11.5px] text-zinc-300">
        {result.name}
      </div>
      <div
        className="min-w-0 flex-1 truncate text-[10.5px] text-zinc-500"
        data-tooltip={result.raw}
      >
        {result.raw}
      </div>
      <div
        className={`w-9 shrink-0 text-right text-[10.5px] font-medium tabular-nums ${
          result.contribution > 0 ? "text-red-400" : "text-zinc-600"
        }`}
      >
        {unknown ? "未核验" : `+${result.contribution}`}
      </div>
    </div>
  );
}
