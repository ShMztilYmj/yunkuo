import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";

export type PageHeadTone = "ready" | "attention" | "pending";

/**
 * 全应用统一页头。四个模块（代理 / 检测 / 指纹浏览器 / AI）共用这一份版式：
 *
 *   [图标] 标题 (徽标)                     [动作…] [模块页签]
 *          状态行 · 状态行 · 状态
 *   说明行（固定占位，没有内容时也留出同样高度）
 *
 * 位置与尺寸只在 `styles/page-head.css` 里定义一次，页面只提供内容，
 * 不允许再写自己的标题尺寸或内边距，避免切页时页头上下跳动。
 */
export default function PageHeader({
  icon: Icon,
  title,
  badge,
  badgeTooltip,
  meta,
  metaTone,
  actions,
  tabs,
  note,
  className,
}: {
  icon: LucideIcon;
  title: ReactNode;
  /** 标题右侧的胶囊徽标，例如版本号、评分、分组数 */
  badge?: ReactNode;
  /** 徽标的悬浮说明（可选） */
  badgeTooltip?: string;
  /** 标题下方的一行状态：用 `·` 分隔，状态词用 <strong> 着色 */
  meta?: ReactNode;
  metaTone?: PageHeadTone;
  /** 页头右侧的动作按钮，排在模块页签左边 */
  actions?: ReactNode;
  /** 模块页签（ProxyTabs / AiTabs），永远贴在最右侧 */
  tabs?: ReactNode;
  /** 头部说明行：提示、进度、卡组状态；为空时只占位不显示文字 */
  note?: ReactNode;
  className?: string;
}) {
  return (
    <div className="page-head">
      <header className={`proxy-page-head${className ? ` ${className}` : ""}`}>
        <div className="proxy-page-title">
          <span className="proxy-page-icon" aria-hidden="true">
            <Icon size={17} />
          </span>
          <div>
            <h1>
              {title}
              {badge != null && (
                <span className="proxy-title-count" data-tooltip={badgeTooltip}>
                  {badge}
                </span>
              )}
            </h1>
            <p className="proxy-page-meta" data-tone={metaTone}>
              {meta}
            </p>
          </div>
        </div>
        {actions != null && <div className="proxy-page-actions">{actions}</div>}
        {tabs}
      </header>
      <div className="page-head-note">{note}</div>
    </div>
  );
}
