import { TriangleAlert } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { listen } from "@tauri-apps/api/event";
import { api } from "../../services/api";

interface ExitBlocked {
  reason?: "proxy";
  detail?: string | null;
}

/**
 * 退出确认。
 *
 * 主窗口关闭时后端可能拦下退出：退出前的代理恢复失败。原来只写一条提示、
 * 窗口不动，看起来像「点多少次关闭都没反应」；
 * 现在弹出确认框，点「仍要退出」就强制退出。
 */
export default function ExitConfirm() {
  const [blocked, setBlocked] = useState<ExitBlocked | null>(null);
  const [busy, setBusy] = useState(false);
  const dialog = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<ExitBlocked>("exit-blocked", (event) => {
      if (disposed) return;
      setBlocked(event.payload ?? { reason: "proxy" });
    })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);
  useLayoutEffect(() => {
    if (blocked)
      dialog.current?.querySelector("button")?.focus({ preventScroll: true });
  }, [blocked]);
  useEffect(() => {
    if (!blocked) return;
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      setBlocked(null);
    };
    document.addEventListener("keydown", escape);
    return () => document.removeEventListener("keydown", escape);
  }, [blocked]);

  if (!blocked) return null;
  return createPortal(
    <div
      className="app-dialog-backdrop"
      onPointerDown={(event) => {
        if (event.target === event.currentTarget) setBlocked(null);
      }}
    >
      <div
        ref={dialog}
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="exit-confirm-title"
        aria-describedby="exit-confirm-body"
        className="app-dialog"
      >
        <h2 id="exit-confirm-title">
          <TriangleAlert size={14} />
          代理回收未完成
        </h2>
        <p id="exit-confirm-body">
          退出前回收系统代理没有成功。继续退出会停在当前状态，可能需要手动关闭系统代理；不退出则可以先去设置里处理。
        </p>
        <p className="app-dialog-quote">
          {blocked.detail || "代理恢复失败，原因未返回。"}
        </p>
        <div className="app-dialog-actions">
          <button type="button" onClick={() => setBlocked(null)}>
            取消
          </button>
          <button
            type="button"
            className="is-danger"
            disabled={busy}
            onClick={() => {
              setBusy(true);
              void api.forceExit().catch((error) => {
                console.error(error);
                setBusy(false);
              });
            }}
          >
            {busy ? "正在退出…" : "仍要退出"}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
