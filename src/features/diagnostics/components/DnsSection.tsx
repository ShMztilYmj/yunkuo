import { ShieldCheck } from "lucide-react";
import DnsDeepPanel from "./DnsDeepPanel";
import DnsVerdictChip from "./DnsVerdictChip";
import { dnsDeepVerdict } from "../signals/dnsDeep";
import type { DnsDeepController } from "../hooks/useDnsDeep";

/**
 * DNS 泄露检测：只保留深度测试，右上角徽章与结论横幅共用同一套判定。
 * 内核 DNS 判定不再参与这里的结论——同一张卡片出现两种相反结论会互相打脸。
 */
export default function DnsSection({
  done,
  deep,
  exitIp,
  exitCountryName,
}: {
  done: boolean;
  deep: DnsDeepController;
  exitIp: string | null;
  exitCountryName: string | null;
}) {
  const verdict = !done
    ? ({ tone: "idle", label: "待检测" } as const)
    : deep.busy
      ? ({ tone: "idle", label: "深度测试中…" } as const)
      : dnsDeepVerdict(deep.report);
  return (
    <section
      aria-labelledby="dns-results-title"
      className="ui-panel rounded-2xl border border-zinc-800 bg-zinc-900/40 p-6"
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h2
            id="dns-results-title"
            className="flex items-center gap-2 text-sm font-medium"
          >
            <ShieldCheck className="h-4 w-4 text-yellow-400" />
            DNS 泄露检测
          </h2>
          <p className="mt-1 text-[11px] leading-relaxed text-zinc-500">
            知道吗？DNS
            可能在悄悄暴露你的真实上网位置。跑一次深度测试，看解析器有没有绕过代理。
          </p>
        </div>
        <DnsVerdictChip tone={verdict.tone} label={verdict.label} />
      </div>

      <DnsDeepPanel
        report={deep.report}
        busy={deep.busy}
        progress={deep.progress}
        message={deep.message}
        at={deep.at}
        exitIp={exitIp}
        exitCountryName={exitCountryName}
        onRun={() => void deep.run()}
        onStop={deep.stop}
      />
    </section>
  );
}
