import type { ReactNode } from "react";

/**
 * 设置分组卡片：图标 + 标题 + 说明，右侧可放一枚状态胶囊。
 * 行内容由调用方提供，行之间的分隔线由 settings.css 统一处理。
 */
export default function Card({
  id,
  title,
  icon,
  description,
  badge,
  badgeTone = "idle",
  children,
}: {
  /** 供页内锚点跳转使用（如保护体检） */
  id?: string;
  title: string;
  icon: ReactNode;
  /** 标题下的一行说明：说清这一组设置影响什么 */
  description?: string;
  /** 卡片右上角状态胶囊：运行中 / 已关闭 / 已配置 */
  badge?: ReactNode;
  badgeTone?: "idle" | "ok" | "warn" | "danger";
  children: ReactNode;
}) {
  return (
    <section className="settings-card" id={id} aria-label={title}>
      <header className="settings-card-head">
        <span className="settings-card-icon" aria-hidden="true">
          {icon}
        </span>
        <div className="settings-card-copy">
          <h3>{title}</h3>
          {description && <p>{description}</p>}
        </div>
        {badge != null && (
          <span className="settings-card-badge" data-tone={badgeTone}>
            {badgeTone !== "idle" && <i aria-hidden="true" />}
            {badge}
          </span>
        )}
      </header>
      <div className="settings-rows">{children}</div>
    </section>
  );
}
