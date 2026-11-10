import { call } from "./api/invoke";

export type FrontendFault = "render" | "rejection" | "script";
const lastReported = new Map<FrontendFault, number>();

/** 仅发送固定类别，不发送错误消息、页面内容、URL 或堆栈。 */
export function recordFault(kind: FrontendFault) {
  const now = Date.now();
  if (now - (lastReported.get(kind) ?? -Infinity) < 30_000) return;
  lastReported.set(kind, now);
  // 日志本身失败时不能制造新的未处理 Promise。
  void call("record_frontend_fault", { kind }).catch(() => {});
}

export function installFaultListeners() {
  window.addEventListener("error", () => recordFault("script"));
  window.addEventListener("unhandledrejection", () => recordFault("rejection"));
}
