import { useCallback, useRef, useState } from "react";
import ConfirmDialog, { type ConfirmDialogProps } from "./ConfirmDialog";

/** 除两个回调外的全部确认框参数。 */
export type ConfirmOptions = Omit<ConfirmDialogProps, "onCancel" | "onConfirm">;

interface PendingRequest {
  options: ConfirmOptions;
  resolve: (ok: boolean) => void;
}

/**
 * 把确认框写成一次 await，用法和原来的原生 ask() 保持一致：
 *
 *   const { confirm, dialog } = useConfirm();
 *   if (!(await confirm({ title: "…", tone: "warning" }))) return;
 *   await doWork();
 *
 *   return <div>…{dialog}</div>;
 *
 * 确认框关掉后再执行真实操作，所以这里不带 busy：耗时进度仍由页面自己的按钮状态表达。
 */
export function useConfirm() {
  const [request, setRequest] = useState<PendingRequest | null>(null);
  const pending = useRef<((ok: boolean) => void) | null>(null);

  const confirm = useCallback(
    (options: ConfirmOptions) =>
      new Promise<boolean>((resolve) => {
        // 同一时刻只留一个确认框：后来的请求先让前一个判否，避免 promise 悬着。
        pending.current?.(false);
        pending.current = resolve;
        setRequest({ options, resolve });
      }),
    [],
  );

  const settle = (ok: boolean) => {
    const resolve = pending.current;
    pending.current = null;
    setRequest(null);
    resolve?.(ok);
  };

  const dialog = request ? (
    <ConfirmDialog
      {...request.options}
      onCancel={() => settle(false)}
      onConfirm={() => settle(true)}
    />
  ) : null;

  return { confirm, dialog };
}

export default useConfirm;
