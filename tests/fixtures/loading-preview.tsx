// 只在本地预览“路由加载占位”和“设置读取占位”，供截图核对版式。
import { createRoot } from "react-dom/client";
import RouteFallback from "../../src/shared/ui/RouteFallback";
import ConfigLoadingState from "../../src/app/pages/settings/ConfigLoadingState";
import "../../src/styles/index.css";

document.documentElement.dataset.theme =
  new URLSearchParams(location.search).get("theme") === "dark" ? "dark" : "light";

createRoot(document.getElementById("root") as HTMLElement).render(
  <div style={{ display: "grid", gap: 8, padding: 8 }}>
    <div className="workspace-content">
      <RouteFallback />
    </div>
    <div className="workspace-content">
      <ConfigLoadingState configLoading configError={null} onReload={() => {}} />
    </div>
  </div>,
);
