import { useEffect, useState } from "react";
import { Bell, X } from "lucide-react";
import { Link } from "react-router-dom";
import { useTauriEvent } from "../../../shared/hooks/useTauriEvent";
import { api } from "../../../services/api";
import { refreshAiMonitor, useAiMonitor } from "../services/aiMonitor";

export function AiCompletionNotice() {
  const notice = useAiMonitor((state) => state.status.notices[0]);
  useTauriEvent("ai-monitor-changed", () => void refreshAiMonitor(true));
  useEffect(() => {
    void refreshAiMonitor();
  }, []);
  if (!notice) return null;
  return (
    <aside
      aria-label="AI 会话完成提醒"
      role="status"
      className="ai-completion-notice ui-panel flex items-start gap-3 border p-4 text-xs shadow-lg"
    >
      <Bell size={17} className="shrink-0 text-emerald-400" />
      <div className="min-w-0 flex-1">
        <p className="font-medium">{notice.message}</p>
        <Link
          to="/ai-stats"
          className="mt-2 inline-block text-zinc-500 hover:text-zinc-200"
        >
          查看会话统计 →
        </Link>
      </div>
      <button
        aria-label="关闭会话提醒"
        className="shrink-0 text-zinc-500"
        onClick={() => {
          void api
            .aiMonitorDismiss(notice.id)
            .then(() =>
              useAiMonitor.setState((state) => ({
                status: {
                  ...state.status,
                  notices: state.status.notices.filter(
                    (item) => item.id !== notice.id,
                  ),
                },
              })),
            )
            .catch((error) => useAiMonitor.setState({ error: String(error) }));
        }}
      >
        <X size={14} />
      </button>
    </aside>
  );
}

export default function AiMonitorControls() {
  const { status, error, loaded } = useAiMonitor();
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const toggle = async () => {
    setBusy(true);
    setFailure(null);
    try {
      await api.aiMonitorSetEnabled(!status.enabled);
      await refreshAiMonitor(true);
    } catch (error) {
      setFailure(`保存提醒设置失败：${String(error)}`);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="mt-4 border-t border-zinc-800 pt-3">
      <div className="flex items-center justify-between gap-3">
        <div>
          <p className="flex items-center gap-1.5 text-xs font-medium">
            <Bell size={13} />
            后台完成提醒
          </p>
          <p className="mt-1 text-[10px] leading-relaxed text-zinc-500">
            开启后每 30
            秒检查本地日志，收起窗口后继续提醒；首次读取不补发历史消息。
          </p>
        </div>
        <button
          role="switch"
          aria-label="后台完成提醒"
          aria-checked={status.enabled}
          disabled={busy || !loaded || !!error}
          className={`relative h-5 w-9 shrink-0 rounded-full disabled:opacity-40 ${status.enabled ? "bg-emerald-500" : "bg-zinc-700"}`}
          onClick={() => void toggle()}
        >
          <span
            className={`absolute top-0.5 h-4 w-4 rounded-full bg-white shadow-sm ${status.enabled ? "right-0.5" : "left-0.5"}`}
          />
        </button>
      </div>
      <p className="mt-2 text-[10px] text-zinc-500">
        系统通知需在安装版中使用；关闭提醒后停止后台扫描。
      </p>
      {(failure || error || status.deliveryError) && (
        <p role="alert" className="mt-2 text-[11px] text-amber-300">
          {failure ?? error ?? status.deliveryError}
        </p>
      )}
      {error && (
        <button
          className="mt-1 text-[11px] text-zinc-400 underline"
          onClick={() => void refreshAiMonitor(true)}
        >
          重新读取提醒设置
        </button>
      )}
    </div>
  );
}
