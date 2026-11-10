import { useSyncExternalStore } from "react";
import { useAppStore } from "../../store";

const listeners = new Set<() => void>();
const changed = () => listeners.forEach((listener) => listener());
function subscribe(listener: () => void) {
  if (!listeners.size) document.addEventListener("visibilitychange", changed);
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
    if (!listeners.size)
      document.removeEventListener("visibilitychange", changed);
  };
}
const documentVisible = () => !document.hidden;

/** 同时跟踪原生托盘隐藏和 WebView 页面可见性；所有调用者共用一个 DOM 监听器。 */
export function usePageActive() {
  const windowVisible = useAppStore((state) => state.windowVisible);
  const pageVisible = useSyncExternalStore(subscribe, documentVisible);
  return windowVisible && pageVisible;
}
