import { useId, useLayoutEffect, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Info, Loader2, TriangleAlert, X, type LucideIcon } from "lucide-react";

export type ConfirmTone = "danger" | "warning" | "neutral";

export interface ConfirmSubject {
  /** 被操作对象的唯一标识，例如配置名、服务商名；等宽显示，可选中复制。 */
  label: string;
  /** 名称右侧的类型徽章。 */
  tag?: ReactNode;
  tagTone?: "blue" | "green" | "amber" | "red";
  /** 名称下方的补充信息，例如更新时间、引用数量。 */
  meta?: ReactNode;
  /** 是否用等宽字体；默认按内容判断（纯 ASCII 才用）。 */
  mono?: boolean;
}

export interface ConfirmDetail {
  label: string;
  value: ReactNode;
}

export interface ConfirmDialogProps {
  /** 危险操作 = 红，需谨慎 = 琥珀，普通确认 = 蓝。 */
  tone?: ConfirmTone;
  /** 标题左侧的图形标记，默认按 tone 取图标。 */
  icon?: LucideIcon;
  /** 标题上方的分类标签，例如「删除配置」。 */
  eyebrow?: string;
  title: string;
  description?: ReactNode;
  subject?: ConfirmSubject | null;
  /** 变更预览：一组「项目 / 目标值」，用于确认前核对具体参数。 */
  details?: ConfirmDetail[] | null;
  /** 后果提示条；tone 缺省时跟随对话框 tone。 */
  note?: { tone?: ConfirmTone; text: ReactNode } | null;
  cancelLabel?: string;
  confirmLabel?: string;
  busyLabel?: string;
  /** 执行中：两个按钮都禁用，Esc / 遮罩点击不再关闭。 */
  busy?: boolean;
  /** 失败原因，就地显示在操作区上方，对话框保持打开。 */
  error?: ReactNode;
  onCancel: () => void;
  onConfirm: () => void;
}

const DEFAULT_ICON: Record<ConfirmTone, LucideIcon> = {
  danger: TriangleAlert,
  warning: TriangleAlert,
  neutral: Info,
};

/**
 * 通用确认框：替代原生 confirm()。
 *
 * 原生弹窗会打断渲染进程、只给一行纯文本，按钮还是系统的 OK / Cancel；
 * 这里用 <dialog showModal> 铺一层应用内卡片，能带上对象名称、类型、后果提示，
 * 并把「取消」作为初始焦点，破坏性操作不会被顺手回车触发。
 */
export default function ConfirmDialog({
  tone = "danger",
  icon,
  eyebrow,
  title,
  description,
  subject,
  details,
  note,
  cancelLabel = "取消",
  confirmLabel = "删除",
  busyLabel = "处理中…",
  busy = false,
  error,
  onCancel,
  onConfirm,
}: ConfirmDialogProps) {
  const dialog = useRef<HTMLDialogElement>(null);
  const cancelButton = useRef<HTMLButtonElement>(null);
  const titleId = useId();
  const bodyId = useId();
  const Icon = icon ?? DEFAULT_ICON[tone];
  // 带中文的名称用正文字体，纯 ASCII 的地址 / UID 才用等宽字体。
  const monoLabel =
    subject?.mono ?? (subject ? !/[\u3400-\u9fff]/.test(subject.label) : false);

  useLayoutEffect(() => {
    const node = dialog.current;
    if (!node) return;
    if (!node.open) node.showModal();
    cancelButton.current?.focus({ preventScroll: true });
    return () => {
      if (node.open) node.close();
    };
  }, []);

  const dismiss = () => {
    if (!busy) onCancel();
  };

  return createPortal(
    <dialog
      ref={dialog}
      className={`confirm-dialog is-${tone}`}
      aria-labelledby={titleId}
      aria-describedby={description ? bodyId : undefined}
      onCancel={(event) => {
        event.preventDefault();
        dismiss();
      }}
      onClick={(event) => {
        if (event.target === event.currentTarget) dismiss();
      }}
    >
      <div className="confirm-card" role="document">
        <header className="confirm-head">
          <span className="confirm-icon" aria-hidden="true">
            <Icon size={16} />
          </span>
          <div className="confirm-heading">
            {eyebrow && <span className="confirm-eyebrow">{eyebrow}</span>}
            <h2 id={titleId}>{title}</h2>
          </div>
          <button
            type="button"
            className="confirm-close"
            aria-label="关闭"
            disabled={busy}
            onClick={onCancel}
          >
            <X size={14} />
          </button>
        </header>

        {description && (
          <p className="confirm-desc" id={bodyId}>
            {description}
          </p>
        )}

        {subject && (
          <div className="confirm-subject">
            <span
              className="confirm-subject-label"
              data-mono={monoLabel ? "true" : "false"}
              title={subject.label}
            >
              {subject.label}
            </span>
            {subject.tag && (
              <span className="confirm-tag" data-tone={subject.tagTone}>
                {subject.tag}
              </span>
            )}
            {subject.meta && (
              <span className="confirm-subject-meta">{subject.meta}</span>
            )}
          </div>
        )}

        {details && details.length > 0 && (
          <dl className="confirm-details">
            {details.map((detail) => (
              <div className="confirm-detail" key={detail.label}>
                <dt>{detail.label}</dt>
                <dd>{detail.value}</dd>
              </div>
            ))}
          </dl>
        )}

        {note && (
          <p className="confirm-note" data-tone={note.tone ?? tone}>
            <span className="confirm-note-icon" aria-hidden="true">
              <TriangleAlert size={13} />
            </span>
            <span>{note.text}</span>
          </p>
        )}

        {error && (
          <p className="confirm-error" role="alert">
            {error}
          </p>
        )}

        <footer className="confirm-actions">
          <span className="confirm-hint">
            <kbd>Esc</kbd> 取消
          </span>
          <div className="confirm-buttons">
            <button
              ref={cancelButton}
              type="button"
              className="confirm-btn is-plain"
              disabled={busy}
              onClick={onCancel}
            >
              {cancelLabel}
            </button>
            <button
              type="button"
              className="confirm-btn is-solid"
              disabled={busy}
              onClick={onConfirm}
            >
              {busy ? (
                <Loader2 className="confirm-spin" size={13} />
              ) : (
                <Icon size={13} />
              )}
              {busy ? busyLabel : confirmLabel}
            </button>
          </div>
        </footer>
      </div>
    </dialog>,
    document.body,
  );
}
