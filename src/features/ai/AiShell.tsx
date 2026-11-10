import { Outlet } from "react-router-dom";

/** AI 模块外壳：页签已并进各子页页头右侧，这里只保留滚动容器。 */
export default function AiShell() {
  return (
    <div className="ai-shell flex h-full flex-col">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <Outlet />
      </div>
    </div>
  );
}
