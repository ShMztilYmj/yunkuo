import { listen } from "@tauri-apps/api/event";

// ---------- 事件监听 ----------

export function listenCoreError(cb: (msg: string) => void) {
  return listen<string>("core-error", (e) => cb(e.payload));
}

export function listenCoreEvent(cb: (kind: string) => void) {
  return listen<string>("core-event", (e) => cb(e.payload));
}
