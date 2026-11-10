import { Clock3, ScanLine, ShieldCheck } from "lucide-react";
import type { BrowserDiagnostics } from "../services/browser";

/** 检测模块最近一次结果的紧凑投影：2×2 数值块，不占页面高度。 */
export default function BrowserDiagnosticsSummary({
  diagnostics,
}: {
  diagnostics?: BrowserDiagnostics;
}) {
  if (!diagnostics?.at)
    return (
      <div className="fp-empty">
        <span className="fp-empty-icon">
          <ScanLine size={16} strokeWidth={1.6} />
        </span>
        <div>
          <strong>
            {diagnostics?.scanning ? "检测模块正在检测" : "暂无检测结果"}
          </strong>
          <p>启动后在 ippure.com 查看实际环境；需要完整报告时前往检测模块。</p>
        </div>
      </div>
    );

  const score = diagnostics.score;
  const scoreTone =
    score === null ? "" : score <= 30 ? "good" : score <= 60 ? "warn" : "bad";
  const items: {
    label: string;
    value: string;
    hint: string;
    tone?: string;
  }[] = [
    {
      label: "出口 IP",
      value: diagnostics.ip ?? "未知",
      hint: "当前代理出口",
    },
    {
      label: "出口地区",
      value: diagnostics.country ?? "未知",
      hint: "与节点出口一致",
    },
    {
      label: "系统时区",
      value: diagnostics.timezone ?? "未知",
      hint: "与出口地区匹配",
    },
    {
      label: "风险评分",
      value: score === null ? "未知" : `${score} / 100`,
      hint:
        score === null
          ? "等待检测结果"
          : scoreTone === "good"
            ? "低风险"
            : scoreTone === "warn"
              ? "中等风险"
              : "高风险",
      tone: scoreTone,
    },
  ];
  return (
    <>
      <div className="fp-stats">
        {items.map(({ label, value, hint, tone }) => (
          <div
            key={label}
            className={`fp-stat${tone ? ` is-${tone}` : ""}`}
            data-tone={tone || undefined}
          >
            <span>{label}</span>
            <strong title={value}>{value}</strong>
            <small>{hint}</small>
            {label === "风险评分" && score !== null && (
              <div className="fp-stat-meter" aria-hidden="true">
                <i
                  style={{
                    width: `${Math.max(0, Math.min(100, score))}%`,
                  }}
                />
              </div>
            )}
          </div>
        ))}
      </div>
      <div className="fp-detect-summary">
        <strong>启动前自动核验</strong>
        <div className="fp-detect-checks">
          <span>
            <ShieldCheck size={10} />
            系统代理
          </span>
          <span>
            <ShieldCheck size={10} />
            虚拟网卡
          </span>
          <span>
            <ShieldCheck size={10} />
            出站模式
          </span>
        </div>
        <p>
          检测结果来自最近一次环境检测；启动浏览器时会重新核验代理与虚拟网卡，避免用错出口。
        </p>
      </div>
      <p className="fp-note fp-detect-time">
        <Clock3 size={11} />
        <span>
          {diagnostics.scanning
            ? "检测模块正在更新结果，以下为上一次记录。"
            : new Date(diagnostics.at).toLocaleString()}
        </span>
      </p>
    </>
  );
}
