import { ArrowLeft, MoreHorizontal, Users, X } from "lucide-react";
import { cliToolName, type AiToolMeta } from "../../services/aiToolCards";
import type { ToolSetup } from "../../services/toolSetup";

/** 卡片顶栏：配置状态徽标 + 续跑 / 账号 / 详情切换 + 移除按钮。 */
export default function ToolCardTopline({
  meta,
  entry,
  configured,
  supportsAccounts,
  showResume,
  showAccounts,
  showDetails,
  cardBusy,
  launching,
  configReady,
  isDefaultCard,
  onToggleResume,
  onOpenAccounts,
  onToggleDetails,
  onRemove,
}: {
  meta: AiToolMeta;
  entry?: ToolSetup;
  configured: boolean;
  supportsAccounts: boolean;
  showResume: boolean;
  showAccounts: boolean;
  showDetails: boolean;
  cardBusy: boolean;
  launching: boolean;
  configReady: boolean;
  isDefaultCard: boolean;
  onToggleResume: () => void;
  onOpenAccounts: () => void;
  onToggleDetails: () => void;
  onRemove: () => void;
}) {
  const toolName = cliToolName(meta.key);
  return (
    <div className="tool-card-topline">
      <span
        className={`tool-card-configured ${configured ? "is-configured" : ""}`}
        data-tooltip="添加卡片不代表已安装；运行中依据本机进程，登录需在工具中确认"
      >
        <i />
        {entry?.running
          ? "运行中"
          : entry?.configured && !entry.shortcutValid
            ? "入口失效"
            : configured
              ? "已配置"
              : entry?.installed
                ? "已安装 · 待配置"
                : "已添加 · 待配置"}
      </span>
      <div className="tool-card-view-actions">
        {supportsAccounts && (
          <button
            type="button"
            className="tool-card-account-toggle"
            aria-label={`${toolName}自动续跑面板`}
            aria-pressed={showResume}
            onClick={onToggleResume}
          >
            续跑
          </button>
        )}
        {supportsAccounts && (
          <button
            type="button"
            className="tool-card-account-toggle"
            aria-label={`${meta.label}账号管理`}
            aria-pressed={showAccounts}
            aria-controls={`tool-card-accounts-${meta.key}`}
            onClick={onOpenAccounts}
          >
            <Users size={13} />
            账号
          </button>
        )}
        <button
          type="button"
          className="tool-card-detail-toggle"
          aria-label={
            showDetails ? `返回 ${meta.label} 卡面` : `${meta.label}详情`
          }
          aria-expanded={showDetails}
          aria-controls={`tool-card-details-${meta.key}`}
          onClick={onToggleDetails}
        >
          {showDetails ? <ArrowLeft size={13} /> : <MoreHorizontal size={15} />}
          {showDetails ? "卡面" : "详情"}
        </button>
      </div>
      {!isDefaultCard && (
        <button
          type="button"
          className="tool-card-remove"
          aria-label={`移除 ${meta.label} 卡片`}
          data-tooltip="移除卡片（保留启动配置）"
          disabled={cardBusy || launching || !configReady}
          onClick={onRemove}
        >
          <X size={15} />
        </button>
      )}
    </div>
  );
}
