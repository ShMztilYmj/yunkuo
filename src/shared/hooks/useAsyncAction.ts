import { useRef, useState } from "react";

/** 忙碌 + 防重入 + 错误文案：异步操作期间统一锁定按钮，避免连点重入。 */
export function useAsyncAction(onBusyChange?: (busy: boolean) => void) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const running = useRef(false);
  const run = async (action: () => Promise<void>) => {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    setError(null);
    onBusyChange?.(true);
    try {
      await action();
    } catch (reason) {
      setError(String(reason));
    } finally {
      running.current = false;
      setBusy(false);
      onBusyChange?.(false);
    }
  };
  return { busy, error, setError, run };
}
