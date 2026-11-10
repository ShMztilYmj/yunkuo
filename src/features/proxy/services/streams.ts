/** 当前内核运输适配，业务页面不接触控制器路径。切换内核时只替换此实现。 */
interface StreamConfig {
  external_controller_port: number;
  secret: string;
}

export interface TrafficSample {
  up: number;
  down: number;
}
export interface ProxyLog {
  type: string;
  payload: string;
  timestamp: number;
}

interface StreamObserver<T> {
  onValue: (value: T) => void;
  onConnected?: (connected: boolean) => void;
  onError?: (message: string) => void;
}

function subscribe<T>(
  config: StreamConfig,
  path: string,
  query: Record<string, string>,
  decode: (value: unknown) => T | null,
  observer: StreamObserver<T>,
): () => void {
  let stopped = false;
  let socket: WebSocket | undefined;
  let retry: ReturnType<typeof setTimeout> | undefined;
  const reconnect = () => {
    clearTimeout(retry);
    if (!stopped) retry = setTimeout(connect, 3000);
  };
  const connect = () => {
    if (stopped) return;
    retry = undefined;
    try {
      const search = new URLSearchParams({ ...query, token: config.secret });
      const current = new WebSocket(
        `ws://127.0.0.1:${config.external_controller_port}/${path}?${search}`,
      );
      socket = current;
      current.onopen = () => {
        if (!stopped && socket === current) observer.onConnected?.(true);
      };
      current.onmessage = ({ data }) => {
        if (stopped || socket !== current) return;
        try {
          const value = decode(JSON.parse(String(data)));
          if (value !== null) observer.onValue(value);
        } catch {
          /* 非法采样不进入业务状态。 */
        }
      };
      current.onclose = () => {
        if (stopped || socket !== current) return;
        socket = undefined;
        observer.onConnected?.(false);
        reconnect();
      };
      current.onerror = () => {
        if (stopped || socket !== current) return;
        observer.onError?.("无法连接代理数据流（内核可能未运行）");
        current.close();
      };
    } catch (reason) {
      observer.onConnected?.(false);
      observer.onError?.(String(reason));
      reconnect();
    }
  };
  connect();
  return () => {
    stopped = true;
    clearTimeout(retry);
    if (socket) {
      socket.onopen = null;
      socket.onmessage = null;
      socket.onclose = null;
      socket.onerror = null;
      socket.close();
      socket = undefined;
    }
  };
}

function object(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null
    ? (value as Record<string, unknown>)
    : null;
}

export function subscribeTraffic(
  config: StreamConfig,
  observer: StreamObserver<TrafficSample>,
): () => void {
  return subscribe(
    config,
    "traffic",
    {},
    (value) => {
      const sample = object(value);
      return sample &&
        typeof sample.up === "number" &&
        Number.isFinite(sample.up) &&
        sample.up >= 0 &&
        typeof sample.down === "number" &&
        Number.isFinite(sample.down) &&
        sample.down >= 0
        ? { up: sample.up, down: sample.down }
        : null;
    },
    observer,
  );
}

export function subscribeLogs(
  config: StreamConfig & { log_level: string },
  observer: StreamObserver<ProxyLog>,
): () => void {
  return subscribe(
    config,
    "logs",
    { level: config.log_level },
    (value) => {
      const log = object(value);
      if (!log || typeof log.payload !== "string") return null;
      const parsed =
        log.ts == null
          ? Date.now()
          : Number.isFinite(Number(log.ts))
            ? Number(log.ts) * 1000
            : Date.parse(String(log.ts));
      return {
        type: typeof log.type === "string" ? log.type : "info",
        payload: log.payload,
        timestamp: Number.isFinite(parsed) ? parsed : Date.now(),
      };
    },
    observer,
  );
}
