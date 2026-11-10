import { useCallback, useEffect, useRef, useState } from "react";
import useConfirm from "../../../shared/ui/useConfirm";

/** 第一次点击只进入待确认态，超时就撤销，避免"隔很久再点一下"被当成确认。 */
const ARM_TIMEOUT_MS = 6000;

/**
 * IPv6 开启前的两道闸：第一次点击只是待确认，第二次点击弹出说明框，
 * 确认后才真正下发。关闭不需要确认。
 *
 * 用法：
 *   const ipv6Guard = useIpv6ToggleGuard();
 *   if (!config.ipv6 && !ipv6Guard.armed) return ipv6Guard.arm();
 *   if (!config.ipv6 && !(await ipv6Guard.confirmEnable())) return;
 *   // …下发 patch
 *   return <>{ipv6Guard.dialog}</>;
 */
export function useIpv6ToggleGuard() {
  const { confirm, dialog } = useConfirm();
  const [armed, setArmed] = useState(false);
  const timer = useRef<number | null>(null);

  const reset = useCallback(() => {
    if (timer.current !== null) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
    setArmed(false);
  }, []);

  // 组件卸载时清掉未触发的定时器。
  useEffect(() => reset, [reset]);

  const arm = useCallback(() => {
    if (timer.current !== null) window.clearTimeout(timer.current);
    setArmed(true);
    timer.current = window.setTimeout(() => {
      timer.current = null;
      setArmed(false);
    }, ARM_TIMEOUT_MS);
  }, []);

  const confirmEnable = useCallback(async () => {
    const ok = await confirm({
      tone: "warning",
      eyebrow: "IPv6",
      title: "确认开启 IPv6？",
      description:
        "开启后代理内核会解析并使用 IPv6。开了 TUN 虚拟网卡时，IPv6 流量会被内核接管；若当前节点不支持 IPv6，部分网站的 IPv6 连接会直接失败或明显变慢（浏览器一般会自动回退 IPv4）。",
      note: {
        text: "不确定就保持关闭：关闭时域名只解析 IPv4，兼容性最好，也不会出现半通的 IPv6 连接。",
      },
      confirmLabel: "仍然开启",
      cancelLabel: "保持关闭",
    });
    reset();
    return ok;
  }, [confirm, reset]);

  return { armed, arm, reset, confirmEnable, dialog };
}

export default useIpv6ToggleGuard;
