import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  ArrowLeft,
  Minus,
  Monitor,
  Moon,
  PanelLeft,
  Square,
  Sun,
  X,
} from "lucide-react";
import { useLocation, useNavigate } from "react-router-dom";
import { setTheme, useThemeMode } from "../../theme";
import { NAV_ITEMS, THEME_LABEL, THEME_NEXT } from "../navigation";
import NoticeCenter from "./NoticeCenter";

interface TitleBarProps {
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
}

/** 自定义标题栏（拖拽/窗口控制 + 当前位置 + 主题快捷切换） */
export default function TitleBar({
  sidebarOpen,
  onToggleSidebar,
}: TitleBarProps) {
  const win = getCurrentWindow();
  const nav = useNavigate();
  const loc = useLocation();
  const themeMode = useThemeMode();
  const section =
    NAV_ITEMS.find((item) => item.routes.includes(loc.pathname))?.label ??
    "工作台";
  return (
    <div
      data-tauri-drag-region
      onDoubleClick={(event) => {
        if (!(event.target as HTMLElement).closest("button"))
          void win.toggleMaximize();
      }}
      className="app-titlebar"
    >
      <button
        className="chrome-button"
        aria-label="返回工作台"
        onClick={() => nav("/")}
      >
        <ArrowLeft size={16} />
      </button>
      <button
        className={`chrome-button sidebar-toggle ${sidebarOpen ? "" : "is-collapsed"}`}
        aria-label={sidebarOpen ? "收起侧栏" : "展开侧栏"}
        data-tooltip={sidebarOpen ? "收起侧栏" : "展开侧栏"}
        aria-expanded={sidebarOpen}
        aria-controls="app-sidebar"
        onClick={onToggleSidebar}
      >
        <PanelLeft size={16} />
      </button>
      <span data-tauri-drag-region className="titlebar-name">
        云阔
        <span className="titlebar-divider" aria-hidden="true">
          /
        </span>
        <span className="titlebar-section">{section}</span>
      </span>
      <div className="ml-auto flex h-full self-stretch">
        <NoticeCenter />
        <button
          aria-label="切换界面主题"
          data-tooltip={`界面主题：${THEME_LABEL[themeMode]}（点击切换）`}
          onClick={() => setTheme(THEME_NEXT[themeMode])}
          className="flex w-11 items-center justify-center text-zinc-500 hover:bg-zinc-800 hover:text-zinc-200"
        >
          {themeMode === "dark" ? (
            <Moon className="h-3.5 w-3.5" />
          ) : themeMode === "light" ? (
            <Sun className="h-3.5 w-3.5" />
          ) : (
            <Monitor className="h-3.5 w-3.5" />
          )}
        </button>
        <button
          aria-label="最小化"
          onClick={() => win.minimize()}
          className="flex w-12 items-center justify-center text-zinc-500 hover:bg-zinc-800 hover:text-zinc-200"
        >
          <Minus className="h-3.5 w-3.5" />
        </button>
        <button
          aria-label="最大化 / 还原"
          onClick={() => win.toggleMaximize()}
          className="flex w-12 items-center justify-center text-zinc-500 hover:bg-zinc-800 hover:text-zinc-200"
        >
          <Square className="h-3 w-3" />
        </button>
        <button
          aria-label="关闭"
          onClick={() => win.close()}
          className="flex w-12 items-center justify-center text-zinc-500 hover:bg-red-600 hover:text-white"
        >
          <X className="h-4 w-4" />
        </button>
      </div>
    </div>
  );
}
