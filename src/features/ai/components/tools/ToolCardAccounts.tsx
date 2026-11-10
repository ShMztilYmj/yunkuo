import type { AiToolMeta } from "../../services/aiToolCards";
import ClaudeIdentityPanel from "../ClaudeIdentityPanel";
import CodexAccountsPanel from "../CodexAccountsPanel";

/** 账号管理抽屉：Codex 走多账号面板，Claude 走身份面板。 */
export default function ToolCardAccounts({
  meta,
  showAccounts,
  onAccountBusyChange,
  onCodexAccountsChanged,
  onClaudeIdentityChanged,
}: {
  meta: AiToolMeta;
  showAccounts: boolean;
  onAccountBusyChange: (busy: boolean) => void;
  onCodexAccountsChanged: () => void;
  onClaudeIdentityChanged: () => void;
}) {
  return (
    <div
      className="tool-card-details tool-card-accounts"
      id={`tool-card-accounts-${meta.key}`}
      hidden={!showAccounts}
      role="region"
      aria-label={`${meta.label}账号管理`}
      tabIndex={showAccounts ? 0 : -1}
    >
      {meta.key === "chatgpt" ? (
        <CodexAccountsPanel
          onChanged={onCodexAccountsChanged}
          onBusyChange={onAccountBusyChange}
        />
      ) : (
        <ClaudeIdentityPanel
          onChanged={onClaudeIdentityChanged}
          onBusyChange={onAccountBusyChange}
          compact
        />
      )}
    </div>
  );
}
