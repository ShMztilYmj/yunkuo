import { Monitor, Moon, Sun } from "lucide-react";
import type { ReactNode } from "react";
import { setTheme, useThemeMode, type ThemeMode } from "../../../theme";

/** 界面主题分段控件：暗 / 亮 / 跟随系统 */
export default function ThemeSegmented() {
  const mode = useThemeMode();
  const opts: [ThemeMode, string, string, ReactNode][] = [
    ["dark", "暗色", "暗色主题", <Moon key="d" />],
    ["light", "亮色", "亮色主题", <Sun key="l" />],
    ["system", "系统", "跟随系统", <Monitor key="s" />],
  ];
  return (
    <div role="group" aria-label="界面主题" className="settings-segmented">
      {opts.map(([m, label, ariaLabel, icon]) => (
        <button
          key={m}
          type="button"
          onClick={() => setTheme(m)}
          aria-label={ariaLabel}
          aria-pressed={mode === m}
          data-active={mode === m}
        >
          {icon}
          {label}
        </button>
      ))}
    </div>
  );
}
