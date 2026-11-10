import { CircleCheck, Info, Lock, Trash2, Undo2, X } from "lucide-react";
import {
  PROVIDER_SOURCE_LABEL,
  PROVIDER_STATE_LABEL,
  type ProviderBinding,
} from "../services/providerAccounts";
import ProviderQuotaPanel from "./ProviderQuotaPanel";

/** 详情卡片显示的是「哪一列的哪一条登记」，与矩阵单元格里的选中项一一对应。 */
export interface ProviderDetailSelection {
  agentKey: string;
  agentLabel: string;
  binding: ProviderBinding;
}

/** 刚被移除的服务商：面板就地给出结果与撤销入口，不留悬空状态。 */
export interface RemovedProviderNotice {
  key: string;
  agentKey: string;
  agentLabel: string;
  provider: string;
  account: string;
}

/** 不可移除时把原因写清楚，而不是只把按钮藏起来。 */
function lockLabel(binding: ProviderBinding): string {
  if (binding.placeholder) return "暂无真实登记";
  // 账号条目不是服务商：换账号/换登录要在对应 Agent 里做。
  if (binding.source === "login" || binding.source === "snapshot")
    return "账号条目 · 不可移除";
  if (binding.source === "tool") return "由工具自管";
  return "官方来源 · 不可移除";
}

/**
 * 对照表下方的独立详情卡片：固定预留高度，和上面的表格卡片分开。
 *
 * 三种状态互斥：刚移除的提示 > 选中项详情 > 空态提示；卡片自带标题行，
 * 无论有没有选中都能看出这一块是「详情」。
 */
export default function ProviderDetailCard({
  selection,
  removed,
  readAt,
  onRemove,
  onUndo,
  onClose,
}: {
  selection: ProviderDetailSelection | null;
  removed: RemovedProviderNotice | null;
  readAt: string | null;
  onRemove: () => void;
  onUndo: () => void;
  onClose: () => void;
}) {
  const mode = removed ? "removed" : selection ? "binding" : "empty";
  const detail = selection?.binding;
  return (
    <section
      className="ai-providers-detail"
      id="provider-detail"
      role="region"
      aria-label="服务商详情"
      aria-live="polite"
      data-mode={mode}
    >
      <header className="ai-providers-detail-head">
        <span className="ai-providers-detail-eyebrow">服务商详情</span>
        {removed ? (
          <div className="ai-providers-detail-actions">
            <button
              type="button"
              className="ai-providers-detail-undo"
              onClick={onUndo}
            >
              <Undo2 size={13} />
              撤销
            </button>
            <button
              type="button"
              className="ai-providers-detail-close"
              aria-label="关闭详情"
              onClick={onClose}
            >
              <X size={14} />
            </button>
          </div>
        ) : detail ? (
          <div className="ai-providers-detail-actions">
            {detail.deletable ? (
              <button
                type="button"
                className="ai-providers-detail-remove"
                aria-label={`移除服务商 ${detail.provider}`}
                onClick={onRemove}
              >
                <Trash2 size={13} />
                移除
              </button>
            ) : (
              <span className="ai-providers-detail-lock">
                <Lock size={11} aria-hidden="true" />
                {lockLabel(detail)}
              </span>
            )}
            <button
              type="button"
              className="ai-providers-detail-close"
              aria-label="关闭详情"
              onClick={onClose}
            >
              <X size={14} />
            </button>
          </div>
        ) : null}
      </header>

      {removed ? (
        <div className="ai-providers-detail-body">
          <div className="ai-providers-detail-identity">
            <span className="ai-providers-detail-mark is-ok" aria-hidden="true">
              <CircleCheck size={13} />
            </span>
            <strong>已移除「{removed.provider}」</strong>
            <span className="ai-providers-detail-account">
              {removed.agentLabel} · {removed.account}
            </span>
          </div>
          <p className="ai-providers-detail-note">
            已从对照表与各 Agent
            详情中隐藏；工具内的配置保持不变，需要彻底删除请在对应 Agent
            内操作。
          </p>
        </div>
      ) : detail && selection ? (
        <div className="ai-providers-detail-body">
          <div className="ai-providers-detail-identity">
            <span
              className="ai-providers-detail-dot"
              data-state={detail.state}
              aria-hidden="true"
            />
            <strong data-tooltip={detail.provider}>{detail.provider}</strong>
            <span
              className="ai-providers-detail-account"
              data-tooltip={detail.account}
            >
              {detail.account}
            </span>
            <em className="ai-providers-detail-chip" data-state={detail.state}>
              {PROVIDER_STATE_LABEL[detail.state]}
            </em>
          </div>
          <dl className="ai-providers-detail-facts">
            <div>
              <dt>Agent</dt>
              <dd>{selection.agentLabel}</dd>
            </div>
            <div>
              <dt>说明</dt>
              <dd data-tooltip={detail.detail ?? undefined}>
                {detail.detail ?? "—"}
              </dd>
            </div>
            <div>
              <dt>来源</dt>
              <dd>{PROVIDER_SOURCE_LABEL[detail.source]}</dd>
            </div>
            <div>
              <dt>读取于</dt>
              <dd>{readAt ?? "尚未读取"}</dd>
            </div>
          </dl>
          {detail.quota && (
            <ProviderQuotaPanel quota={detail.quota} readAt={readAt} />
          )}
        </div>
      ) : (
        <div className="ai-providers-detail-empty">
          <span className="ai-providers-detail-mark" aria-hidden="true">
            <Info size={13} />
          </span>
          <p>
            点击上方任一服务商，在这里查看账号、来源与状态；非官方添加的服务商可以在这里移除。
          </p>
        </div>
      )}
    </section>
  );
}
