// Runs in an isolated native WebView2 with bundled production frontend assets.
// `fixtureInternals` is the lexical preview-mock object, never the native bridge.
const options = window.__nativeUiProbeOptions;
const started = performance.now();
const limit = 2048;
const state = {
  ipc: {}, listeners: new Map(), nextListener: 0, sockets: new Set(),
  fixtureErrors: [], fixtureErrorsTotal: 0, runtimeErrors: [], runtimeErrorsTotal: 0,
  consoleErrors: [], consoleErrorsTotal: 0, resourceFailures: 0,
  rafSamples: [], rafCount: 0, rafMaxMs: 0, timerSamples: [], timerCount: 0,
  longTasks: [], longTaskCount: 0, longTaskTotalMs: 0, longTaskMaxMs: 0,
  requestedRoute: options.route, navigationStarted: started, navigationObservedMs: null,
  fixtureTicks: 0,
};
const bounded = (array, value) => { array.push(value); if (array.length > limit) array.shift(); };
const problem = (fixture, kind, value) => {
  const key = fixture ? "fixtureErrors" : "runtimeErrors";
  state[`${key}Total`]++;
  if (state[key].length < 30) state[key].push({ kind, message: String(value).slice(0, 500), route: location.hash });
};
window.addEventListener("error", (event) => {
  if (!event.message && event.target !== window) { state.resourceFailures++; return; }
  problem(String(event.message).includes("fixture:") || String(event.message).includes("预览 mock"), "error", event.message);
});
window.addEventListener("unhandledrejection", (event) => {
  const reason = String(event.reason);
  problem(reason.includes("fixture:") || reason.includes("预览 mock"), "unhandledrejection", reason);
});
const realConsoleError = console.error.bind(console);
console.error = (...args) => {
  state.consoleErrorsTotal++;
  if (state.consoleErrors.length < 30) state.consoleErrors.push(args.map(String).join(" ").slice(0, 500));
  realConsoleError(...args);
};
try {
  new PerformanceObserver((list) => {
    for (const item of list.getEntries()) {
      state.longTaskCount++; state.longTaskTotalMs += item.duration;
      state.longTaskMaxMs = Math.max(state.longTaskMaxMs, item.duration);
      bounded(state.longTasks, item.duration);
    }
  }).observe({ type: "longtask", buffered: true });
} catch { /* Unsupported WebView versions expose no longtask observer. */ }
let lastFrame;
function frame(now) {
  if (lastFrame != null) {
    const delta = now - lastFrame;
    bounded(state.rafSamples, delta); state.rafCount++; state.rafMaxMs = Math.max(state.rafMaxMs, delta);
  }
  lastFrame = now; requestAnimationFrame(frame);
}
requestAnimationFrame(frame);
let lastTimer = performance.now();
setInterval(() => {
  const now = performance.now(); bounded(state.timerSamples, Math.max(0, now - lastTimer - 1000));
  state.timerCount++; lastTimer = now;
}, 1000);

// All commands are fixtures, including plugins. Unknown commands fail visibly.
const originalInvoke = fixtureInternals.invoke;
const now = () => new Date().toISOString();
const members = Array.from({ length: options.agents }, (_, index) => ({
  id: `fixture-member-${index}`, botId: `fixture-bot-${index}`, name: `虚构 Agent ${index + 1}`,
  role: "隔离负载", runtime: "codex", model: "fixture-model", binding: "native", access: "read-only",
  status: "running", sessionId: `fixture-session-${index}`, toolsetIds: [], workspace: "C:/fixture-only",
}));
const room = {
  id: "fixture-room", name: "虚构并发 Agent 负载", workspace: "C:/fixture-only", leadId: members[0]?.id ?? "",
  status: "active", turns: 0, turnLimit: 100, maxMembers: 64, concurrency: options.agents,
  members, messages: [], tasks: [], artifacts: [], attachments: [], updatedAt: now(),
};
const store = {
  rooms: options.agents ? [room] : [], toolsets: [], bots: [], projects: [], attention: [], revision: 0,
  workflow: { inbox: [], requests: [], schedules: [], budgets: [], executions: [] },
};
const emit = (event, payload) => {
  for (const [id, listener] of state.listeners) {
    if (listener.event === event) {
      const message = { event, id, payload };
      if (typeof listener.handler === "function") listener.handler(message);
      else window.__TAURI_INTERNALS__.runCallback(listener.handler, message);
    }
  }
};
window.__nativeUiFixtureInvoke = async (command, args = {}) => {
  state.ipc[command] = (state.ipc[command] || 0) + 1;
  try {
    if (command === "plugin:event|listen") {
      const id = ++state.nextListener; state.listeners.set(id, args); return id;
    }
    if (command === "plugin:event|unlisten") { state.listeners.delete(args.eventId); return; }
    if (command === "plugin:event|emit" || command === "plugin:event|emit_to") { emit(args.event, args.payload); return; }
    if (command === "plugin:window|is_visible") return !options.hidden;
    if (command === "plugin:window|theme") return "dark";
    if (command === "plugin:app|version") return options.version;
    if (command === "record_frontend_fault") {
      if (args.kind === "render") problem(false, "render-boundary", "React ErrorBoundary reported a render failure");
      return;
    }
    if (command === "core_status") return { running: true, adopted: false, version: "fixture-native", started_at: 1000, elevated: false };
    if (command === "privacy_status") return { armed: false, healthy: false, tunnelOn: false, error: null, checks: [], checkedAt: null };
    if (command === "network_mode_backup_status") return { available: false, createdAt: null };
    if (command === "browser_request_status") return { pendingCount: 0, notice: null };
    if (command === "browser_environment_status") return {
      installed: true, slot: "fixture-only", config: { seed: 42, language: "en-US", timezone: "America/New_York" }, measurement: null,
    };
    if (command === "computer_environment_status") return {
      current: { languages: ["en-US"], culture: "en-US", uiLanguage: "en-US", timezone: "Eastern Standard Time" }, backup: null, presets: [],
    };
    if (command === "ai_usage_status") return { items: [] };
    if (command === "ai_monitor_status") return {
      enabled: false, notices: [], summary: { todayTokens: 0, running: 0, scannedAt: null }, deliveryError: null,
    };
    if (command === "ai_control_snapshot") return {
      status: { available: true, running: true, version: "fixture-only" }, plans: { schema: 1, tools: {} },
      data: { models: [{ id: "fixture-model", name: "虚构离线模型" }], providers: [], groups: [] },
      activeSessions: options.agents, engineProtocol: 1,
    };
    if (command === "ai_chat_list") return [];
    if (command === "team_snapshot") return structuredClone({ store, runtimes: [], activeRuns: options.agents, memoryAttention: [] });
    if (command === "team_detect_models") return { models: [], runtime: args.runtime, binding: args.binding, source: "fixture", notice: "虚构模型目录" };
    if (command === "get_ip_info") return { local_ip: "192.0.2.10", exitIp: "198.51.100.10", exitCountry: "US", exitCountryName: "fixture" };
    if (command === "mihomo_request" || command === "native_proxy_request") return args.path === "/proxies"
      ? { proxies: {} } : args.path === "/rules" ? { rules: [] } : args.path === "/connections"
        ? { connections: [], uploadTotal: 0, downloadTotal: 0 } : {};
    if (command === "proxy_nodes") return {};
    if (command === "proxy_rules") return [];
    if (command === "proxy_connections") return { downloaded: 0, uploaded: 0, items: [] };
    if (command === "plugin:window|is_maximized") return false;
    return await originalInvoke(command, args);
  } catch (error) {
    problem(true, "ipc-fixture", `${command}: ${error}`);
    throw new Error(`fixture: ${command}: ${error}`);
  }
};

// Virtual sockets exercise real React rendering without opening any connection.
window.WebSocket = class extends EventTarget {
  static CONNECTING = 0; static OPEN = 1; static CLOSING = 2; static CLOSED = 3;
  readyState = 1;
  constructor(url) {
    super(); this.url = String(url); state.sockets.add(this);
    queueMicrotask(() => this.onopen?.({}));
  }
  close() { if (this.readyState === 3) return; this.readyState = 3; state.sockets.delete(this); this.onclose?.({}); }
  send() {}
};
const realFetch = window.fetch.bind(window);
window.fetch = (input, init) => {
  const url = new URL(typeof input === "string" ? input : input.url, location.href);
  return url.origin === location.origin ? realFetch(input, init) : Promise.reject(new Error("fixture: external fetch blocked"));
};
window.XMLHttpRequest = class { open() { throw new Error("fixture: XMLHttpRequest blocked"); } };
window.RTCPeerConnection = class { constructor() { throw new Error("fixture: RTCPeerConnection blocked"); } };
setInterval(() => {
  state.fixtureTicks++;
  for (const socket of state.sockets) {
    const log = socket.url.includes("/logs");
    socket.onmessage?.({ data: JSON.stringify(log ? { type: "info", payload: `fixture-log-${state.fixtureTicks}`, ts: Date.now() / 1000 }
      : { up: state.fixtureTicks % 100 * 1024, down: state.fixtureTicks % 60 * 2048 }) });
  }
  if (!options.agents) return;
  const member = members[state.fixtureTicks % members.length];
  room.messages.push({ id: `fixture-message-${state.fixtureTicks}`, from: member.id, to: null,
    text: `虚构输出 ${state.fixtureTicks}：仅用于渲染压力测量，无 Agent 进程或模型请求。`, kind: "message", taskId: null, at: now() });
  if (room.messages.length > 300) room.messages.splice(0, room.messages.length - 300);
  room.messageCount = room.messages.length; room.updatedAt = now(); store.revision++;
  emit("team-changed", { revision: store.revision });
}, options.agents ? Math.max(50, Math.floor(1000 / options.agents)) : 1000);

const statistics = (values) => {
  const sorted = values.splice(0).sort((a, b) => a - b);
  return { count: sorted.length, p50: sorted[Math.floor(sorted.length * 0.5)] ?? null,
    p95: sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * 0.95))] ?? null,
    max: sorted.at(-1) ?? null };
};
const expectedText = {
  "/": "准备好开始了吗", "/overview": "网络速度", "/environment": "指纹浏览器",
  "/fingerprint-browser": "指纹浏览器", "/ai": "AI 启动工具", "/workspace/chat": "AI 工作台", "/logs": "内核日志",
};
window.__nativeUiProbe = {
  navigate(route) {
    state.requestedRoute = route; state.navigationStarted = performance.now(); state.navigationObservedMs = null;
    location.hash = route;
  },
  snapshot() {
    const root = document.getElementById("root"), main = document.querySelector("main");
    const text = main?.textContent?.trim() ?? "";
    const expected = expectedText[state.requestedRoute];
    const rendered = !!root?.childElementCount && !!main && text.length > 12 && (!expected || text.includes(expected));
    if (rendered && state.navigationObservedMs == null) state.navigationObservedMs = performance.now() - state.navigationStarted;
    const memory = performance.memory;
    return {
      frontendElapsedMs: performance.now() - started, route: location.hash.slice(1), requestedRoute: state.requestedRoute,
      rendered, rootChildren: root?.childElementCount ?? 0, mainTextLength: text.length,
      errorBoundary: document.body?.textContent?.includes("页面暂时无法显示") ?? false,
      navigationObservedMs: state.navigationObservedMs, documentHidden: document.hidden, visibilityState: document.visibilityState,
      domElements: document.getElementsByTagName("*").length,
      jsHeap: memory ? { usedBytes: memory.usedJSHeapSize, totalBytes: memory.totalJSHeapSize, limitBytes: memory.jsHeapSizeLimit,
        source: "performance.memory (browser estimate)" } : null,
      rafIntervalMs: statistics(state.rafSamples), rafCount: state.rafCount, rafLifetimeMaxMs: state.rafMaxMs,
      timerDelayMs: statistics(state.timerSamples), timerCount: state.timerCount,
      longTaskMs: statistics(state.longTasks), longTaskCount: state.longTaskCount,
      longTaskTotalMs: state.longTaskTotalMs, longTaskLifetimeMaxMs: state.longTaskMaxMs,
      ipcCalls: { ...state.ipc }, eventListeners: state.listeners.size, openFixtureSockets: state.sockets.size,
      fixtureTicks: state.fixtureTicks, boundedFixtureMessages: room.messages.length,
      fixtureErrorsTotal: state.fixtureErrorsTotal, fixtureErrors: state.fixtureErrors,
      runtimeErrorsTotal: state.runtimeErrorsTotal, runtimeErrors: state.runtimeErrors,
      consoleErrorsTotal: state.consoleErrorsTotal, consoleErrors: state.consoleErrors, resourceFailures: state.resourceFailures,
    };
  },
};
location.hash = options.route;
