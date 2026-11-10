import { CheckCircle2, Loader2, Play } from "lucide-react";
import { Link } from "react-router-dom";
import type { AiToolMeta } from "../../services/aiToolCards";
import type { ToolSetup } from "../../services/toolSetup";

/** 卡片页脚：状态文案（role=status）+ 启动按钮。 */
export default function ToolCardFooter({
  meta,
  accountWorking,
  message,
  configured,
  scanned,
  pass,
  entry,
  canLaunch,
  busy,
  onLaunch,
}: {
  meta: AiToolMeta;
  accountWorking: boolean;
  message?: string | null;
  configured: boolean;
  scanned: boolean;
  pass: boolean;
  entry?: ToolSetup;
  canLaunch: boolean;
  busy: boolean;
  onLaunch: () => void;
}) {
  return (
    <div className="tool-card-footer">
      <div
        className="tool-card-message"
        role="status"
        aria-live="polite"
        data-tooltip={message ?? undefined}
      >
        {accountWorking ? (
          <span className="text-zinc-500">正在处理账号资料…</span>
        ) : message ? (
          <span
            className={`whitespace-pre-wrap ${
              message.includes("已通过") || message.includes("启动方式已更新")
                ? "text-emerald-300"
                : "text-red-300"
            }`}
          >
            {message}
          </span>
        ) : !configured ? (
          <span className="text-zinc-600">先选择快捷方式</span>
        ) : !scanned ? (
          <Link to="/detect" className="underline underline-offset-2">
            先做一次环境检测
          </Link>
        ) : pass ? (
          <span className="flex items-center gap-1 text-emerald-400">
            <CheckCircle2 className="h-3 w-3" />{" "}
            {entry?.shortcutValid === false
              ? "请更换失效入口"
              : "启动时检查服务"}
          </span>
        ) : (
          <span className="text-amber-400">环境未全绿，可确认后启动</span>
        )}
      </div>
      <button
        onClick={onLaunch}
        disabled={!canLaunch || busy}
        className="tool-card-launch"
        data-tooltip={
          accountWorking
            ? "账号资料处理完成后可启动"
            : !configured
              ? "先配置快捷方式"
              : !scanned
                ? "先检测环境"
                : !pass
                  ? "环境未全绿，点击后会先确认"
                  : `启动 ${meta.label}`
        }
      >
        {busy ? (
          <Loader2 className="h-4 w-4 animate-spin" />
        ) : (
          <Play className="h-4 w-4" />
        )}
        启动
      </button>
    </div>
  );
}
