import { AlertTriangle, Loader2, ScanLine, Square } from "lucide-react";
import {
  DNS_DEEP_PER_ROUND,
  DNS_DEEP_ROUNDS,
  DNS_DEEP_TOTAL,
  domesticResolverCount,
  type DnsDeepProgress,
} from "../signals/dnsDeep";
import type { DnsDeepReport } from "../../../services/api";
import CcBadge from "./CcBadge";

/**
 * DNS 深度测试面板：状态由 `useDnsDeep` 持有，一键检测与按钮共用同一份结果。
 * 结论只有两种——出现境内解析器要复核，或者没出现；「拿不到解析器」不算通过。
 *
 * 版式：左边操作、出口摘要与结论，右边解析器明细。明细用两行一组的紧凑清单
 * 而不是表格：IP + 归属/服务商一行、判定结果右对齐，窄栏下也不会被截断。
 */
export default function DnsDeepPanel({
  report,
  busy,
  progress,
  message,
  at,
  exitIp,
  exitCountryName,
  onRun,
  onStop,
}: {
  report: DnsDeepReport | null;
  busy: boolean;
  progress: DnsDeepProgress | null;
  message: string | null;
  at: number | null;
  exitIp: string | null;
  exitCountryName: string | null;
  onRun: () => void;
  onStop: () => void;
}) {
  const total = progress?.total ?? DNS_DEEP_TOTAL;
  const label =
    progress?.phase === "preparing"
      ? "正在准备权威观察会话…"
      : progress?.phase === "collecting"
        ? `${total} 次请求已尝试，正在等待权威观察结果…`
        : `正在进行第 ${progress?.round ?? 0} / ${progress?.rounds ?? DNS_DEEP_ROUNDS} 轮探测…`;
  const resolvers = report?.resolvers ?? [];
  const domestic = domesticResolverCount(report);

  return (
    <div className="mt-3 flex flex-col gap-3">
      <div className="grid items-start gap-4 lg:grid-cols-[minmax(0,0.86fr)_minmax(0,1.14fr)]">
        {/* 左：深度测试的操作、出口摘要与结论 */}
        <div className="flex min-w-0 flex-col items-center gap-2.5">
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={onRun}
              disabled={busy}
              className="flex items-center gap-1.5 rounded-lg btn-primary px-5 py-1.5 text-[13px] font-medium text-white disabled:opacity-60"
            >
              {busy ? (
                <Loader2 size={14} className="animate-spin" />
              ) : (
                <ScanLine size={14} />
              )}
              {busy ? "深度测试中…" : "深度测试"}
            </button>
            {busy && (
              <button
                type="button"
                onClick={onStop}
                className="flex items-center gap-1 rounded-lg bg-zinc-800 px-3 py-1.5 text-[12px] text-zinc-200 hover:bg-zinc-700"
              >
                <Square size={11} />
                停止
              </button>
            )}
          </div>
          <p className="text-center text-[10px] leading-4 text-zinc-600">
            {DNS_DEEP_ROUNDS} 轮 × {DNS_DEEP_PER_ROUND} 个新域名，约 12
            秒；经当前软件 WebView 的真实路径发起
          </p>

          {busy && (
            <div
              role="status"
              className="w-full space-y-1.5 text-xs text-zinc-500"
            >
              <p className="text-center text-[10.5px]">{label}</p>
              <div
                role="progressbar"
                aria-label="深度 DNS 请求进度"
                aria-valuemin={0}
                aria-valuemax={total}
                aria-valuenow={progress?.attempted ?? 0}
                className="h-1.5 overflow-hidden rounded-full bg-zinc-800"
              >
                <div
                  className="h-full rounded-full bg-emerald-400"
                  style={{
                    width: `${((progress?.attempted ?? 0) / total) * 100}%`,
                  }}
                />
              </div>
              <p className="text-center text-[9.5px]">
                已尝试 {progress?.attempted ?? 0} / {total} 个唯一域名
              </p>
            </div>
          )}

          <p className="text-center text-[11.5px] leading-5 text-zinc-400">
            出口 IP{" "}
            <span className="font-mono text-zinc-200 select-text">
              {exitIp || "未知"}
            </span>
            {exitCountryName ? (
              <span className="ml-1 text-zinc-500">（{exitCountryName}）</span>
            ) : null}
          </p>

          {message && (
            <div
              role="alert"
              className="flex w-full items-start gap-2 rounded-lg bg-amber-500/10 px-3 py-2 text-[11px] text-amber-300"
            >
              <AlertTriangle size={13} className="mt-0.5 shrink-0" />
              <span className="break-words select-text">{message}</span>
            </div>
          )}

          {/* 结论横幅 */}
          {report && (
            <div
              role="status"
              className={`w-full rounded-lg px-3 py-2 text-center text-[12.5px] font-medium ${
                domestic > 0
                  ? "bg-red-500/10 text-red-300"
                  : resolvers.length > 0
                    ? "bg-emerald-500/10 text-emerald-300"
                    : "bg-zinc-800/60 text-zinc-300"
              }`}
            >
              {domestic > 0
                ? `⚠ 发现 ${domestic} 个境内 DNS 解析器，可能泄露`
                : resolvers.length > 0
                  ? "✓ 完美！未检测到 DNS 泄露"
                  : "本轮未取得解析器，不能判定为通过"}
            </div>
          )}
          {report?.detail && (
            <p className="text-center text-[10px] leading-4 text-zinc-500">
              {report.detail}
            </p>
          )}
          {at && (
            <p className="text-center text-[9.5px] text-zinc-600">
              本次采样：
              {new Date(at).toLocaleTimeString("zh-CN", { hour12: false })}
            </p>
          )}
        </div>

        {/* 右：解析器明细 */}
        <div className="flex min-w-0 flex-col gap-1.5">
          {resolvers.length > 0 ? (
            <>
              <div className="flex items-baseline justify-between gap-2 px-0.5 text-[10px] text-zinc-500">
                <span>解析器明细</span>
                <span className="tabular-nums">
                  共 {resolvers.length} 个
                  {report?.filtered ? ` · 过滤 ${report.filtered} 条` : ""}
                </span>
              </div>
              <ul className="max-h-[232px] overflow-y-auto rounded-xl border border-zinc-800 bg-zinc-900/30">
                {resolvers.map((row) => {
                  const state =
                    row.cc === "CN"
                      ? { label: "需复核", tone: "text-amber-400" }
                      : row.cc
                        ? { label: "正确", tone: "text-emerald-400" }
                        : { label: "归属未知", tone: "text-zinc-500" };
                  const meta = [row.country, row.city, row.operator]
                    .filter(Boolean)
                    .join(" · ");
                  return (
                    <li
                      key={row.ip}
                      className={`flex items-center gap-2 border-b border-zinc-800/60 px-2.5 py-1.5 transition-colors last:border-b-0 hover:bg-zinc-800/40 ${
                        row.cc === "CN" ? "bg-amber-500/5" : ""
                      }`}
                    >
                      <CcBadge cc={row.cc} />
                      <div className="min-w-0 flex-1">
                        <div className="flex items-baseline gap-1.5">
                          <span className="truncate font-mono text-[11.5px] text-zinc-200 select-text">
                            {row.ip}
                          </span>
                          <span className="shrink-0 text-[9px] text-zinc-600">
                            {row.family}
                          </span>
                        </div>
                        <p
                          className="truncate text-[9.5px] leading-4 text-zinc-500"
                          title={meta || undefined}
                        >
                          {meta || "归属未知"}
                        </p>
                      </div>
                      <span
                        className={`shrink-0 text-[10px] font-medium ${state.tone}`}
                      >
                        {state.label}
                      </span>
                    </li>
                  );
                })}
              </ul>
            </>
          ) : (
            <div className="flex min-h-[128px] items-center justify-center rounded-xl border border-dashed border-zinc-800 px-6 py-5 text-center text-[10.5px] leading-5 text-zinc-600">
              {busy
                ? "正在收集解析器…"
                : report
                  ? "本轮未取得解析器。"
                  : "运行深度测试后，解析器明细会列在这里。"}
            </div>
          )}
        </div>

        <p className="text-[9.5px] leading-relaxed text-zinc-600 lg:col-span-2">
          第三方权威观察服务会看到本次请求及解析器地址。浏览器按自身策略查询
          A/AAAA；解析器使用 IPv6 不代表本机 IPv6
          绕行。这里的结果不代替指纹浏览器、Claude 或整机网络的验收。
        </p>
      </div>
    </div>
  );
}
