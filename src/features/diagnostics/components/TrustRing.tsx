import { BAND_COLORS } from "./constants";

/**
 * 信任评分环形进度；score 为 null 时只显示轨道和占位。
 * 有上次评分时轨道改为虚线并标注「上次 N」——只做参照，不填当前进度。
 */
export default function TrustRing({
  score,
  previous = null,
}: {
  score: number | null;
  previous?: number | null;
}) {
  const R = 52;
  const C = 2 * Math.PI * R;
  const band =
    (score ?? 0) >= 70 ? "high" : (score ?? 0) >= 40 ? "medium" : "low";
  const color = BAND_COLORS[band as keyof typeof BAND_COLORS].ring;
  const showPrevious = score == null && previous != null;
  return (
    <div className="trust-ring relative">
      <svg width="140" height="140" viewBox="0 0 140 140">
        <circle
          cx="70"
          cy="70"
          r={R}
          stroke="var(--chart-track)"
          strokeWidth="8"
          fill="none"
          strokeDasharray={showPrevious ? "3 7" : undefined}
        />
        {score != null && (
          <circle
            cx="70"
            cy="70"
            r={R}
            stroke={color}
            strokeWidth="8"
            fill="none"
            strokeLinecap="round"
            strokeDasharray={`${C}px`}
            strokeDashoffset={`${C * (1 - score / 100)}px`}
            transform="rotate(-90 70 70)"
            style={{ transition: "stroke-dashoffset 0.5s ease, stroke 0.3s" }}
          />
        )}
      </svg>
      <div className="absolute inset-0 flex flex-col items-center justify-center">
        <div
          className={`text-3xl font-bold ${score == null ? "text-zinc-500" : ""}`}
        >
          {score ?? "—"}
        </div>
        <div className="text-[11px] text-zinc-500">信任评分</div>
        {showPrevious && (
          <div className="text-[10px] text-zinc-600">上次 {previous}</div>
        )}
      </div>
    </div>
  );
}
