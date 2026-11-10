import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { usePageActive } from "../hooks/usePageActive";

/** 装饰性品牌图标可用的头像标识；工具名到标识的映射由各业务模块负责。 */
export type AgentBotId =
  | "claude-code"
  | "codex"
  | "grok"
  | "glm"
  | "deepseek"
  | "kimi"
  | "antigravity"
  | "cursor"
  | "opencode";
export type AgentBotState = "idle" | "working" | "asking" | "done";

const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
const subscribeMotion = (listener: () => void) => {
  motion.addEventListener("change", listener);
  return () => motion.removeEventListener("change", listener);
};
const reducedMotion = () => motion.matches;

/** 默认待机与完成使用静态图，只有工作动效创建视频；HUD 悬浮条可用 loopIdle 让四个状态都循环。 */
export default function AgentBot({
  bot,
  state = "idle",
  size = 40,
  className = "",
  animate = true,
  loopIdle = false,
}: {
  bot: AgentBotId | undefined;
  state?: AgentBotState;
  size?: number;
  className?: string;
  /** 父级判定当前不值得解码时传 false：保留静帧，释放视频。 */
  animate?: boolean;
  /** 待机/完成也播放循环视频；只给常驻悬浮条用，列表页仍走静帧省解码。 */
  loopIdle?: boolean;
}) {
  return bot ? (
    <BotMedia
      key={`${bot}:${state}`}
      id={bot}
      state={state}
      size={size}
      className={className}
      animate={animate}
      loopIdle={loopIdle}
    />
  ) : null;
}

function BotMedia({
  id,
  state,
  size,
  className,
  animate,
  loopIdle,
}: {
  id: AgentBotId;
  state: AgentBotState;
  size: number;
  className: string;
  animate: boolean;
  loopIdle: boolean;
}) {
  const container = useRef<HTMLSpanElement>(null);
  const video = useRef<HTMLVideoElement>(null);
  const active = usePageActive();
  const reduced = useSyncExternalStore(subscribeMotion, reducedMotion);
  const [visible, setVisible] = useState(false);
  const [failed, setFailed] = useState(false);
  const [ready, setReady] = useState(false);
  const src = `/agent-bots/${id}/${state}`;
  const animated = loopIdle || state === "working" || state === "asking";
  const playing =
    animated && animate && active && visible && !reduced && !failed;
  useEffect(() => {
    if (!animated) return;
    const observer = new IntersectionObserver(([entry]) => {
      setVisible(entry.isIntersecting);
    });
    if (container.current) observer.observe(container.current);
    return () => observer.disconnect();
  }, [animated]);
  useEffect(() => {
    setReady(false);
    const media = video.current;
    if (!playing || !media) return;
    void media.play().catch(() => {});
    return () => {
      media.pause();
      media.removeAttribute("src");
      media.load();
    };
  }, [playing]);
  return (
    <span
      ref={container}
      className={`agent-bot ${className}`}
      style={{ width: size, height: size }}
      data-agent-bot={id}
      data-bot-state={state}
      data-ready={playing && ready}
      aria-hidden="true"
    >
      <img
        src={`${src}.svg`}
        alt=""
        width={size}
        height={size}
        decoding="async"
        draggable={false}
      />
      {playing && (
        <video
          ref={video}
          src={`${src}.webm`}
          loop
          muted
          playsInline
          preload="metadata"
          tabIndex={-1}
          disablePictureInPicture
          data-ready={ready}
          onLoadedData={() => setReady(true)}
          onError={() => setFailed(true)}
        />
      )}
    </span>
  );
}
