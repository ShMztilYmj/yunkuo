import {
  Fingerprint,
  Globe,
  Home,
  Layers,
  ScanLine,
  Settings,
  ShieldCheck,
} from "lucide-react";
import type { ThemeMode } from "../theme";
import { PROXY_ROUTES } from "../features/proxy/navigation";

export const THEME_LABEL: Record<ThemeMode, string> = {
  dark: "暗色",
  light: "亮色",
  system: "跟随系统",
};
export const THEME_NEXT: Record<ThemeMode, ThemeMode> = {
  dark: "light",
  light: "system",
  system: "dark",
};

export { PROXY_SUB, PROXY_ROUTES } from "../features/proxy/navigation";

/** 一级导航按业务职责组织，旧地址归入同一个模块。 */
export const NAV_ITEMS = [
  { to: "/", label: "首页", icon: Home, routes: ["/"] },
  {
    to: "/overview",
    label: "代理中心",
    icon: Globe,
    routes: [...PROXY_ROUTES, "/proxy"],
  },
  {
    to: "/detect",
    label: "检测与修复",
    icon: ScanLine,
    routes: ["/detect", "/diagnostics"],
  },
  {
    to: "/fingerprint-browser",
    label: "指纹浏览器",
    icon: Fingerprint,
    routes: ["/fingerprint-browser", "/environment", "/firefox"],
  },
  {
    to: "/ai",
    label: "AI 管理",
    icon: Layers,
    routes: ["/ai", "/ai-providers", "/ai-stats", "/ai-capabilities"],
  },
  {
    to: "/maintenance",
    label: "首次使用与维护",
    icon: ShieldCheck,
    routes: ["/maintenance"],
  },
  { to: "/settings", label: "设置", icon: Settings, routes: ["/settings"] },
];
export type NavItem = (typeof NAV_ITEMS)[number];
export const NAV_GROUPS = [
  { label: "工作空间", paths: ["/"] },
  {
    label: "功能模块",
    paths: ["/overview", "/detect", "/fingerprint-browser", "/ai"],
  },
  { label: "偏好与支持", paths: ["/maintenance", "/settings"] },
];
export function navItem(to: string): NavItem {
  const item = NAV_ITEMS.find((candidate) => candidate.to === to);
  if (!item) throw new Error(`未定义的主导航入口：${to}`);
  return item;
}
export function isNavigationActive(item: NavItem, pathname: string): boolean {
  return item.routes.includes(pathname);
}

/** 导航入口对应的路由分包：悬停或空闲时预取，切换页面时不必等待分包下载。 */
const ROUTE_PREFETCH: Record<string, () => Promise<unknown>> = {
  "/overview": () => import("../features/proxy/pages/Overview"),
  "/detect": () => import("../features/diagnostics/DiagnosticsPage"),
  "/fingerprint-browser": () =>
    import("../features/fingerprint-browser/FingerprintBrowserPage"),
  "/ai": () => import("../features/ai/pages/AiTool"),
  "/maintenance": () => import("./pages/Maintenance"),
  "/settings": () => import("./pages/Settings"),
};
export function prefetchRoute(to: string) {
  const load = ROUTE_PREFETCH[to];
  if (load) void load().catch(() => {});
}
