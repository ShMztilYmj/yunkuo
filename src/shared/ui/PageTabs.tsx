import type { LucideIcon } from "lucide-react";
import { useLocation, useNavigate } from "react-router-dom";

export interface PageTab {
  to: string;
  label: string;
  icon: LucideIcon;
}

/**
 * 模块页签：并进页头右侧，与页面动作同一行。
 * 代理与 AI 两个模块共用这一份实现，切页时页签位置不跳动。
 */
export default function PageTabs({
  label,
  tabs,
}: {
  label: string;
  tabs: readonly PageTab[];
}) {
  const nav = useNavigate();
  const loc = useLocation();
  return (
    <nav className="proxy-page-tabs" aria-label={label}>
      {tabs.map(({ to, label: title, icon: Icon }) => {
        const active = loc.pathname === to;
        return (
          <button
            key={to}
            type="button"
            onClick={() => nav(to)}
            aria-current={active ? "page" : undefined}
            data-active={active}
            className="proxy-page-tab"
          >
            <Icon aria-hidden="true" />
            <span>{title}</span>
          </button>
        );
      })}
    </nav>
  );
}
