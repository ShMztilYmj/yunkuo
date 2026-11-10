import { Zap } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useLocation } from "react-router-dom";
import ProxyStatusPanel from "./ProxyStatusPanel";

/** 侧栏底部的代理快捷控制：悬浮按钮 + 传送门弹窗 */
export default function ProxyQuickControl() {
  const loc = useLocation();
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState({ left: 12, top: 12 });
  const trigger = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setOpen(false);
  }, [loc.pathname, loc.search]);
  useLayoutEffect(() => {
    if (!open) return;
    const place = () => {
      if (!trigger.current || !panel.current) return;
      const anchor = trigger.current.getBoundingClientRect();
      // Entrance motion scales the panel; anchor it using its final layout size.
      const box = {
        width: panel.current.offsetWidth,
        height: panel.current.offsetHeight,
      };
      setPosition({
        left: Math.max(
          12,
          Math.min(anchor.right + 12, window.innerWidth - box.width - 12),
        ),
        top: Math.max(
          12,
          Math.min(
            anchor.bottom - box.height,
            window.innerHeight - box.height - 12,
          ),
        ),
      });
    };
    place();
    const observer = new ResizeObserver(place);
    if (panel.current) observer.observe(panel.current);
    panel.current?.querySelector("button")?.focus({ preventScroll: true });
    window.addEventListener("resize", place);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", place);
    };
  }, [open]);
  useEffect(() => {
    if (!open) return;
    const outside = (event: Event) => {
      if (
        event.target instanceof Node &&
        !panel.current?.contains(event.target) &&
        !trigger.current?.contains(event.target) &&
        // 确认框是独立 portal，点它不该把快捷面板一起关掉。
        !(
          event.target instanceof Element &&
          event.target.closest(".confirm-dialog")
        )
      )
        setOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      // 确认框自己处理 Esc，别顺手把快捷面板也关掉。
      if (document.querySelector(".confirm-dialog")) return;
      event.preventDefault();
      setOpen(false);
      trigger.current?.focus({ preventScroll: true });
    };
    const blur = () => setOpen(false);
    document.addEventListener("pointerdown", outside);
    document.addEventListener("focusin", outside);
    document.addEventListener("keydown", escape);
    window.addEventListener("blur", blur);
    return () => {
      document.removeEventListener("pointerdown", outside);
      document.removeEventListener("focusin", outside);
      document.removeEventListener("keydown", escape);
      window.removeEventListener("blur", blur);
    };
  }, [open]);

  return (
    <>
      <button
        ref={trigger}
        className="sidebar-about sidebar-proxy-trigger"
        aria-label="代理快捷控制"
        data-tooltip="代理快捷控制"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls="proxy-quick-popover"
        onClick={() => setOpen((value) => !value)}
      >
        <Zap size={20} />
      </button>
      {open &&
        createPortal(
          <div
            ref={panel}
            id="proxy-quick-popover"
            role="dialog"
            aria-label="代理快捷控制"
            className="proxy-quick-popover ui-popover"
            style={position}
          >
            <ProxyStatusPanel />
          </div>,
          document.body,
        )}
    </>
  );
}
