import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  ArrowUpRight,
  ChartNoAxesCombined,
  ListTree,
  Loader2,
  PanelsTopLeft,
  ChevronDown,
  GripVertical,
  Pin,
  PinOff,
  RefreshCw,
  Settings2,
  X,
} from "lucide-react";
import AgentBot from "../../shared/ui/AgentBot";
import BrowserBot from "../../features/fingerprint-browser/components/BrowserBot";
import HudAgentDetails, { type HudDetailView } from "./HudAgentDetails";
import HudRing from "./HudRing";
import TooltipLayer from "../../shared/ui/TooltipLayer";
import HudProxyStatus, { type HudNetworkDetail } from "./HudProxyStatus";
import {
  agentHud,
  type AgentHudOptions,
  type AgentHudStatus,
  type AgentHudTool,
} from "../../features/ai/services/agentHud";
import {
  botStateForUsage,
  agentBotId,
} from "../../features/ai/services/agentBots";
import {
  quotaRows,
  recentSessions,
  sessionState,
} from "../../features/ai/services/aiStats";
import { readAiUsage } from "../../features/ai/services/aiUsage";
import { api, type AiUsageItem, type AppConfig } from "../../services/api";
import { usePolling } from "../../shared/hooks/usePolling";
import { usePageActive } from "../../shared/hooks/usePageActive";
import { useTauriEvent } from "../../shared/hooks/useTauriEvent";
import { useAppStore } from "../../store";
import { useThemeInit } from "../../theme";
import {
  AI_TOOL_METADATA,
  aiToolCards,
} from "../../features/ai/services/aiToolCards";

const TOOLS = AI_TOOL_METADATA.map((tool) => ({
  key: tool.key,
  label: tool.hudLabel,
  short: tool.short,
}));
const stateLabels = {
  idle: "待机",
  working: "工作中",
  asking: "等待确认",
  done: "已完成",
};
type LaunchNotice = {
  phase: "success" | "error" | "blocked";
  messages: string[];
};

// 尺寸请求只负责几何；其快照可能早于刚保存的设置，不能回写显示范围或开关。
function withLayout(current: AgentHudStatus | null, layout: AgentHudStatus) {
  return current
    ? {
        ...current,
        dockEdge: layout.dockEdge,
        stripOffsetX: layout.stripOffsetX,
        stripOffsetY: layout.stripOffsetY,
        compactWidth: layout.compactWidth,
        sideHeight: layout.sideHeight,
      }
    : layout;
}

export default function AgentHud() {
  useThemeInit();
  const config = useAppStore((state) => state.config);
  const keys = aiToolCards(config).map((tool) => tool.key as string);
  const active = usePageActive();
  const [expanded, setExpanded] = useState(false);
  const [closing, setClosing] = useState(false);
  const [pinned, setPinned] = useState(false);
  const [selected, setSelected] = useState("claude");
  const [detailView, setDetailView] = useState<HudDetailView>("usage");
  const [items, setItems] = useState<AiUsageItem[] | null>(null);
  const [readAt, setReadAt] = useState<number | null>(null);
  const [now, setNow] = useState(() => Date.now() / 1000);
  const [error, setError] = useState("");
  const [usageError, setUsageError] = useState("");
  const [optionsError, setOptionsError] = useState("");
  const failedOptions = useRef<AgentHudOptions | null>(null);
  const [busy, setBusy] = useState(false);
  const [ready, setReady] = useState(false);
  const [networkDetail, setNetworkDetail] = useState<HudNetworkDetail>(null);
  const [networkHost, setNetworkHost] = useState<HTMLDivElement | null>(null);
  const [preferences, setPreferences] = useState<AgentHudStatus | null>(null);
  const [localTools, setLocalTools] = useState<AgentHudTool[] | null>(null);
  const [toolError, setToolError] = useState("");
  const [launchingKey, setLaunchingKey] = useState<string | null>(null);
  const [launchNotices, setLaunchNotices] = useState<
    Record<string, LaunchNotice>
  >({});
  const launchPending = useRef(false);
  const [toolRevision, setToolRevision] = useState(0);
  const [optionsBusy, setOptionsBusy] = useState(false);
  const savingOptions = useRef(false);
  const pending = useRef(0),
    mounted = useRef(false),
    expandedRef = useRef(false);
  const root = useRef<HTMLDivElement>(null);
  const detailsScroll = useRef<HTMLDivElement>(null);
  const collapseTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const resizeQueue = useRef(Promise.resolve());
  const toolCount = useRef(0);
  const hoverBlocked = useRef(false);
  const transitionPending = useRef(false);
  const pinnedRef = useRef(pinned);
  pinnedRef.current = pinned;
  // 悬浮条动效常驻：不再按指针静止进入静止态（原 idle 冻结已移除）。
  // 仅保留窗口/页面不可见（data-paused）与系统「减少动效」两条暂停路径。
  const changeExpanded = useCallback((value: boolean) => {
    clearTimeout(collapseTimer.current);
    if (expandedRef.current === value) return;
    expandedRef.current = value;
    transitionPending.current = true;
    if (value) setClosing(false);
    // 序列化原生尺寸请求，避免快速移入移出时窗口尺寸与 React 状态反向。
    resizeQueue.current = resizeQueue.current
      .catch(() => {})
      .then(async () => {
        if (!mounted.current || expandedRef.current !== value) return;
        try {
          if (
            !value &&
            !window.matchMedia("(prefers-reduced-motion: reduce)").matches
          ) {
            setClosing(true);
            await new Promise((resolve) => setTimeout(resolve, 150));
            if (!mounted.current || expandedRef.current !== value) return;
          }
          const layout = await agentHud.expand(value, toolCount.current);
          if (mounted.current && expandedRef.current === value) {
            if (layout)
              setPreferences((current) => withLayout(current, layout));
            setExpanded(value);
            setClosing(false);
            transitionPending.current = false;
          }
        } catch (error) {
          if (mounted.current && expandedRef.current === value) {
            expandedRef.current = !value;
            transitionPending.current = false;
            setClosing(false);
            setError(String(error));
          }
        }
      });
  }, []);
  useEffect(() => {
    if (!expanded) setNetworkDetail(null);
  }, [expanded]);
  useEffect(() => {
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      hoverBlocked.current = true;
      setPinned(false);
      changeExpanded(false);
      (document.activeElement as HTMLElement)?.blur();
    };
    document.addEventListener("keydown", escape);
    return () => document.removeEventListener("keydown", escape);
  }, [changeExpanded]);
  // 原生 ready 后可能立刻推送状态：两个事件监听与窗口移动监听都就绪再握手。
  const registered = useRef(0);
  const readyCalled = useRef(false);
  const markRegistered = () => {
    registered.current += 1;
    if (registered.current < 3 || readyCalled.current) return;
    readyCalled.current = true;
    void agentHud
      .ready()
      .then((status) => {
        if (!mounted.current) return;
        setPreferences(status);
        useAppStore.setState({ windowVisible: status.enabled });
        setReady(true);
      })
      .catch((error) => {
        if (mounted.current) setError(String(error));
      });
  };
  useTauriEvent<boolean>(
    "agent-hud-visibility",
    (payload) => {
      useAppStore.setState({ windowVisible: payload });
      if (!payload) {
        setPinned(false);
        setExpanded(false);
        setClosing(false);
        expandedRef.current = false;
        transitionPending.current = false;
      }
    },
    true,
    markRegistered,
  );
  useTauriEvent<AgentHudStatus>(
    "agent-hud-changed",
    (payload) => {
      setPreferences(payload);
      useAppStore.setState({ windowVisible: payload.enabled });
      // 保存范围也会广播原生快照；展开尚在途时不能用旧 expanded 撤销用户操作。
      if (!payload.enabled || !transitionPending.current) {
        setExpanded(payload.expanded);
        expandedRef.current = payload.expanded;
      }
      if (
        !payload.enabled ||
        (!transitionPending.current && !payload.expanded)
      ) {
        setPinned(false);
        setClosing(false);
      }
    },
    true,
    markRegistered,
  );
  // 事件订阅交给上面的 hook；这里只保留窗口移动吸附与首次 ready 握手。
  useEffect(() => {
    mounted.current = true;
    let active = true;
    let moveTimer: ReturnType<typeof setTimeout> | undefined;
    const snapAfterMove = async () => {
      if (!active) return;
      try {
        const snapped = await agentHud.snap();
        if (active && snapped === false)
          moveTimer = setTimeout(() => void snapAfterMove(), 150);
      } catch (error) {
        if (active) setError(String(error));
      }
    };
    const move = getCurrentWindow().onMoved(() => {
      clearTimeout(moveTimer);
      moveTimer = setTimeout(() => void snapAfterMove(), 400);
    });
    void move
      .then(() => markRegistered())
      .catch((error) => {
        if (mounted.current) setError(String(error));
      });
    return () => {
      active = false;
      mounted.current = false;
      clearTimeout(moveTimer);
      clearTimeout(collapseTimer.current);
      void move.then((unlisten) => unlisten()).catch(console.error);
    };
  }, []);
  const refresh = useCallback(
    async (isCurrent: () => boolean = () => true, force = false) => {
      pending.current++;
      setBusy(true);
      setNow(Date.now() / 1000);
      try {
        const result = await readAiUsage(force);
        if (mounted.current && isCurrent()) {
          setItems(result.items);
          setNow(Date.now() / 1000);
          setReadAt(Date.now());
          setUsageError("");
        }
      } catch (error) {
        if (mounted.current && isCurrent())
          setUsageError(`统计暂不可用：${String(error)}`);
      } finally {
        pending.current--;
        if (mounted.current && pending.current === 0) setBusy(false);
      }
    },
    [],
  );
  usePolling(
    (current) => refresh(current),
    30_000,
    true,
    ready && !!config,
    keys.join(","),
  );
  usePolling(
    async (isCurrent) => {
      try {
        const tools = await agentHud.tools();
        if (isCurrent()) {
          setLocalTools(tools);
          setToolError("");
        }
      } catch (error) {
        if (isCurrent()) {
          setToolError(`本机工具识别暂不可用：${String(error)}`);
        }
      }
    },
    10_000,
    true,
    ready,
    toolRevision,
  );
  useTauriEvent<AppConfig>(
    "config-changed",
    (payload) => {
      if (Array.isArray(payload?.ai_tool_cards))
        useAppStore.getState().updateConfig(payload);
      else void useAppStore.getState().refreshConfig().catch(console.error);
      setToolRevision((value) => value + 1);
    },
    active && ready,
  );
  useEffect(() => {
    if (!active || !ready) return;
    void useAppStore.getState().refreshConfig().catch(console.error);
  }, [active, ready]);
  const scope = preferences?.toolScope ?? "installed";
  // 已添加名单来自配置，进程识别只决定在线筛选；扫描失败仍可查看和启动已添加项。
  const visibleTools = TOOLS.filter((tool) => keys.includes(tool.key)).filter(
    (tool) =>
      scope !== "online" ||
      localTools?.some((local) => local.key === tool.key && local.online),
  );
  useEffect(() => {
    toolCount.current = visibleTools.length;
    if (!ready) return;
    // 图标数量改变也同步原生窗口；仅缩小 CSS 会留下拦截鼠标的透明区域。
    resizeQueue.current = resizeQueue.current
      .catch(() => {})
      .then(async () => {
        if (!mounted.current) return;
        try {
          const layout = await agentHud.expand(
            expandedRef.current,
            toolCount.current,
          );
          if (mounted.current && layout)
            setPreferences((current) => withLayout(current, layout));
        } catch (error) {
          if (mounted.current) setError(String(error));
        }
      });
  }, [ready, visibleTools.length]);
  const tool =
    visibleTools.find((tool) => tool.key === selected) ?? visibleTools[0];
  const selectedKey = tool?.key;
  useEffect(() => {
    if (selectedKey) setSelected(selectedKey);
  }, [selectedKey]);
  const updateOptions = async (options: AgentHudOptions) => {
    if (savingOptions.current) return;
    savingOptions.current = true;
    setOptionsBusy(true);
    setOptionsError("");
    try {
      const status = await agentHud.options(options);
      if (mounted.current) {
        setPreferences(status);
        setOptionsError("");
        failedOptions.current = null;
      }
    } catch (error) {
      if (mounted.current) {
        failedOptions.current = options;
        setOptionsError(`保存悬浮条设置失败：${String(error)}`);
      }
    } finally {
      savingOptions.current = false;
      if (mounted.current) setOptionsBusy(false);
    }
  };
  const current = items?.find((item) => item.key === selectedKey);
  const state = botStateForUsage(current, now);
  const sessions = recentSessions(current ? [current] : [], now);
  const running = sessions.filter(
    (session) => sessionState(session, now) === "running",
  ).length;
  const plan =
    current?.account.planLabel ||
    current?.account.plan ||
    current?.rate.metrics?.find((metric) => metric.plan)?.plan;
  const closeLater = () => {
    if (pinned) return;
    clearTimeout(collapseTimer.current);
    collapseTimer.current = setTimeout(() => {
      if (
        !pinnedRef.current &&
        !root.current?.querySelector(".agent-hud-card")?.matches(":hover") &&
        !root.current?.querySelector(":focus-visible") &&
        !document.querySelector(".app-tooltip:hover")
      )
        changeExpanded(false);
    }, 280);
  };
  const perform = (action: () => Promise<unknown>) => {
    void action().catch((error) => {
      if (mounted.current) setError(String(error));
    });
  };
  const launch = async (tool: (typeof TOOLS)[number]) => {
    // ref 在同一事件循环内生效，避免 React 更新前重复提交启动请求。
    if (launchPending.current) return;
    launchPending.current = true;
    detailsScroll.current?.scrollTo(0, 0);
    setLaunchingKey(tool.key);
    setSelected(tool.key);
    setNetworkDetail(null);
    changeExpanded(true);
    setLaunchNotices((notices) => {
      const next = { ...notices };
      delete next[tool.key];
      return next;
    });
    try {
      const result = await api.launchAi(tool.key);
      if (!mounted.current) return;
      setLaunchNotices((notices) => ({
        ...notices,
        [tool.key]: result.ok
          ? {
              phase: "success",
              messages: [`${tool.label} 启动请求已发送，按所选运行方案打开。`],
            }
          : {
              phase: "blocked",
              messages: result.failures.length
                ? result.failures
                : ["环境校验未通过，请重新检测后启动。"],
            },
      }));
      if (result.ok) setToolRevision((value) => value + 1);
    } catch (reason) {
      if (mounted.current)
        setLaunchNotices((notices) => ({
          ...notices,
          [tool.key]: { phase: "error", messages: [String(reason)] },
        }));
    } finally {
      launchPending.current = false;
      if (mounted.current) setLaunchingKey(null);
    }
  };
  const launchNotice = selectedKey ? launchNotices[selectedKey] : undefined;
  const dock = preferences?.dockEdge ?? "top";

  return (
    <>
      <div
        ref={root}
        data-paused={!active}
        data-dock={dock}
        data-tooltip-side={
          dock === "left"
            ? "right"
            : dock === "right"
              ? "left"
              : dock === "free" && (preferences?.stripOffsetY ?? 0) > 0
                ? "top"
                : "bottom"
        }
        data-panel-up={dock === "free" && (preferences?.stripOffsetY ?? 0) > 0}
        className={`agent-hud-viewport ${expanded ? "is-expanded" : ""} ${closing ? "is-closing" : ""}`}
        style={
          {
            "--hud-strip-width": preferences
              ? `${preferences.compactWidth - 12}px`
              : undefined,
            "--hud-side-height": preferences
              ? `${preferences.sideHeight - 12}px`
              : undefined,
            "--hud-strip-x": `${preferences?.stripOffsetX ?? 0}px`,
            "--hud-strip-y": `${preferences?.stripOffsetY ?? 0}px`,
          } as CSSProperties
        }
        onMouseEnter={() => {
          if (!hoverBlocked.current) changeExpanded(true);
        }}
        onMouseLeave={closeLater}
        onMouseMove={(event) => {
          // 悬停后指针真正移动：解除 Esc 造成的展开压制。
          if (hoverBlocked.current && (event.movementX || event.movementY)) {
            hoverBlocked.current = false;
            changeExpanded(true);
          }
        }}
        onFocusCapture={() => {
          hoverBlocked.current = false;
          changeExpanded(true);
        }}
        onBlurCapture={closeLater}
      >
        <main
          className={`agent-hud-card ${expanded && networkDetail ? "has-network-detail" : ""}`}
          aria-label="Agent 动态悬浮条"
          onMouseLeave={closeLater}
        >
          <div className="agent-hud-strip">
            <button
              className="hud-drag hud-icon"
              aria-label="拖动悬浮条"
              data-tooltip="拖动悬浮条"
              data-tooltip-detail="拖动调整位置，靠近屏幕边缘自动停靠。"
              onMouseEnter={() => setNetworkDetail(null)}
              onFocus={() => setNetworkDetail(null)}
              onPointerDown={(event) => {
                if (event.button === 0) {
                  event.preventDefault();
                  clearTimeout(collapseTimer.current);
                  perform(() => getCurrentWindow().startDragging());
                }
              }}
            >
              <GripVertical size={15} />
            </button>
            <div
              className="hud-agents"
              aria-label={scope === "online" ? "当前在线工具" : "已添加工具"}
              onMouseMove={(event) => {
                // 展开导致图标移到静止指针下方时，保留用户刚打开的网络详情。
                if (event.movementX || event.movementY) setNetworkDetail(null);
              }}
              onFocusCapture={() => setNetworkDetail(null)}
            >
              {visibleTools.map((tool) => {
                const online =
                  localTools?.some(
                    (local) => local.key === tool.key && local.online,
                  ) ?? false;
                const item = items?.find((item) => item.key === tool.key),
                  state = botStateForUsage(item, now);
                const quota = quotaRows(item, now)
                  .filter(
                    (view) =>
                      !view.stale && !view.resetPassed && view.used != null,
                  )
                  .sort((a, b) => b.used! - a.used!)[0];
                const label = item?.recentSessions?.length
                  ? stateLabels[state]
                  : online
                    ? "在线 · 暂无会话事件"
                    : "已添加 · 未运行";
                return (
                  <button
                    key={tool.key}
                    className={`hud-agent ${expanded && selectedKey === tool.key ? "is-selected" : ""} ${state === "working" ? "is-working" : ""}`}
                    data-tool={tool.key}
                    data-state={state}
                    data-online={online}
                    aria-label={`${tool.label} · ${label}`}
                    aria-pressed={expanded && selectedKey === tool.key}
                    aria-busy={launchingKey === tool.key}
                    aria-disabled={launchingKey != null}
                    data-tooltip-off="true"
                    onMouseEnter={() => {
                      setSelected(tool.key);
                      setNetworkDetail(null);
                      if (!hoverBlocked.current) changeExpanded(true);
                    }}
                    onFocus={() => {
                      setSelected(tool.key);
                      setNetworkDetail(null);
                      changeExpanded(true);
                    }}
                    onClick={(event) => {
                      if (event.detail < 2) void launch(tool);
                    }}
                  >
                    <HudRing
                      className={`hud-agent-ring ${quota?.tone ?? "muted"}`}
                      value={quota?.used ?? null}
                      showTrack={quota != null}
                    >
                      <AgentBot
                        bot={agentBotId(tool.key)}
                        state={state}
                        size={42}
                        loopIdle
                      />
                    </HudRing>
                    {launchingKey === tool.key && (
                      <span className="hud-launch-spinner" aria-hidden="true">
                        <Loader2 size={12} className="animate-spin" />
                      </span>
                    )}
                    <i className={`${state} ${online ? "is-online" : ""}`} />
                  </button>
                );
              })}
              {visibleTools.length === 0 && (
                <span className="hud-empty-tools">
                  {toolError
                    ? "工具识别失败"
                    : !localTools
                      ? "正在识别工具…"
                      : scope === "online"
                        ? "暂无在线工具"
                        : "未检测到本机工具"}
                </span>
              )}
            </div>
            <div
              className="hud-environment"
              role="group"
              aria-label="浏览器与代理"
            >
              <button
                className="hud-browser"
                aria-label="打开指纹浏览器"
                data-tooltip="指纹浏览器"
                data-tooltip-detail="点击打开独立资料与浏览器环境设置。"
                onMouseEnter={() => setNetworkDetail(null)}
                onFocus={() => setNetworkDetail(null)}
                onClick={() => perform(() => agentHud.open("environment"))}
              >
                <HudRing value={null} showTrack={false}>
                  <BrowserBot size={42} animated={ready && active} />
                </HudRing>
              </button>
              <HudProxyStatus
                ready={ready}
                expanded={expanded}
                detail={networkDetail}
                onDetailChange={setNetworkDetail}
                detailsHost={networkHost}
              />
            </div>
            <button
              className="hud-icon hud-expand"
              aria-label={expanded ? "收起悬浮条" : "展开悬浮条"}
              aria-expanded={expanded}
              onMouseEnter={() => setNetworkDetail(null)}
              onFocus={() => setNetworkDetail(null)}
              onClick={() => {
                setPinned(!expanded);
                changeExpanded(!expanded);
              }}
            >
              <ChevronDown size={15} />
            </button>
          </div>
          {expanded && (
            <div
              ref={setNetworkHost}
              className="hud-network-details"
              hidden={!networkDetail}
            />
          )}
          {expanded && (
            <section
              className="hud-details"
              style={
                {
                  "--hud-agent-accent":
                    (
                      {
                        claude: "#dfa080",
                        chatgpt: "#76b895",
                        cursor: "#abb0ba",
                        antigravity: "#8baeff",
                        deepseek: "#799bec",
                        glm: "#a1c7bd",
                        kimi: "#a9b4c1",
                        grok: "#b0bec5",
                        opencode: "#b4a9c8",
                      } as Record<string, string>
                    )[selectedKey ?? ""] ?? "#76b895",
                } as CSSProperties
              }
              hidden={networkDetail != null}
              aria-label="Agent 用量与会话"
            >
              <div className="hud-display-options">
                <div role="group" aria-label="悬浮条显示范围">
                  {(
                    [
                      ["online", "当前在线"],
                      ["installed", "已添加工具"],
                    ] as const
                  ).map(([value, label]) => (
                    <button
                      key={value}
                      disabled={optionsBusy || !preferences}
                      aria-pressed={scope === value}
                      onClick={() => void updateOptions({ toolScope: value })}
                    >
                      {label}
                    </button>
                  ))}
                </div>
                <button
                  className="hud-dock-toggle"
                  role="switch"
                  aria-label="悬浮条自动吸边"
                  aria-checked={preferences?.autoDockTop ?? true}
                  disabled={optionsBusy || !preferences}
                  data-tooltip="自动吸边"
                  data-tooltip-detail="拖到屏幕顶部、左侧或右侧后自动停靠，详情向内展开。"
                  onClick={() =>
                    void updateOptions({
                      autoDockTop: !(preferences?.autoDockTop ?? true),
                    })
                  }
                >
                  <PanelsTopLeft size={12} />
                  自动吸边
                </button>
              </div>
              <header className="hud-details-header">
                <div className="hud-current-agent">
                  <div>
                    <h2 title={tool?.label}>{tool?.label ?? "AI 详情"}</h2>
                    {plan && (
                      <span className="hud-plan" title={plan}>
                        {plan}
                      </span>
                    )}
                  </div>
                  <p>
                    {running > 0 ? (
                      <>
                        <i className="hud-live-dot" />
                        {running} 个会话工作中
                      </>
                    ) : current?.recentSessions?.length ? (
                      stateLabels[state]
                    ) : items ? (
                      "暂无本地数据"
                    ) : (
                      "正在读取统计…"
                    )}
                  </p>
                </div>
                <div className="hud-actions">
                  <div
                    className="hud-view-switch"
                    role="tablist"
                    aria-label="AI 详情视图"
                  >
                    <button
                      className="hud-icon"
                      role="tab"
                      aria-label="额度与用量"
                      aria-selected={detailView === "usage"}
                      data-tooltip="额度与用量"
                      onClick={() => setDetailView("usage")}
                    >
                      <ChartNoAxesCombined size={15} />
                    </button>
                    <button
                      className="hud-icon"
                      role="tab"
                      aria-label="会话详情"
                      aria-selected={detailView === "sessions"}
                      data-tooltip="会话详情"
                      onClick={() => setDetailView("sessions")}
                    >
                      <ListTree size={15} />
                    </button>
                  </div>
                  <button
                    className="hud-icon"
                    aria-label={pinned ? "取消固定展开" : "固定展开"}
                    aria-pressed={pinned}
                    onClick={() => setPinned(!pinned)}
                  >
                    {pinned ? <PinOff size={14} /> : <Pin size={14} />}
                  </button>
                  <button
                    className="hud-icon"
                    aria-label="关闭悬浮条"
                    data-tooltip="关闭悬浮条"
                    data-tooltip-detail="关闭后可在 AI 统计的悬浮条设置中重新开启。"
                    onClick={() => perform(() => agentHud.enabled(false))}
                  >
                    <X size={15} />
                  </button>
                </div>
              </header>
              <div ref={detailsScroll} className="hud-scroll">
                {launchingKey === selectedKey && (
                  <p className="hud-launch-notice" role="status">
                    正在校验环境并启动 {tool?.label}…
                  </p>
                )}
                {tool && launchNotice && (
                  <div
                    className="hud-launch-notice"
                    data-phase={launchNotice.phase}
                    role={launchNotice.phase === "success" ? "status" : "alert"}
                  >
                    <strong>
                      {launchNotice.phase === "success"
                        ? "启动请求已发送"
                        : launchNotice.phase === "blocked"
                          ? "环境校验未通过"
                          : `${tool.label} 启动失败`}
                    </strong>
                    {launchNotice.messages.map((message, index) => (
                      <p key={index}>{message}</p>
                    ))}
                    {launchNotice.phase !== "success" && (
                      <div>
                        <button
                          disabled={launchingKey != null}
                          onClick={() => void launch(tool)}
                        >
                          重新启动
                        </button>
                        <button
                          onClick={() =>
                            perform(() =>
                              agentHud.open(
                                launchNotice.phase === "blocked"
                                  ? "detect"
                                  : "tools",
                              ),
                            )
                          }
                        >
                          {launchNotice.phase === "blocked"
                            ? "前往环境检测"
                            : "配置启动入口"}
                          <ArrowUpRight size={11} />
                        </button>
                      </div>
                    )}
                  </div>
                )}
                {tool ? (
                  <HudAgentDetails
                    item={current}
                    tool={tool.key}
                    label={tool.label}
                    now={now}
                    view={detailView}
                    onOpen={() => perform(() => agentHud.open("stats"))}
                  />
                ) : (
                  <p className="hud-muted">
                    {scope === "online"
                      ? "当前没有识别到在线工具，可切换到“已添加工具”。"
                      : "在 AI 启动工具中添加卡片后，这里会同步显示。"}
                  </p>
                )}
                {optionsError && (
                  <p role="alert" className="hud-error">
                    {optionsError}
                    <button
                      disabled={optionsBusy}
                      onClick={() => {
                        if (failedOptions.current)
                          void updateOptions(failedOptions.current);
                      }}
                    >
                      重试保存
                    </button>
                  </p>
                )}
                {error && (
                  <p role="alert" className="hud-error">
                    {error}
                  </p>
                )}
                {usageError && (
                  <p role="alert" className="hud-error">
                    {usageError} {items ? "保留上次读数。" : ""}
                    <button onClick={() => void refresh(undefined, true)}>
                      重试
                    </button>
                  </p>
                )}
                {toolError && (
                  <p role="alert" className="hud-error">
                    {toolError}
                    <button
                      onClick={() => setToolRevision((value) => value + 1)}
                    >
                      重新识别
                    </button>
                  </p>
                )}
              </div>
              <footer className="hud-footer">
                <span>
                  {readAt
                    ? `${new Date(readAt).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", hour12: false })} 更新`
                    : "等待统计"}
                </span>
                <div>
                  <button
                    className="hud-icon"
                    aria-label="刷新悬浮条统计"
                    disabled={busy}
                    onClick={() => void refresh(undefined, true)}
                  >
                    <RefreshCw
                      size={13}
                      className={busy ? "animate-spin" : ""}
                    />
                  </button>
                  <button
                    aria-label="打开悬浮条设置"
                    onClick={() => perform(() => agentHud.open("settings"))}
                  >
                    <Settings2 size={13} />
                  </button>
                  <button onClick={() => perform(() => agentHud.open("stats"))}>
                    查看统计
                    <ArrowUpRight size={13} />
                  </button>
                </div>
              </footer>
            </section>
          )}
        </main>
      </div>
      <TooltipLayer
        onHoverChange={(hovered) => {
          if (hovered) clearTimeout(collapseTimer.current);
          else closeLater();
        }}
      />
    </>
  );
}
