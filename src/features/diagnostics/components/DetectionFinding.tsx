import type { Verdict } from "../signals/exit";

const colors: Record<Verdict, string> = {
  g: "text-emerald-400 bg-emerald-500/10",
  y: "text-amber-400 bg-amber-500/10",
  r: "text-red-400 bg-red-500/10",
  n: "text-zinc-500 bg-zinc-800/60",
};
const labels: Record<Verdict, string> = {
  g: "良好",
  y: "注意",
  r: "风险",
  n: "未知",
};

export default function DetectionFinding({
  label,
  verdict,
  msg,
  pending = false,
}: {
  label?: string;
  verdict: Verdict;
  msg: string;
  pending?: boolean;
}) {
  return (
    <div className="check-row flex items-start gap-2.5 rounded-lg bg-zinc-900/60 px-3 py-2 text-xs">
      <span
        className={`shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium ${colors[pending ? "n" : verdict]}`}
      >
        {pending ? "待检测" : labels[verdict]}
      </span>
      <div className="min-w-0">
        {label && <div className="text-zinc-300">{label}</div>}
        <div className="break-words text-[11px] leading-5 text-zinc-500">
          {msg}
        </div>
      </div>
    </div>
  );
}
