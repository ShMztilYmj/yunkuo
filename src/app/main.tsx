import { lazy, StrictMode, Suspense } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import TooltipLayer from "../shared/ui/TooltipLayer";
import ErrorBoundary from "../shared/ui/ErrorBoundary";
import { installFaultListeners } from "../services/faults";
import { bindAiUsageScope } from "./aiUsageScope";
import { useAppStore } from "../store";
import "../styles/index.css";

const surface = new URLSearchParams(window.location.search).get("view");
const tray = surface === "tray";
const hud = surface === "hud";
const App = lazy(() => import("./Application"));
const TrayMenu = lazy(() => import("./windows/TrayMenu"));
const AgentHud = lazy(() => import("./windows/AgentHud"));
if (tray) document.documentElement.dataset.surface = "tray";
if (hud) document.documentElement.dataset.surface = "hud";
// 辅助窗口在原生 ready 握手完成前不可见，不提前启动轮询和媒体。
if (tray || hud) useAppStore.setState({ windowVisible: false });

installFaultListeners();
// 每个WebView都有独立store，主窗口、HUD、托盘均需装配相同的AI范围同步。
const unbindAiUsageScope = bindAiUsageScope(useAppStore);
if (import.meta.hot) import.meta.hot.dispose(unbindAiUsageScope);
createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>
    <ErrorBoundary>
      <Suspense fallback={null}>
        {tray ? (
          <TrayMenu />
        ) : hud ? (
          <AgentHud />
        ) : (
          <>
            <App />
            <TooltipLayer />
          </>
        )}
      </Suspense>
    </ErrorBoundary>
  </StrictMode>,
);

// 主窗口在配置里以隐藏状态创建：等启动画面完成首帧再显示，
// 用户看到的是品牌底色而不是 WebView2 初始化时的白屏。静默启动由原生侧忽略。
if (!tray && !hud) {
  requestAnimationFrame(() =>
    requestAnimationFrame(() => {
      invoke("main_show").catch(() => {});
    }),
  );
  // 渲染阶段就抛错时 App 的 effect 不会执行，启动画面要有兜底移除。
  setTimeout(() => document.getElementById("boot")?.remove(), 8000);
}
