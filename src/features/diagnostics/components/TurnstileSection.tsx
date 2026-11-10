import type { RefObject } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  Cloud,
  Loader2,
  RefreshCw,
} from "lucide-react";
import type { TurnItem, TurnState } from "../signals/turnstile";
import type { TurnstileMounts } from "./types";

/** 状态徽章：与 DNS 卡片共用一套色阶，避免每张卡自造颜色 */
const STATE_CHIP: Record<TurnState, { text: string; cls: string }> = {
  idle: {
    text: "待检测",
    cls: "border-zinc-700/70 bg-zinc-800/50 text-zinc-400",
  },
  running: {
    text: "运行中",
    cls: "border-blue-500/25 bg-blue-500/10 text-blue-400",
  },
  ok: {
    text: "自检完成",
    cls: "border-emerald-500/25 bg-emerald-500/10 text-emerald-400",
  },
  fail: {
    text: "失败",
    cls: "border-red-500/25 bg-red-500/10 text-red-400",
  },
};

/**
 * Cloudflare Turnstile 组件自检卡：托管 / 非交互两种模式各占一格。
 *
 * 版式要点：
 * - 挂载区按组件实际占位留高：65px 的组件 + 官方测试密钥自带的红底提示条。
 *   留矮了就会把提示条切掉半行，看起来像组件渲染坏了。
 * - 挂载容器必须是普通块级盒：flex 会让 flexible 组件退化成 300px 定宽。
 * - 非交互模式的组件是纯隐形的，容器里永远没有可见内容，
 *   因此由我们自己画一层说明，否则右边那格只剩一片空白。
 */
export default function TurnstileSection({
  items,
  running,
  onRun,
  mounts,
}: {
  items: TurnItem[];
  running: boolean;
  onRun: () => void;
  mounts: RefObject<TurnstileMounts>;
}) {
  return (
    <section
      data-turnstile-section
      aria-labelledby="turnstile-section-title"
      className="ui-panel rounded-2xl border border-zinc-800 bg-zinc-900/40 p-6"
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h2
            id="turnstile-section-title"
            className="flex items-center gap-2 text-sm font-medium"
          >
            <Cloud className="h-4 w-4 shrink-0 text-sky-400" />
            Cloudflare Turnstile 组件自检
          </h2>
          <p className="mt-1 text-[11px] leading-relaxed text-zinc-500">
            使用官方恒成功测试密钥，仅检查脚本与组件能否加载；不代表真实站点、账号或节点通过风控，不参与信任评分。
          </p>
        </div>
        <button
          type="button"
          onClick={onRun}
          disabled={running}
          className="btn-secondary ml-auto flex shrink-0 items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs disabled:opacity-50"
        >
          {running ? (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          ) : (
            <RefreshCw className="h-3.5 w-3.5" />
          )}
          {running ? "自检中…" : "重新自检"}
        </button>
      </div>

      <div className="mt-4 grid grid-cols-1 gap-3 md:grid-cols-2">
        {items.map((item) => {
          const chip = STATE_CHIP[item.state];
          // 非交互模式的组件不画任何东西，只能靠占位层说明；
          // 托管模式在组件真正注入容器后让位，避免文字压在组件上。
          const widgetMounted =
            (mounts.current[item.mode as "managed" | "nonInteractive"]
              ?.childElementCount ?? 0) > 0;
          const showOverlay =
            item.mode === "nonInteractive" ||
            item.state === "idle" ||
            item.state === "fail" ||
            (item.state === "running" && !widgetMounted);
          const failed = item.state === "fail";
          return (
            <div
              key={item.mode}
              data-turnstile-mode={item.mode}
              className="flex h-full flex-col rounded-xl border border-zinc-800/80 bg-zinc-900/60 p-4"
            >
              <div className="flex items-start justify-between gap-3">
                <span className="min-w-0 truncate text-xs font-medium text-zinc-200">
                  {item.label}
                </span>
                <span
                  className={`shrink-0 rounded-md border px-2 py-[3px] text-[10px] ${chip.cls}`}
                >
                  {chip.text}
                </span>
              </div>

              <div className="relative mt-3 h-[70px] overflow-hidden rounded-lg bg-zinc-950/40">
                {/* 挂载容器由 Turnstile 独占，React 不放子节点，避免与注入的 iframe 打架；
                    容器必须是普通块级盒：flex 会让 flexible 组件退化成 300px 定宽 */}
                <div
                  ref={(el) => {
                    mounts.current[item.mode as "managed" | "nonInteractive"] =
                      el;
                  }}
                  data-turnstile-host={item.mode}
                  className="block h-full w-full"
                />
                {showOverlay && <Overlay item={item} />}
              </div>

              <div className="mt-auto flex items-start justify-between gap-3 pt-2.5">
                <p
                  className={`min-w-0 text-[11px] leading-relaxed ${
                    failed ? "text-red-300" : "text-zinc-500"
                  }`}
                >
                  {item.detail}
                </p>
                {item.ms != null && (
                  <span className="shrink-0 font-mono text-[10px] tabular-nums text-zinc-600">
                    {item.ms} ms
                  </span>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </section>
  );
}

/** 挂载区占位层：只解释状态，不接收鼠标事件，也不挡 Turnstile 的组件 */
function Overlay({ item }: { item: TurnItem }) {
  const invisible = item.mode === "nonInteractive";
  const view =
    item.state === "ok"
      ? {
          icon: <CheckCircle2 className="h-3.5 w-3.5 text-emerald-400" />,
          text: invisible ? "隐形校验已通过" : "组件已渲染",
          sub: invisible ? "组件不渲染界面，只校验令牌能否拿到" : null,
          tone: "text-zinc-300",
          frame: "border-zinc-800 bg-zinc-900/60",
        }
      : item.state === "running"
        ? {
            icon: (
              <Loader2 className="h-3.5 w-3.5 animate-spin text-blue-400" />
            ),
            text: invisible ? "隐形校验中…" : "组件加载中…",
            sub: null,
            tone: "text-zinc-400",
            frame: "border-zinc-800 border-dashed bg-zinc-900/40",
          }
        : item.state === "fail"
          ? {
              icon: <AlertTriangle className="h-3.5 w-3.5 text-red-400" />,
              text: invisible ? "隐形校验未通过" : "未渲染组件",
              sub: null,
              tone: "text-zinc-400",
              frame: "border-red-500/25 bg-red-500/5",
            }
          : {
              icon: <Cloud className="h-3.5 w-3.5 text-zinc-600" />,
              text: "待检测",
              sub: null,
              tone: "text-zinc-500",
              frame: "border-zinc-800 border-dashed bg-zinc-900/40",
            };
  return (
    <div
      className={`pointer-events-none absolute inset-0 flex flex-col items-center justify-center gap-1 rounded-lg border px-3 text-center ${view.frame}`}
    >
      <span
        className={`flex items-center gap-1.5 text-[11px] leading-none ${view.tone}`}
      >
        {view.icon}
        {view.text}
      </span>
      {view.sub && (
        <span className="text-[10px] leading-none text-zinc-600">
          {view.sub}
        </span>
      )}
    </div>
  );
}
