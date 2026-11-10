import { ShieldCheck } from "lucide-react";
import SystemDetectionPanel from "./SystemDetectionPanel";
import type { ExitResult } from "../signals/exit";
import type { SignalResult } from "../signals/web";
import type { NativeDetect } from "../../../services/api";
import type { ComputerStatus } from "../services/environment";

/** 系统级检测卡（内核 / 代理 / 出口 IP / Claude 连通性 / 语言与时区） */
export default function SystemDetectionSection({
  native,
  exit,
  signals,
  requireClaudeReachable,
  refreshKey,
  disabled,
  onBusyChange,
  onSystem,
}: {
  native: NativeDetect | null;
  exit: ExitResult | null;
  signals: SignalResult[];
  requireClaudeReachable: boolean;
  refreshKey: number | null;
  disabled: boolean;
  onBusyChange: (busy: boolean) => void;
  onSystem?: (current: ComputerStatus["current"]) => void;
}) {
  // 修改系统设置会撤销检测结果；保持此卡片挂载，以便查看结果和恢复。
  return (
    <section
      id="system-detection"
      aria-labelledby="system-detection-title"
      className="ui-panel rounded-2xl border border-zinc-800 bg-zinc-900/40 p-6"
    >
      <h2
        id="system-detection-title"
        className="mb-4 flex items-center gap-2 text-sm font-medium"
      >
        <ShieldCheck className="h-4 w-4 text-blue-400" />
        系统级检测
      </h2>
      <SystemDetectionPanel
        native={native}
        exit={exit}
        signals={signals}
        requireClaudeReachable={requireClaudeReachable}
        refreshKey={refreshKey}
        disabled={disabled}
        onBusyChange={onBusyChange}
        onSystem={onSystem}
      />
    </section>
  );
}
