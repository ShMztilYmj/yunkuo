import { useEffect, useRef, useState } from "react";
import { Eraser, Pause, Play, Terminal } from "lucide-react";
import { useProxyStore } from "../state";
import { subscribeLogs } from "../services/streams";
import ProxyTabs from "../components/ProxyTabs";
import PageHeader from "../../../shared/ui/PageHeader";
import { usePageActive } from "../../../shared/hooks/usePageActive";

interface LogItem {
  id: number;
  type: string;
  payload: string;
  time: string;
}

const timeFormat = new Intl.DateTimeFormat("zh-CN", {
  hour12: false,
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
});

export default function Logs() {
  const config = useProxyStore((s) => s.config);
  const active = usePageActive();
  const coreRunning = useProxyStore((s) => s.coreStatus?.running === true);
  const [logs, setLogs] = useState<LogItem[]>([]);
  const [paused, setPaused] = useState(false);
  const [streamError, setStreamError] = useState<string | null>(null);
  const boxRef = useRef<HTMLDivElement>(null);
  const pending = useRef<LogItem[]>([]);
  const nextId = useRef(0);
  const followTail = useRef(true);

  useEffect(() => {
    if (!config || !active || !coreRunning || paused) return;
    let flushTimer: ReturnType<typeof setTimeout> | undefined;
    const flush = () => {
      flushTimer = undefined;
      const batch = pending.current;
      pending.current = [];
      setLogs((prev) => [...prev, ...batch].slice(-500));
    };

    const unsubscribe = subscribeLogs(config, {
      onConnected: (connected) => {
        if (connected) setStreamError(null);
      },
      onError: setStreamError,
      onValue: (log) => {
        pending.current.push({
          id: ++nextId.current,
          type: log.type,
          payload: log.payload,
          time: timeFormat.format(new Date(log.timestamp)),
        });
        if (pending.current.length > 500)
          pending.current.splice(0, pending.current.length - 500);
        if (flushTimer === undefined) flushTimer = setTimeout(flush, 100);
      },
    });

    return () => {
      unsubscribe();
      clearTimeout(flushTimer);
      pending.current = [];
    };
  }, [
    config?.external_controller_port,
    config?.log_level,
    config?.secret,
    active,
    coreRunning,
    paused,
  ]);

  // 自动滚动
  useEffect(() => {
    const box = boxRef.current;
    if (box && followTail.current) box.scrollTop = box.scrollHeight;
  }, [logs]);

  const streaming = coreRunning && !paused;
  const streamHealthy = streaming && !streamError;
  return (
    <div className="proxy-data-page logs-page">
      <PageHeader
        icon={Terminal}
        title="内核日志"
        badge={`${logs.length} 行`}
        meta={
          <>
            <span
              className="proxy-live-dot"
              data-live={streamHealthy}
              data-error={Boolean(streamError)}
              aria-hidden="true"
            />
            <span>
              {streamError
                ? "日志流中断"
                : streaming
                  ? "实时输出"
                  : paused
                    ? "已暂停接收"
                    : coreRunning
                      ? "等待内核输出"
                      : "内核未运行"}
            </span>
            <i className="proxy-meta-dot" />
            <span>最多保留 500 行</span>
            <i className="proxy-meta-dot" />
            <span>仅显示当前会话</span>
          </>
        }
        actions={
          <>
            <button
              type="button"
              onClick={() => setPaused((p) => !p)}
              className={`proxy-action${paused ? " is-warn" : ""}`}
            >
              {paused ? <Play /> : <Pause />}
              {paused ? "继续" : "暂停"}
            </button>
            <button
              type="button"
              onClick={() => {
                pending.current = [];
                followTail.current = true;
                setLogs([]);
              }}
              className="proxy-action"
            >
              <Eraser /> 清空
            </button>
          </>
        }
        tabs={<ProxyTabs />}
      />

      <div className="proxy-data-content">
        <section className="proxy-panel log-panel" aria-label="内核日志输出">
          <div className="log-panel-head">
            <span
              className="log-live"
              data-live={streamHealthy}
              data-error={Boolean(streamError)}
            >
              <i aria-hidden="true" />
              {streamError
                ? "日志流中断"
                : streaming
                  ? "实时输出"
                  : paused
                    ? "已暂停"
                    : "等待日志"}
            </span>
            {streamError && (
              <span className="log-error" title={streamError}>
                {streamError}
              </span>
            )}
            <span className="log-count">{logs.length} 行</span>
          </div>
          <div
            ref={boxRef}
            onScroll={(event) => {
              const box = event.currentTarget;
              followTail.current =
                box.scrollHeight - box.scrollTop - box.clientHeight < 32;
            }}
            className="log-panel-body"
          >
            {logs.length === 0 ? (
              <div className="proxy-empty">
                <span className="proxy-empty-icon">
                  <Terminal size={20} />
                </span>
                <strong>等待内核日志</strong>
                <p>
                  内核运行后，mihomo
                  的输出会实时显示在这里；暂停后仍可继续接收，清空只影响当前窗口。
                </p>
              </div>
            ) : (
              logs.map((l) => (
                <div key={l.id} className="log-line" data-type={l.type}>
                  <span className="log-time">{l.time}</span>
                  <span className="log-level" data-type={l.type}>
                    {l.type}
                  </span>
                  <span className="log-text">{l.payload}</span>
                </div>
              ))
            )}
          </div>
        </section>
      </div>
    </div>
  );
}
