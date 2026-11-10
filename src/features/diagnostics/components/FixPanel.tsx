import {
  AlertTriangle,
  CheckCircle2,
  CircleDashed,
  Loader2,
  Navigation,
  Wrench,
} from "lucide-react";
import type { FixItem, FixState } from "./types";

/** 修复清单卡：逐项列出待处理问题及独立修复按钮 */
export default function FixPanel({
  done,
  fixes,
  fixStates,
  fixMsg,
  pass,
  onFix,
  navigate,
  previous = null,
}: {
  done: boolean;
  fixes: FixItem[];
  fixStates: Record<string, FixState>;
  fixMsg: Record<string, string>;
  pass: boolean;
  onFix: (item: FixItem) => void;
  navigate: (to: string) => void;
  /** 上次未通过的项：只在未检测时作为历史摘要展示 */
  previous?: { at: number; failures: string[] } | null;
}) {
  return (
    <div className="ui-panel rounded-2xl border border-zinc-800 bg-zinc-900/40 p-6">
      <div className="mb-4 flex items-center gap-2 text-sm font-medium">
        <Wrench className="h-4 w-4 text-zinc-400" />
        <h2>修复清单</h2>
        {done && fixes.length > 0 && (
          <span className="rounded-full bg-amber-500/15 px-2 py-0.5 text-[10px] font-medium text-amber-300">
            {fixes.length} 项待处理
          </span>
        )}
      </div>
      {!done ? (
        previous && previous.failures.length > 0 ? (
          <div className="space-y-2">
            <div className="flex items-start gap-2 text-xs text-zinc-500">
              <CircleDashed className="mt-0.5 h-3.5 w-3.5 shrink-0" />
              <span>
                上次检测有 {previous.failures.length} 项未通过，结论已过期；
                重新检测后按当前状态给出修复项。
              </span>
            </div>
            <details className="text-[11px] text-zinc-500">
              <summary className="cursor-pointer select-none hover:text-zinc-400">
                查看上次未通过项
              </summary>
              <ul className="mt-1.5 list-disc space-y-1 pl-4">
                {previous.failures.map((text) => (
                  <li key={text} className="break-words">
                    {text}
                  </li>
                ))}
              </ul>
            </details>
          </div>
        ) : (
          <div className="flex items-center gap-2 text-xs text-zinc-500">
            <CircleDashed className="h-3.5 w-3.5 shrink-0" />
            待检测，完成后显示需要处理的项目。
          </div>
        )
      ) : fixes.length === 0 ? (
        <div className="flex items-center gap-2 text-xs text-emerald-300">
          <CheckCircle2 className="h-3.5 w-3.5" />
          无需修复 —— 所有信号已对齐为干净的海外身份。
        </div>
      ) : (
        <div className="space-y-2">
          {fixes.map((f) => {
            const st = fixStates[f.key];
            return (
              <div
                key={f.key}
                className={`fix-item rounded-lg border px-3 py-2.5 ${
                  st === "error"
                    ? "border-red-500/40 bg-red-500/5"
                    : st === "done"
                      ? "border-emerald-500/40 bg-emerald-500/5"
                      : "bg-zinc-900/60"
                }`}
              >
                <div className="flex items-start justify-between gap-3">
                  <span className="flex min-w-0 items-start gap-1.5 text-xs text-zinc-300">
                    {st === "fixing" ? (
                      <Loader2 className="mt-0.5 h-3 w-3 shrink-0 animate-spin text-blue-400" />
                    ) : st === "done" ? (
                      <CheckCircle2 className="mt-0.5 h-3 w-3 shrink-0 text-emerald-400" />
                    ) : (
                      <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0 text-amber-400" />
                    )}
                    <span className="min-w-0">{f.text}</span>
                  </span>
                  <span className="flex shrink-0 items-center gap-1.5">
                    {st === "fixing" && (
                      <span className="text-[11px] text-blue-300">修复中…</span>
                    )}
                    {st === "done" && (
                      <span className="text-[11px] font-medium text-emerald-400">
                        已修复
                      </span>
                    )}
                    {st !== "fixing" &&
                      st !== "done" &&
                      f.actionLabel &&
                      f.run && (
                        <button
                          onClick={() => onFix(f)}
                          className="rounded-md btn-primary px-2.5 py-1 text-[11px] font-medium text-white hover:opacity-90"
                        >
                          {f.actionLabel}
                        </button>
                      )}
                    {st !== "fixing" &&
                      st !== "done" &&
                      (f.navigateTo || f.targetId) && (
                        <button
                          onClick={() => {
                            if (f.targetId)
                              document
                                .getElementById(f.targetId)
                                ?.scrollIntoView({
                                  behavior: "smooth",
                                  block: "center",
                                });
                            else navigate(f.navigateTo!);
                          }}
                          className="flex items-center rounded-md border border-zinc-700 px-2 py-1 text-[11px] text-zinc-300 hover:border-zinc-500"
                          data-tooltip={f.manualHint ?? ""}
                        >
                          <Navigation className="mr-1 h-3 w-3" />
                          前往
                        </button>
                      )}
                  </span>
                </div>
                {(fixMsg[f.key] ||
                  (st !== "fixing" && st !== "done" && f.manualHint)) && (
                  <div
                    className={`mt-1.5 text-[11px] ${
                      fixMsg[f.key] ? "text-red-300" : "text-zinc-500"
                    }`}
                  >
                    {fixMsg[f.key] || f.manualHint}
                  </div>
                )}
              </div>
            );
          })}
          {!pass && (
            <div className="text-[11px] text-zinc-500">
              自动修复后会复测；手动调整地区或时区后，请点击上方按钮重新检查。
            </div>
          )}
        </div>
      )}
    </div>
  );
}
