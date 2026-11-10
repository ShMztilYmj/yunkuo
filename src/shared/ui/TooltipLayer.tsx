import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { createPortal } from "react-dom";

interface Hint {
  target: HTMLElement;
  text: string;
  detail: string;
}

// data-tooltip keeps hints independent of button layout, including disabled controls.
// Icon-only buttons can use their existing accessible label as the short hint.
function hintFor(node: EventTarget | null): Hint | null {
  if (!(node instanceof Element)) return null;
  const target = node.closest<HTMLElement>(
    "[data-tooltip], button[aria-label], a[aria-label], [role='button'][aria-label]",
  );
  if (!target || target.closest("[inert]")) return null;
  // 有些控件悬停后自己就会展开详情（如悬浮条头像），重复的提示会盖住下方内容。
  if (target.closest("[data-tooltip-off]")) return null;
  const text = target.hasAttribute("data-tooltip")
    ? target.getAttribute("data-tooltip")
    : target.textContent?.trim()
      ? null
      : target.getAttribute("aria-label");
  return text?.trim()
    ? {
        target,
        text: text.trim(),
        detail: target.dataset.tooltipDetail?.trim() ?? "",
      }
    : null;
}

export default function TooltipLayer({
  onHoverChange,
}: { onHoverChange?: (hovered: boolean) => void } = {}) {
  const id = useId();
  const [hint, setHint] = useState<Hint | null>(null);
  const popup = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({
    left: 0,
    top: 0,
    arrow: 16,
    arrowY: 16,
    side: "bottom",
    ready: false,
  });

  useEffect(() => {
    let current: Hint | null = null;
    let pending: Hint | null = null;
    let opening: ReturnType<typeof setTimeout> | undefined;
    let closing: ReturnType<typeof setTimeout> | undefined;
    const cancelTimers = () => {
      clearTimeout(opening);
      clearTimeout(closing);
    };
    const hide = () => {
      cancelTimers();
      current = pending = null;
      setHint(null);
    };
    const show = (next: Hint, delay: number) => {
      clearTimeout(closing);
      if (
        current?.target === next.target &&
        current.text === next.text &&
        current.detail === next.detail
      )
        return;
      if (
        pending?.target === next.target &&
        pending.text === next.text &&
        pending.detail === next.detail
      )
        return;
      clearTimeout(opening);
      pending = next;
      current = null;
      setHint(null);
      opening = setTimeout(() => {
        if (!next.target.isConnected) return;
        current = next;
        pending = null;
        setHint(hintFor(next.target));
      }, delay);
    };
    const leave = () => {
      clearTimeout(opening);
      pending = null;
      clearTimeout(closing);
      closing = setTimeout(hide, 140);
    };
    const insidePopup = (node: EventTarget | null) =>
      node instanceof Node && popup.current?.contains(node);
    const over = (event: PointerEvent) => {
      if (event.pointerType === "touch") return;
      if (insidePopup(event.target)) {
        clearTimeout(closing);
        return;
      }
      const next = hintFor(event.target);
      if (next) show(next, current ? 100 : 360);
      else leave();
    };
    const out = (event: PointerEvent) => {
      if (insidePopup(event.relatedTarget)) return;
      const next = hintFor(event.relatedTarget);
      if (next?.target === (current ?? pending)?.target) return;
      if (
        current?.target === document.activeElement &&
        current.target.matches(":focus-visible")
      )
        return;
      leave();
    };
    const focus = (event: FocusEvent) => {
      const next = hintFor(event.target);
      if (next && next.target.matches(":focus-visible")) show(next, 0);
    };
    const blur = (event: FocusEvent) => {
      if (
        !event.relatedTarget ||
        hintFor(event.relatedTarget)?.target !== (current ?? pending)?.target
      )
        hide();
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") hide();
    };
    const scroll = (event: Event) => {
      if (!insidePopup(event.target)) hide();
    };
    const resize = () => {
      // HUD 原生窗口收起后不在狭窄的透明窗口里残留提示。
      if (
        document.documentElement.dataset.surface === "hud" &&
        (window.innerWidth <= 72 || window.innerHeight <= 72)
      )
        hide();
    };
    document.addEventListener("pointerover", over, true);
    document.addEventListener("pointerout", out, true);
    document.addEventListener("pointerdown", hide, true);
    document.addEventListener("focusin", focus, true);
    document.addEventListener("focusout", blur, true);
    document.addEventListener("keydown", key, true);
    document.addEventListener("scroll", scroll, true);
    document.addEventListener("visibilitychange", hide);
    window.addEventListener("blur", hide);
    window.addEventListener("hashchange", hide);
    window.addEventListener("resize", resize);
    return () => {
      cancelTimers();
      document.removeEventListener("pointerover", over, true);
      document.removeEventListener("pointerout", out, true);
      document.removeEventListener("pointerdown", hide, true);
      document.removeEventListener("focusin", focus, true);
      document.removeEventListener("focusout", blur, true);
      document.removeEventListener("keydown", key, true);
      document.removeEventListener("scroll", scroll, true);
      document.removeEventListener("visibilitychange", hide);
      window.removeEventListener("blur", hide);
      window.removeEventListener("hashchange", hide);
      window.removeEventListener("resize", resize);
    };
  }, []);

  useLayoutEffect(() => {
    if (!hint || !popup.current) return;
    const update = () => {
      const anchor = hint.target.getBoundingClientRect();
      const box = popup.current?.getBoundingClientRect();
      if (!box) return;
      const margin = 10,
        gap = 9;
      const below = window.innerHeight - anchor.bottom - margin - gap;
      const above = anchor.top - margin - gap;
      const preferred = hint.target.closest<HTMLElement>("[data-tooltip-side]")
        ?.dataset.tooltipSide;
      let side = below >= box.height || below >= above ? "bottom" : "top";
      if (preferred === "top" && above >= box.height) side = "top";
      else if (preferred === "bottom" && below >= box.height) side = "bottom";
      else if (
        preferred === "right" &&
        window.innerWidth - anchor.right - margin - gap >= box.width
      )
        side = "right";
      else if (preferred === "left" && anchor.left - margin - gap >= box.width)
        side = "left";
      const horizontal = side === "left" || side === "right";
      const desiredLeft = horizontal
        ? side === "left"
          ? anchor.left - box.width - gap
          : anchor.right + gap
        : anchor.left + anchor.width / 2 - box.width / 2;
      const left = Math.max(
        margin,
        Math.min(desiredLeft, window.innerWidth - box.width - margin),
      );
      const desiredTop = horizontal
        ? anchor.top + anchor.height / 2 - box.height / 2
        : side === "bottom"
          ? anchor.bottom + gap
          : anchor.top - box.height - gap;
      const top = Math.max(
        margin,
        Math.min(desiredTop, window.innerHeight - box.height - margin),
      );
      const arrow = Math.max(
        14,
        Math.min(anchor.left + anchor.width / 2 - left, box.width - 14),
      );
      const arrowY = Math.max(
        14,
        Math.min(anchor.top + anchor.height / 2 - top, box.height - 14),
      );
      setPosition({ left, top, arrow, arrowY, side, ready: true });
    };
    update();
    const resize = new ResizeObserver(update);
    resize.observe(popup.current);
    resize.observe(hint.target);
    const scope = hint.target.closest("[data-tooltip-side]");
    const placement = new MutationObserver(update);
    if (scope)
      placement.observe(scope, {
        attributes: true,
        attributeFilter: ["data-tooltip-side", "style"],
      });
    window.addEventListener("resize", update);
    return () => {
      resize.disconnect();
      placement.disconnect();
      window.removeEventListener("resize", update);
    };
  }, [hint]);

  useEffect(() => {
    if (!hint) return;
    const target = hint.target;
    const description = (target.getAttribute("aria-describedby") ?? "")
      .split(/\s+/)
      .filter(Boolean);
    target.setAttribute(
      "aria-describedby",
      [...new Set([...description, id])].join(" "),
    );
    const observer = new MutationObserver(() => {
      const next = hintFor(target);
      if (!target.isConnected || !next) setHint(null);
      else if (next.text !== hint.text || next.detail !== hint.detail)
        setHint(next);
      else if (
        document.documentElement.dataset.surface === "hud" &&
        (!target.closest(".agent-hud-viewport.is-expanded") ||
          target.closest('[data-paused="true"]'))
      )
        setHint(null);
    });
    observer.observe(document.body, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: [
        "data-tooltip",
        "data-tooltip-detail",
        "data-tooltip-side",
        "aria-label",
        "data-paused",
      ],
    });
    return () => {
      observer.disconnect();
      const remaining = (target.getAttribute("aria-describedby") ?? "")
        .split(/\s+/)
        .filter((value) => value && value !== id);
      if (remaining.length)
        target.setAttribute("aria-describedby", remaining.join(" "));
      else target.removeAttribute("aria-describedby");
    };
  }, [hint, id]);

  if (!hint) return null;
  return createPortal(
    <div
      ref={popup}
      id={id}
      role="tooltip"
      className={`app-tooltip${hint.detail ? " has-detail" : ""}`}
      data-side={position.side}
      onPointerEnter={() => onHoverChange?.(true)}
      onPointerLeave={() => onHoverChange?.(false)}
      style={
        {
          left: position.left,
          top: position.top,
          visibility: position.ready ? "visible" : "hidden",
          "--tooltip-arrow-x": `${position.arrow}px`,
          "--tooltip-arrow-y": `${position.arrowY}px`,
        } as CSSProperties
      }
    >
      <span className="app-tooltip-arrow" aria-hidden="true" />
      <div className="app-tooltip-content">
        {hint.detail ? (
          <>
            <div className="app-tooltip-title">{hint.text}</div>
            <div className="app-tooltip-detail">{hint.detail}</div>
          </>
        ) : (
          hint.text
        )}
      </div>
    </div>,
    document.body,
  );
}
