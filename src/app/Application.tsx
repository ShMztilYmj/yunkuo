import { HashRouter, Navigate, Route, Routes } from "react-router-dom";
import Layout, { ProxyShell } from "./layout/AppShell";
import Home from "./pages/Home";
import Proxies from "../features/proxy/pages/Proxies";
import { lazy, useEffect } from "react";
import { useAppStore } from "../store";
import {
  listenCoreError,
  listenCoreEvent,
  type AppConfig,
} from "../services/api";
import { useThemeInit } from "../theme";
import { listen } from "@tauri-apps/api/event";
import { browserEnvironment } from "../features/fingerprint-browser/services/browser";
import { invalidateEnvironment } from "../features/diagnostics/services/environment";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { AiCompletionNotice } from "../features/ai/components/AiMonitorControls";
import AiShell from "../features/ai/AiShell";
import { useDiagnosticsStore } from "../features/diagnostics/state";
import { useEnvironmentVerdict } from "../features/diagnostics/hooks/useEnvironmentVerdict";
import { ENVIRONMENT_INVALIDATED } from "../shared/events/environment";
import { prefetchRoute } from "./navigation";

const Overview = lazy(() => import("../features/proxy/pages/Overview"));
const DiagnosticsPage = lazy(
  () => import("../features/diagnostics/DiagnosticsPage"),
);
const AiTool = lazy(() => import("../features/ai/pages/AiTool"));
const AiProviders = lazy(() => import("../features/ai/pages/AiProviders"));
const AiCapabilities = lazy(
  () => import("../features/ai/pages/AiCapabilities"),
);
const AiStats = lazy(() => import("../features/ai/pages/AiStats"));
const Environment = lazy(
  () => import("../features/fingerprint-browser/FingerprintBrowserPage"),
);
const Profiles = lazy(() => import("../features/proxy/pages/Profiles"));
const Connections = lazy(() => import("../features/proxy/pages/Connections"));
const Logs = lazy(() => import("../features/proxy/pages/Logs"));
const Rules = lazy(() => import("../features/proxy/pages/Rules"));
const Settings = lazy(() => import("./pages/Settings"));
const Maintenance = lazy(() => import("./pages/Maintenance"));

/** 跨模块数据由应用层组合；浏览器不直接依赖检测的内部状态。 */
function BrowserEnvironment() {
  const detect = useDiagnosticsStore((s) => s.detect);
  return (
    <Environment
      diagnostics={{
        scanning: detect.scanning,
        at: detect.at,
        score: detect.finished ? detect.webTotal : null,
        ip: detect.native?.exitIp ?? null,
        country:
          detect.native?.exitCountryName ?? detect.native?.exitCountry ?? null,
        timezone: detect.native?.currentTzIana ?? null,
      }}
    />
  );
}

/**
 * AI 页面只接收判定结果；判定本身由检测模块的唯一入口给出，
 * 与「环境检测并修复」页读取同一份快照、同一组策略参数。
 */
function AiToolRoute() {
  const gate = useEnvironmentVerdict();
  return (
    <AiTool
      gate={{
        pass: gate.pass,
        failures: gate.failures,
        scanned: gate.scanned,
      }}
    />
  );
}

export default function App() {
  const load = useAppStore((s) => s.load);
  const refreshCore = useAppStore((s) => s.refreshCore);
  const refreshErrors = useAppStore((s) => s.refreshErrors);
  useThemeInit();

  // 外壳已经绘制完成，再让启动画面淡出，避免中间露出一帧空白底色。
  useEffect(() => {
    const boot = document.getElementById("boot");
    if (!boot) return;
    boot.classList.add("boot-done");
    const timer = setTimeout(() => boot.remove(), 240);
    return () => clearTimeout(timer);
  }, []);

  // 空闲时预取主导航分包：切页时通常直接命中缓存，不再出现“加载中…”。
  useEffect(() => {
    const prefetch = () => {
      ["/overview", "/ai", "/settings"].forEach(prefetchRoute);
    };
    const idle =
      window.requestIdleCallback?.(prefetch) ??
      window.setTimeout(prefetch, 1200);
    return () => {
      if (window.cancelIdleCallback) window.cancelIdleCallback(idle as number);
      else window.clearTimeout(idle as number);
    };
  }, []);

  useEffect(() => {
    let active = true,
      attempt = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const initialize = async () => {
      if (!active || (attempt > 0 && useAppStore.getState().config)) return;
      await load().catch(console.error);
      if (active && !useAppStore.getState().config && attempt < 3) {
        timer = setTimeout(
          () => void initialize(),
          [750, 2000, 5000][attempt++],
        );
      }
    };
    void initialize();
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [load]);

  useEffect(() => {
    let active = true;
    let requestSequence = 0;
    const routeRequest = () => {
      if (active)
        window.location.hash = `/fingerprint-browser?browser-request=${++requestSequence}`;
    };
    const listening = listen("browser-request", routeRequest);
    // 先监听再读取内存状态，覆盖 React 挂载前收到的冷启动链接。
    listening
      .then(async () => {
        const state = await browserEnvironment.browserRequests();
        if (state.pendingCount || state.notice) routeRequest();
      })
      .catch(console.error);
    return () => {
      active = false;
      listening.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    let active = true;
    let observedVisibility = false;
    window.addEventListener(ENVIRONMENT_INVALIDATED, invalidateEnvironment);
    const errorListener = listenCoreError(() => {
      void refreshErrors().catch(console.error);
    });
    errorListener.then(() => refreshErrors()).catch(console.error);
    const visibilityListener = listen<boolean>("app-visibility", (event) => {
      observedVisibility = true;
      useAppStore.setState({ windowVisible: event.payload });
    });
    getCurrentWindow()
      .isVisible()
      .then((windowVisible) => {
        if (active && !observedVisibility)
          useAppStore.setState({ windowVisible });
      })
      .catch(console.error);
    const unlisteners = [
      listen<AppConfig>("config-changed", ({ payload }) =>
        useAppStore.getState().updateConfig(payload),
      ),
      listen("environment-invalidated", () => invalidateEnvironment()),
      listenCoreEvent(() => {
        void refreshCore(true).catch(console.error);
      }),
      errorListener,
      visibilityListener,
    ];
    return () => {
      active = false;
      window.removeEventListener(
        ENVIRONMENT_INVALIDATED,
        invalidateEnvironment,
      );
      unlisteners.forEach((p) => p.then((fn) => fn()));
    };
  }, [refreshCore, refreshErrors]);

  return (
    <HashRouter>
      <Routes>
        <Route element={<Layout />}>
          {/* 代理页：页签并进各子页页头右侧，默认「首页」 */}
          <Route element={<ProxyShell />}>
            <Route path="/overview" element={<Overview />} />
            <Route path="/proxies" element={<Proxies />} />
            <Route path="/profiles" element={<Profiles />} />
            <Route path="/connections" element={<Connections />} />
            <Route path="/logs" element={<Logs />} />
            <Route path="/rules" element={<Rules />} />
          </Route>
          <Route path="/" element={<Home />} />
          <Route path="/proxy" element={<Navigate to="/overview" replace />} />
          <Route path="/detect" element={<DiagnosticsPage />} />
          <Route
            path="/diagnostics"
            element={<Navigate to="/detect" replace />}
          />
          <Route path="/fingerprint-browser" element={<BrowserEnvironment />} />
          <Route
            path="/firefox"
            element={<Navigate to="/environment" replace />}
          />
          <Route path="/environment" element={<BrowserEnvironment />} />
          <Route element={<AiShell />}>
            <Route path="/ai" element={<AiToolRoute />} />
            <Route path="/ai-providers" element={<AiProviders />} />
            <Route path="/ai-capabilities" element={<AiCapabilities />} />
            <Route path="/ai-stats" element={<AiStats />} />
          </Route>
          {/* 旧链接兼容：账号入口已移除，历史地址回到启动工具页。 */}
          <Route path="/accounts" element={<Navigate to="/ai" replace />} />
          <Route path="/settings" element={<Settings />} />
          <Route path="/maintenance" element={<Maintenance />} />
          <Route path="*" element={<Navigate to="/overview" replace />} />
        </Route>
      </Routes>
      <AiCompletionNotice />
    </HashRouter>
  );
}
