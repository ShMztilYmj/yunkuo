import { cliToolName, type AiToolMeta } from "../../services/aiToolCards";
import CodexResumePanel from "../CodexResumePanel";

/** 自动续跑抽屉：Claude 与 Codex 共用 CodexResumePanel。 */
export default function ToolCardResume({
  meta,
  showResume,
  onResumeBusyChange,
}: {
  meta: AiToolMeta;
  showResume: boolean;
  onResumeBusyChange: (busy: boolean) => void;
}) {
  const toolName = cliToolName(meta.key);
  return (
    <div
      className="tool-card-details"
      hidden={!showResume}
      role="region"
      aria-label={`${toolName}自动续跑`}
      tabIndex={showResume ? 0 : -1}
    >
      <CodexResumePanel
        tool={meta.key === "claude" ? "claude" : "codex"}
        active={showResume}
        onBusyChange={onResumeBusyChange}
      />
    </div>
  );
}
