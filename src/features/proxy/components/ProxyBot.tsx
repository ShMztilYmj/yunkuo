import {
  useEffect,
  useId,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";

type ProxyState = "on" | "off" | "unknown";
type Pose = "sleeping" | "waking" | "awake" | "dozing" | "uncertain";
const steadyPose = (state: ProxyState): Pose =>
  ({ on: "awake", off: "sleeping", unknown: "uncertain" })[state] as Pose;
const subscribeMotion = (changed: () => void) => {
  const media = window.matchMedia("(prefers-reduced-motion: reduce)");
  media.addEventListener("change", changed);
  return () => media.removeEventListener("change", changed);
};
const reducedMotion = () =>
  window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/** 真实代理状态驱动表情；一次性启停动作结束后回到轻量循环，刷新数据不会重播。 */
export default function ProxyBot({
  state,
  size = 42,
  active = true,
  wakeOnMount = true,
}: {
  state: ProxyState;
  size?: number;
  active?: boolean;
  wakeOnMount?: boolean;
}) {
  const id = useId().replace(/:/g, "");
  const reduced = useSyncExternalStore(
    subscribeMotion,
    reducedMotion,
    () => true,
  );
  const initialPose =
    active && !reduced && wakeOnMount && state === "on"
      ? "waking"
      : steadyPose(state);
  const [pose, setPose] = useState<Pose>(initialPose);
  const previous = useRef(state),
    currentPose = useRef<Pose>(initialPose);
  useEffect(() => {
    const before = previous.current;
    let next = currentPose.current;
    if (!active || reduced) next = steadyPose(state);
    else if (before !== state)
      next =
        state === "on"
          ? "waking"
          : state === "off" && before !== "unknown"
            ? "dozing"
            : steadyPose(state);
    previous.current = state;
    currentPose.current = next;
    setPose(next);
    const duration = next === "waking" ? 1600 : next === "dozing" ? 1100 : 0;
    if (!duration) return;
    const timer = setTimeout(() => {
      currentPose.current = steadyPose(state);
      setPose(currentPose.current);
    }, duration);
    return () => clearTimeout(timer);
  }, [state, active, reduced]);

  return (
    <svg
      className="hud-proxy-bot"
      data-proxy-bot={state}
      data-pose={pose}
      data-paused={!active}
      width={size}
      height={size}
      viewBox="0 0 100 100"
      aria-hidden="true"
    >
      <defs>
        <radialGradient id={`${id}-body`} cx="28%" cy="16%" r="95%">
          <stop stopColor="var(--proxy-body-light, #4a514b)" />
          <stop offset=".55" stopColor="var(--proxy-body-mid, #252b26)" />
          <stop offset="1" stopColor="var(--proxy-body-dark, #141916)" />
        </radialGradient>
      </defs>
      <g className="proxy-bot-art" transform="translate(0 -2.5)">
        <ellipse
          className="proxy-bot-shadow"
          cx="50"
          cy="83"
          rx="19"
          ry="2.4"
          fill="#183122"
          opacity=".1"
        />
        <g className="proxy-bot-float">
          <g className="proxy-bot-body">
            <rect x="25" y="29" width="50" height="50" rx="17" fill="#111613" />
            <rect
              x="25"
              y="26"
              width="50"
              height="50"
              rx="17"
              fill={`url(#${id}-body)`}
            />
            <path
              d="M30 43c0-7 6-12 13-12h13"
              fill="none"
              stroke="#fff"
              strokeOpacity="var(--proxy-highlight, .12)"
              strokeWidth="2"
              strokeLinecap="round"
            />
            <g className="proxy-bot-eyes" fill="#f5fff7">
              <rect x="36" y="43" width="8" height="14" rx="4" />
              <rect x="56" y="43" width="8" height="14" rx="4" />
            </g>
          </g>
        </g>
        <g
          className="proxy-bot-sleep-marks"
          fill="none"
          stroke="currentColor"
          strokeWidth="2.6"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path className="proxy-bot-sleep-mark" d="M77 36h7l-7 6h7" />
          <path className="proxy-bot-sleep-mark" d="M74 25h9l-9 8h9" />
          <path className="proxy-bot-sleep-mark" d="M64 14h10l-10 9h10" />
        </g>
      </g>
    </svg>
  );
}
