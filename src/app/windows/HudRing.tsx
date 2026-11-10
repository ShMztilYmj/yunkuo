import type { HTMLAttributes } from "react";

type Props = HTMLAttributes<HTMLSpanElement> & {
  value: number | null;
  showTrack?: boolean;
};

/** 圆弧和内容使用相同的中心，内容大小不参与圆环布局。 */
export default function HudRing({
  value,
  showTrack = true,
  className = "",
  children,
  ...props
}: Props) {
  const known = value != null && Number.isFinite(value);
  const percent = known ? Math.max(0, Math.min(100, value)) : 0;
  return (
    <span {...props} className={`hud-ring ${className}`} data-known={known}>
      {showTrack && (
        <svg className="hud-ring-track" viewBox="0 0 44 44" aria-hidden="true">
          <circle className="hud-ring-base" cx="22" cy="22" r="20.5" />
          {known && percent > 0 && (
            <circle
              className="hud-ring-progress"
              cx="22"
              cy="22"
              r="20.5"
              pathLength="100"
              strokeDasharray={`${percent} 100`}
              strokeLinecap="round"
              transform="rotate(-90 22 22)"
            />
          )}
        </svg>
      )}
      <span className="hud-ring-content">{children}</span>
    </span>
  );
}
