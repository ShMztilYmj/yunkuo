import { AlertTriangle, XCircle } from "lucide-react";

/** 环境未达标提示：逐条列出未通过的检查项。 */
export default function GateWarning({ failures }: { failures: string[] }) {
  return (
    <div className="ai-tools-warning rounded-xl border border-amber-700/40 bg-amber-500/10 p-4">
      <div className="mb-2 flex items-center gap-2 text-sm font-medium text-amber-300">
        <AlertTriangle className="h-4 w-4" />
        启动前需要完成：
      </div>
      <ul className="space-y-1 text-xs text-amber-200/90">
        {failures.map((f, i) => (
          <li key={i} className="flex items-start gap-1.5">
            <XCircle className="mt-0.5 h-3 w-3 shrink-0" />
            {f}
          </li>
        ))}
      </ul>
      <div className="mt-2 text-[11px] text-amber-200/70">
        修复请到「环境检测并修复」页逐项处理。
      </div>
    </div>
  );
}
