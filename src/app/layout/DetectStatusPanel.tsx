import { ScanLine } from "lucide-react";
import { useLocation, useNavigate } from "react-router-dom";
import { useEnvironmentVerdict } from "../../features/diagnostics/hooks/useEnvironmentVerdict";
import { useAppStore } from "../../store";

/** 侧边栏「检测」卡片：标题行（点击进检测页）+ 主要检测结果，与「代理」卡片同款 */
export default function DetectStatusPanel() {
  const nav = useNavigate();
  const loc = useLocation();
  const detect = useAppStore((s) => s.detect);
  const onDetectRoute = ["/detect", "/diagnostics"].includes(loc.pathname);

  const nd = detect.native;
  const { pass } = useEnvironmentVerdict();
  const running = detect.scanning;
  const scanned = detect.finished && nd != null;

  const statusDot = running
    ? "bg-blue-400"
    : pass
      ? "bg-emerald-400"
      : scanned
        ? "bg-amber-400"
        : "bg-zinc-500";
  const statusText = running
    ? "检测中"
    : pass
      ? "通过"
      : scanned
        ? "待修复"
        : "未检测";

  const summary = scanned
    ? [nd.exitCountry ?? "未知地区", nd.currentTzIana || "未知时区"].join(" · ")
    : running
      ? "正在采集环境信号…"
      : "检查网络与本机环境";
  return (
    <button
      onClick={() => nav("/detect")}
      aria-current={onDetectRoute ? "page" : undefined}
      aria-label="检测与修复"
      data-tooltip={`检测与修复 · ${statusText} · ${summary}`}
      className={
        "detect-nav sidebar-link " + (onDetectRoute ? "is-active" : "")
      }
    >
      <ScanLine size={19} className="sidebar-icon" />
      <span className="sidebar-copy min-w-0 flex-1">
        <span className="block">检测与修复</span>
        {onDetectRoute && (
          <span className="mt-1 block truncate text-[10px] font-normal text-zinc-500">
            {summary}
          </span>
        )}
      </span>
      <span className="sidebar-trailing flex shrink-0 items-center gap-1 text-[10px] font-normal text-zinc-500">
        <span className={"h-1.5 w-1.5 rounded-full " + statusDot} />
        {statusText}
      </span>
      <span aria-hidden="true" className={`sidebar-compact-dot ${statusDot}`} />
    </button>
  );
}
