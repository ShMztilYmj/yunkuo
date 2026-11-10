import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { getVersion } from "@tauri-apps/api/app";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Zap } from "lucide-react";
import { api, type AppConfig, type CoreStatus } from "../../services/api";
import { formatTrafficBytes } from "../../features/proxy/services/subscriptionTraffic";
import { tray } from "../services/tray";
import { useThemeInit } from "../../theme";
import { useAppStore } from "../../store";
import { useTauriEvent } from "../../shared/hooks/useTauriEvent";

const MODE_LABEL: Record<string, string> = {
  rule: "规则",
  global: "全局",
  direct: "直连",
};

export default function TrayMenu() {
  useThemeInit();
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [core, setCore] = useState<CoreStatus | null>(null);
  const [traffic, setTraffic] = useState<{
    upload: number;
    download: number;
  } | null>(null);
  const [version, setVersion] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<{
    error: boolean;
    text: string;
  } | null>(null);
  const running = useRef(false);
  const request = useRef(0);
  const panel = useRef<HTMLElement>(null);
  const refresh = useCallback(async () => {
    const current = ++request.current;
    const [nextCore, nextConfig, nextTraffic] = await Promise.all([
      api.coreStatus().catch(() => null),
      api.getConfig().catch(() => null),
      api.connectionTotals().catch(() => null),
    ]);
    if (current !== request.current) return;
    setCore(nextCore);
    setConfig(nextConfig);
    setTraffic(
      nextTraffic
        ? {
            upload: nextTraffic.uploadTotal,
            download: nextTraffic.downloadTotal,
          }
        : null,
    );
  }, []);

  // 原生冷启动会在 ready 后立刻发出首次打开事件，必须等四个监听都注册完成再握手。
  const registered = useRef(0);
  const readyCalled = useRef(false);
  const markRegistered = () => {
    registered.current += 1;
    if (registered.current < 4 || readyCalled.current) return;
    readyCalled.current = true;
    void tray.ready().catch(console.error);
  };
  useTauriEvent(
    "tray-opened",
    () => {
      useAppStore.setState({ windowVisible: true });
      setMessage(null);
      void refresh();
      if (!window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
        panel.current?.animate(
          [
            { opacity: 0, transform: "translateY(6px) scale(.985)" },
            { opacity: 1, transform: "none" },
          ],
          { duration: 170, easing: "cubic-bezier(.2,.75,.25,1)" },
        );
      }
      panel.current
        ?.querySelector<HTMLButtonElement>("button")
        ?.focus({ preventScroll: true });
    },
    true,
    markRegistered,
  );
  useTauriEvent<boolean>(
    "tray-visibility",
    (payload) => {
      useAppStore.setState({ windowVisible: payload });
    },
    true,
    markRegistered,
  );
  useTauriEvent(
    "config-changed",
    () => {
      if (useAppStore.getState().windowVisible) void refresh();
    },
    true,
    markRegistered,
  );
  useTauriEvent(
    "core-event",
    () => {
      if (useAppStore.getState().windowVisible) void refresh();
    },
    true,
    markRegistered,
  );
  useEffect(() => {
    let active = true;
    void refresh();
    getVersion()
      .then((value) => {
        if (active) setVersion(value);
      })
      .catch(() => {});
    // 面板可能开着不动：这段时间里让牌心的流量与状态跟着走。
    const ticker = window.setInterval(() => {
      if (useAppStore.getState().windowVisible) void refresh();
    }, 3000);
    // 握手交给上面的 markRegistered：四个监听都注册完成才 tray.ready()。
    return () => {
      active = false;
      window.clearInterval(ticker);
      ++request.current;
    };
  }, [refresh]);

  const dismiss = () => {
    useAppStore.setState({ windowVisible: false });
    void getCurrentWindow().hide().catch(console.error);
  };
  const act = async (action: string) => {
    if (running.current) return;
    running.current = true;
    setBusy(action);
    setMessage(null);
    try {
      const result = await tray.action(action);
      if (
        ["show", "launch", "detect", "settings", "data", "ai-stats"].includes(
          action,
        )
      )
        dismiss();
      else {
        await refresh();
        if (result) setMessage({ error: false, text: result });
      }
    } catch (error) {
      setMessage({ error: true, text: String(error) });
      await refresh();
    } finally {
      running.current = false;
      setBusy(null);
    }
  };
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === "Escape") {
      event.preventDefault();
      dismiss();
      return;
    }
    const buttons = Array.from(
      panel.current?.querySelectorAll<HTMLButtonElement>(
        "button:not(:disabled)",
      ) ?? [],
    );
    if (!buttons.length) return;
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
    let next: number | null = null;
    if (event.key === "ArrowDown") next = (index + 1) % buttons.length;
    if (event.key === "ArrowUp")
      next = (index - 1 + buttons.length) % buttons.length;
    if (event.key === "Home") next = 0;
    if (event.key === "End") next = buttons.length - 1;
    if (next != null) {
      event.preventDefault();
      buttons[next].focus();
    }
  };

  // 牌心显示：当前网络方案 / 路由方式，两个开关的实际状态，本次会话流量。
  const modeLabel = config?.network_mode
    ? config.network_mode === "daily"
      ? "日常分流"
      : "严格保护"
    : (MODE_LABEL[config?.mode ?? "rule"] ?? "规则");
  const online = !!core?.running;
  return (
    <div
      className="tray-viewport"
      onKeyDown={onKeyDown}
      onContextMenu={(event) => event.preventDefault()}
      onPointerDown={(event) => {
        if (event.target === event.currentTarget) dismiss();
      }}
    >
      <section
        ref={panel}
        role="dialog"
        aria-label="云阔快捷面板"
        className="tray-card"
      >
        <header className="tray-header">
          <h1 title={version ? `云阔 v${version}` : "云阔"}>云阔</h1>
          <span
            className={`tray-core ${online ? "is-online" : ""}`}
            title={core ? (online ? "内核运行中" : "内核未运行") : "状态未知"}
          >
            <i />
            {core ? (online ? "在线" : "离线") : "未知"}
          </span>
        </header>

        <div className="tray-content">
          <div className="tray-figure">
            <span className="tray-suit" aria-hidden="true">
              <Zap size={20} strokeWidth={1.6} />
            </span>
            <strong className={online ? "" : "is-idle"}>
              {online ? modeLabel : "未运行"}
            </strong>
            <div className="tray-flags">
              <span>
                <i className={core?.system_proxy ? "is-on" : ""} />
                系统代理
              </span>
              <span>
                <i className={core?.tun ? "is-on" : ""} />
                TUN
              </span>
            </div>
            <p className="tray-traffic">
              {online
                ? `↑ ${formatTrafficBytes(traffic?.upload ?? 0)}　↓ ${formatTrafficBytes(traffic?.download ?? 0)}`
                : "内核已停止"}
            </p>
          </div>

          {message && (
            <p
              className={`tray-message ${message.error ? "is-error" : ""}`}
              role={message.error ? "alert" : "status"}
            >
              {message.text}
            </p>
          )}

          <button
            type="button"
            className="tray-open"
            disabled={busy != null}
            onClick={() => void act("show")}
          >
            打开
          </button>
          <button
            type="button"
            className="tray-exit"
            disabled={busy != null}
            onClick={() => void act("quit")}
          >
            退出
          </button>
        </div>
      </section>
    </div>
  );
}
