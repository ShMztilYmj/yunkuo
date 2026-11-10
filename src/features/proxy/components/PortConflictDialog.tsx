import { useEffect, useId, useRef } from "react";
import { Info, X } from "lucide-react";
import type { ProxyPortUsage } from "../../../services/api";

/** 进入代理页时的端口占用提示；使用原生 dialog，Esc / 点击遮罩即可关闭。 */
export default function PortConflictDialog({
  usage,
  onClose,
  onOpenSettings,
}: {
  usage: ProxyPortUsage;
  onClose: () => void;
  onOpenSettings: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => {
    const dialog = ref.current;
    dialog?.showModal();
    return () => dialog?.close();
  }, []);
  const occupant = usage.process
    ? `${usage.process}${usage.pid ? `（PID ${usage.pid}）` : ""}`
    : usage.pid
      ? `PID ${usage.pid}`
      : "未知程序";
  return (
    <dialog
      ref={ref}
      aria-labelledby={titleId}
      onCancel={onClose}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
      className="m-auto w-[min(430px,calc(100vw-32px))] rounded-2xl border border-zinc-700/60 bg-zinc-900 p-0 text-zinc-100 shadow-2xl backdrop:bg-black/50"
    >
      <div className="flex items-start gap-3 p-5">
        <span className="mt-0.5 rounded-full bg-amber-500/15 p-2 text-amber-400">
          <Info size={18} />
        </span>
        <div className="min-w-0 flex-1">
          <h2 id={titleId} className="text-sm font-semibold">
            端口 {usage.port} 已被占用
          </h2>
          <p className="mt-1 text-xs leading-relaxed text-zinc-400">
            占用进程：<span className="text-zinc-200">{occupant}</span>
            <br />
            云阔无法在 {usage.port}{" "}
            端口启动代理。请关闭占用该端口的程序，或在设置里改用其他端口。
          </p>
        </div>
        <button
          type="button"
          aria-label="关闭端口占用提示"
          className="rounded-md p-1 text-zinc-500 hover:bg-zinc-800 hover:text-zinc-200"
          onClick={onClose}
        >
          <X size={16} />
        </button>
      </div>
      <div className="flex justify-end gap-2 border-t border-zinc-800 px-5 py-3">
        <button
          type="button"
          className="rounded-md px-3 py-2 text-xs text-zinc-300 hover:bg-zinc-800"
          onClick={onClose}
        >
          知道了
        </button>
        <button
          type="button"
          className="rounded-md bg-sky-600 px-3 py-2 text-xs text-white hover:bg-sky-500"
          onClick={onOpenSettings}
        >
          打开设置
        </button>
      </div>
    </dialog>
  );
}
