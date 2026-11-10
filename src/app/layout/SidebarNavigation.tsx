import { Fragment } from "react";
import { ChevronRight } from "lucide-react";
import { useLocation, useNavigate } from "react-router-dom";
import {
  NAV_GROUPS,
  isNavigationActive,
  navItem,
  prefetchRoute,
  type NavItem,
} from "../navigation";
import DetectStatusPanel from "./DetectStatusPanel";

/** 主导航。分组与顺序见 nav.ts 的 NAV_GROUPS；环境检测用带状态点的卡片，其余是普通入口。 */
export default function SidebarNavigation() {
  const nav = useNavigate();
  const loc = useLocation();
  const navButton = (item: NavItem) => {
    const active = isNavigationActive(item, loc.pathname);
    const Icon = item.icon;
    return (
      <button
        key={item.to}
        onClick={() => nav(item.to)}
        onPointerEnter={() => prefetchRoute(item.to)}
        onFocus={() => prefetchRoute(item.to)}
        aria-label={item.label}
        data-tooltip={item.label}
        aria-current={active ? "page" : undefined}
        className={"sidebar-link " + (active ? "is-active" : "")}
      >
        <Icon size={19} className="sidebar-icon" />
        <span className="sidebar-copy">{item.label}</span>
        {item.to === "/overview" && (
          <ChevronRight
            size={14}
            className="sidebar-trailing ml-auto text-zinc-500"
          />
        )}
      </button>
    );
  };

  return (
    <nav className="sidebar-navigation" aria-label="主导航">
      {NAV_GROUPS.map((group) => (
        <Fragment key={group.label}>
          <div className="sidebar-section-label">
            <span>{group.label}</span>
          </div>
          {group.paths.map((path) =>
            path === "/detect" ? (
              <DetectStatusPanel key={path} />
            ) : (
              navButton(navItem(path))
            ),
          )}
        </Fragment>
      ))}
    </nav>
  );
}
