import { useEffect, useRef } from "react";
import { usePageActive } from "./usePageActive";

/** 完成后再计时；隐藏时暂停，旧页面结果可通过 isCurrent 拒绝提交。 */
export function usePolling(
  task: (isCurrent: () => boolean) => Promise<unknown>,
  interval: number,
  immediate = true,
  enabled = true,
  refreshKey: string | number = 0,
) {
  const visible = usePageActive() && enabled;
  const taskRef = useRef(task);
  const pending = useRef<Promise<unknown> | null>(null);
  taskRef.current = task;
  useEffect(() => {
    let active = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      if (!active) return;
      try {
        // 恢复显示时先等上一轮退出，随后立即刷新；跨 effect 也不能并发。
        if (pending.current) await pending.current.catch(() => {});
        if (!active) return;
        const request = Promise.resolve().then(() =>
          taskRef.current(() => active),
        );
        pending.current = request;
        try {
          await request;
        } finally {
          if (pending.current === request) pending.current = null;
        }
      } catch (error) {
        console.error(error);
      }
      if (active) timer = setTimeout(tick, interval);
    };
    if (visible) timer = setTimeout(tick, immediate ? 0 : interval);
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [visible, interval, immediate, refreshKey]);
}
