import type { DnsDeepTone } from "../signals/dnsDeep";

const TONE: Record<DnsDeepTone, string> = {
  idle: "border-zinc-700/70 bg-zinc-800/50 text-zinc-400",
  good: "border-emerald-500/25 bg-emerald-500/10 text-emerald-400",
  bad: "border-red-500/25 bg-red-500/10 text-red-400",
};

/** DNS 卡片右上角的状态徽章：只认深度测试结论，与结论横幅同源。 */
export default function DnsVerdictChip({
  tone,
  label,
}: {
  tone: DnsDeepTone;
  label: string;
}) {
  return (
    <span
      className={`shrink-0 rounded-md border px-2 py-[3px] text-[10px] ${TONE[tone]}`}
      title={label}
    >
      {label}
    </span>
  );
}
