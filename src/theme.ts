import { useEffect, useReducer } from "react";

export type ThemeMode = "dark" | "light" | "system";

const KEY = "coc.theme";

/** 当前主题选择（暗/亮/跟随系统） */
export const themeStore: { mode: ThemeMode; listeners: Set<() => void> } = {
  mode: readSaved(),
  listeners: new Set(),
};

function readSaved(): ThemeMode {
  try {
    const v = localStorage.getItem(KEY);
    if (v === "dark" || v === "light" || v === "system") return v;
  } catch {
    /* 忽略 */
  }
  return "light";
}

/** 解析实际生效的主题（system 时读系统偏好） */
export function effectiveTheme(
  mode: ThemeMode = themeStore.mode,
): "dark" | "light" {
  if (mode !== "system") return mode;
  return window.matchMedia("(prefers-color-scheme: light)").matches
    ? "light"
    : "dark";
}

/** 把 data-theme 写到 html 根元素上，驱动 light-dark() 配色 */
function apply(mode: ThemeMode = themeStore.mode) {
  document.documentElement.setAttribute("data-theme", effectiveTheme(mode));
}

export function setTheme(mode: ThemeMode) {
  themeStore.mode = mode;
  try {
    localStorage.setItem(KEY, mode);
  } catch {
    /* 忽略 */
  }
  apply(mode);
  themeStore.listeners.forEach((fn) => fn());
}

/** 在 App 根组件挂一次：应用已存主题并监听系统亮暗变化 */
export function useThemeInit() {
  useEffect(() => {
    apply();
    const mq = window.matchMedia("(prefers-color-scheme: light)");
    const onChange = () => {
      if (themeStore.mode === "system") apply("system");
    };
    mq.addEventListener("change", onChange);
    const onStorage = (event: StorageEvent) => {
      if (event.key === KEY) {
        themeStore.mode = readSaved();
        apply();
        themeStore.listeners.forEach((fn) => fn());
      }
    };
    window.addEventListener("storage", onStorage);
    return () => {
      mq.removeEventListener("change", onChange);
      window.removeEventListener("storage", onStorage);
    };
  }, []);
}

/** 组件内订阅当前模式（用于高亮分段控件） */
export function useThemeMode(): ThemeMode {
  const [, force] = useReducerForce();
  useEffect(() => {
    const fn = () => force((n) => n + 1);
    themeStore.listeners.add(fn);
    return () => {
      themeStore.listeners.delete(fn);
    };
  }, [force]);
  return themeStore.mode;
}

/* 极简 useReducer 写法，避免多引状态库 */
function useReducerForce(): [number, (fn: (n: number) => number) => void] {
  return useReducer((n: number, fn: (n: number) => number) => fn(n), 0);
}
