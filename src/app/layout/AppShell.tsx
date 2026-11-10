import { useEffect, useState } from "react";
import { usePolling } from "../../shared/hooks/usePolling";
import { useAppStore } from "../../store";
import AppSidebar from "./AppSidebar";
import ExitConfirm from "./ExitConfirm";
import TitleBar from "./TitleBar";
import WorkspaceContent from "./WorkspaceContent";

/** 应用外壳：自定义标题栏 + 主侧栏 + 工作区；路由内容由 WorkspaceContent 内的 Outlet 渲染 */
export default function Layout() {
  const [sidebarOpen, setSidebarOpen] = useState(() => {
    try {
      return localStorage.getItem("coc.sidebarCollapsed") !== "true";
    } catch {
      return true;
    }
  });
  useEffect(() => {
    try {
      localStorage.setItem("coc.sidebarCollapsed", String(!sidebarOpen));
    } catch {
      /* 保持本次会话可用 */
    }
  }, [sidebarOpen]);
  const refreshCore = useAppStore((s) => s.refreshCore);
  usePolling(() => refreshCore(), 3000);

  return (
    <div className="app-shell">
      <TitleBar
        sidebarOpen={sidebarOpen}
        onToggleSidebar={() => setSidebarOpen((open) => !open)}
      />
      <div className="app-body">
        <AppSidebar sidebarOpen={sidebarOpen} />
        <WorkspaceContent />
      </div>
      <ExitConfirm />
    </div>
  );
}

/** 代理页外壳（页签已并进子页页头右侧）仍从本模块导出，App.tsx 的引用保持不变 */
export { default as ProxyShell } from "../../features/proxy/components/ProxyShell";
