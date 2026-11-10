import { Outlet } from "react-router-dom";

/** 代理页外壳：页签导航已并进各子页页头右侧，这里只保留滚动容器 */
export default function ProxyShell() {
  return (
    <div className="proxy-shell flex h-full flex-col">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <Outlet />
      </div>
    </div>
  );
}
