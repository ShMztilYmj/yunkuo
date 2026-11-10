import { useEffect, useId, useRef, useState } from "react";
import { usePageActive } from "../../../shared/hooks/usePageActive";

export type BrowserBotState =
  "idle" | "checking" | "passed" | "attention" | "unavailable";

/** 状态由启动检查和浏览器运行状态驱动；离屏或隐藏时暂停 CSS 动效。 */
export default function BrowserBot({
  state = "idle",
  size = 48,
  animated = true,
  className = "",
}: {
  state?: BrowserBotState;
  size?: number;
  animated?: boolean;
  className?: string;
}) {
  const id = useId();
  const container = useRef<HTMLSpanElement>(null);
  const active = usePageActive();
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    if (!animated || !container.current) return;
    const observer = new IntersectionObserver(([entry]) =>
      setVisible(entry.isIntersecting),
    );
    observer.observe(container.current);
    return () => observer.disconnect();
  }, [animated]);

  return (
    <span
      ref={container}
      className={`browser-bot ${className}`}
      style={{ width: size, height: size }}
      data-browser-bot={state}
      data-design="cloud"
      data-state={state}
      data-paused={!animated || !active || !visible || state === "unavailable"}
      aria-hidden="true"
    >
      <svg width={size} height={size} viewBox="0 0 96 96" focusable="false">
        <defs>
          <linearGradient id={`${id}-cloud`} x1="0" y1="0" x2=".8" y2="1">
            <stop stopColor="#ffffff" />
            <stop offset=".65" stopColor="#f4f3ec" />
            <stop offset="1" stopColor="#cbdedb" />
          </linearGradient>
        </defs>
        <ellipse
          className="browser-bot-shadow"
          cx="48"
          cy="85"
          rx="27"
          ry="3"
          fill="#182c32"
          opacity=".13"
        />
        <g className="browser-bot-body">
          <path
            className="browser-bot-breeze"
            d="M7 46h9M4 54h10M9 62h7"
            fill="none"
            stroke="#99c6c9"
            strokeWidth="2"
            strokeLinecap="round"
          />
          <path d="M50 33v-8q0-6 6-6h8q6 0 6 6v12" fill="#a3c7d5" />
          <path
            d="M29 76C7 76 6 47 23 43c0-25 30-28 38-11 21-4 32 14 24 27-1 12-10 17-22 17Z"
            fill="#b4ccca"
          />
          <path
            d="M29 72C7 72 6 43 23 39c0-25 30-28 38-11 21-4 32 14 24 27-1 12-10 17-22 17Z"
            fill={`url(#${id}-cloud)`}
            stroke="#d8e0d9"
            strokeWidth=".7"
          />
          <path
            d="M28 37q1-13 13-14"
            fill="none"
            stroke="#fff"
            strokeWidth="2.5"
            strokeLinecap="round"
          />
          <g className="browser-bot-face">
            <g className="browser-bot-eyes">
              <rect x="33" y="44" width="7" height="13" rx="3.5" />
              <rect x="56" y="44" width="7" height="13" rx="3.5" />
            </g>
            <path
              className="browser-bot-happy-eyes"
              d="M33 51q3.5-7 7 0m16 0q3.5-7 7 0"
            />
            <path className="browser-bot-mouth" d="M45 63q3 1 6 0" />
            <path className="browser-bot-smile" d="M43 61q5 7 10 0" />
            <path className="browser-bot-worry" d="M45 63h6" />
          </g>
        </g>
      </svg>
    </span>
  );
}
