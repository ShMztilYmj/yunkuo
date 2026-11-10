import { Info, Loader2, RefreshCw, ScanLine } from "lucide-react";
import type { LastScanSummary } from "../../../services/api";
import PageHeader from "../../../shared/ui/PageHeader";
import { relativeTime, STALE_TEXT, type StaleReason } from "../lastScan";

/** 状态行语气：就绪 / 需要处理 / 未出结论 */
type Tone = "ready" | "attention" | "pending";

/**
 * 检测页页头：内容交给共用 <PageHeader />，位置与尺寸和首页概览、指纹浏览器、
 * AI 完全一致。页头下面那行说明位固定占位：平时放提示，检测中放进度，
 * 出错时放报错，三种状态互相替换，页面高度不变。
 */
export default function DetectHeader({
  at,
  history,
  stale,
  scanning,
  detectScanning,
  computerBusy,
  scanError,
  pass,
  failures,
  signals,
  score,
  onRun,
}: {
  at: number | null;
  /** 上次检测摘要；只用于展示，不能当作本轮结论 */
  history?: LastScanSummary | null;
  stale?: StaleReason;
  scanning: boolean;
  detectScanning: boolean;
  computerBusy: boolean;
  scanError: string | null;
  /** 本轮门控是否全绿 */
  pass: boolean;
  /** 本轮待修复项数量（修复清单条数） */
  failures: number;
  /** 本轮浏览器信号数量 */
  signals: number;
  /** 信任评分：本轮优先，其次上次；没有就不显示徽标 */
  score: number | null;
  onRun: () => void;
}) {
  const busy = scanning || detectScanning;
  const round = at != null;
  const tone: Tone = busy
    ? "pending"
    : round
      ? pass
        ? "ready"
        : "attention"
      : stale
        ? "attention"
        : "pending";
  const status = busy
    ? "检测中"
    : round
      ? pass
        ? "就绪"
        : failures > 0
          ? `需处理 ${failures} 项`
          : "需要检查"
      : stale
        ? "结论已过期"
        : history
          ? "待重新检测"
          : "待检测";
  const stamp = at
    ? new Date(at).toLocaleTimeString("zh-CN", { hour12: false })
    : history
      ? relativeTime(history.at)
      : null;
  // 没有本轮结论时，历史时间点本身就是「重新检测」的入口。
  const rerunnable = !round && history != null;

  const note = scanError ? (
    <span className="is-error" role="alert">
      {scanError}
    </span>
  ) : busy ? (
    <span role="status">
      <Loader2 className="animate-spin" size={13} aria-hidden="true" />
      正在检测浏览器信号、DNS 深度测试与系统级状态…
    </span>
  ) : (
    <span>
      <Info size={13} aria-hidden="true" />
      干净稳定的国外住宅IP，苹果/谷歌的稳定支付是关键
    </span>
  );

  return (
    <PageHeader
      className="detect-head"
      icon={ScanLine}
      title="环境检测并修复"
      badge={score != null ? `${score} 分` : undefined}
      badgeTooltip={round ? "本轮信任评分" : "上次检测的信任评分（仅供参考）"}
      metaTone={tone}
      meta={
        <>
          {rerunnable ? (
            <button
              type="button"
              onClick={onRun}
              disabled={busy}
              className="detect-head-rerun"
              data-tooltip={`${stale ? `${STALE_TEXT[stale]}，` : ""}点此重新检测`}
            >
              上次检测 {stamp}
            </button>
          ) : stamp ? (
            <span>上次检测 {stamp}</span>
          ) : (
            <span>检查本机环境、网络出口与服务连通性</span>
          )}
          {round && signals > 0 && (
            <>
              <i className="proxy-meta-dot" aria-hidden="true" />
              <span>{signals} 项浏览器信号</span>
            </>
          )}
          <i className="proxy-meta-dot" aria-hidden="true" />
          <strong>{status}</strong>
        </>
      }
      actions={
        <button
          type="button"
          onClick={onRun}
          disabled={busy || computerBusy}
          aria-busy={busy}
          data-tooltip="检测浏览器信号、系统与网络出口，并自动修复可修复项"
          className="proxy-action detect-head-run"
        >
          {busy ? (
            <Loader2 className="animate-spin" />
          ) : (
            <RefreshCw aria-hidden="true" />
          )}
          {busy ? "检测中…" : "一键检测并修复"}
        </button>
      }
      note={note}
    />
  );
}
