import { useNavigate } from "react-router-dom";
import { useAppStore } from "../../store";
import ProxyQuickControl from "../../features/proxy/components/ProxyQuickControl";
import SidebarNavigation from "./SidebarNavigation";

interface AppSidebarProps {
  sidebarOpen: boolean;
}

/** 主侧栏：品牌入口 + 主导航 + 底部连接状态与代理快捷控制 */
export default function AppSidebar({ sidebarOpen }: AppSidebarProps) {
  const nav = useNavigate();
  const coreStatus = useAppStore((s) => s.coreStatus);

  return (
    <aside
      id="app-sidebar"
      className={
        "app-sidebar " + (sidebarOpen ? "is-expanded" : "is-collapsed")
      }
      aria-label="主侧栏"
      data-collapsed={!sidebarOpen}
    >
      <button
        className="sidebar-brand"
        onClick={() => nav("/")}
        aria-label="云阔工作台"
        data-tooltip="云阔 · 自在工作"
      >
        <span className="brand-symbol sidebar-icon" aria-hidden="true" />
        <span className="sidebar-copy brand-name">
          云阔 <span className="brand-caption">自在工作</span>
        </span>
      </button>
      <SidebarNavigation />
      <div className="sidebar-bottom">
        <div className="sidebar-footer">
          <span
            className={
              "status-dot sidebar-copy " +
              (coreStatus?.running ? "is-running" : "")
            }
          />
          <span className="sidebar-copy">
            {coreStatus?.running ? "本地服务已连接" : "本地服务未连接"}
          </span>
          <ProxyQuickControl />
        </div>
      </div>
    </aside>
  );
}
