import { useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";

/**
 * 订阅 Tauri 事件：回调存在 ref 里避免重订阅；卸载后不再投递，异步 unlisten 也不会漏。
 *
 * `onRegistered` 在注册真正完成后回调一次。托盘与悬浮条必须等全部监听就绪再向原生侧
 * 握手，否则原生 ready 后立刻发出的事件会丢（见 tests/fixtures/tray.html 的时序用例）。
 */
export function useTauriEvent<T = unknown>(
  event: string,
  handler: (payload: T) => void,
  enabled = true,
  onRegistered?: () => void,
): void {
  const handlerRef = useRef(handler);
  handlerRef.current = handler;
  const registeredRef = useRef(onRegistered);
  registeredRef.current = onRegistered;
  useEffect(() => {
    if (!enabled) return;
    let active = true;
    const subscription = listen<T>(event, (message) => {
      if (active) handlerRef.current(message.payload);
    });
    // 解绑统一交给下面的 cleanup，这里只负责报告"订阅已就绪"。
    void subscription
      .then(() => {
        if (active) registeredRef.current?.();
      })
      .catch(console.error);
    return () => {
      active = false;
      void subscription.then((unlisten) => unlisten()).catch(console.error);
    };
  }, [event, enabled]);
}
