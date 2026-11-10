import { useCallback, useEffect, useRef, useState } from "react";
import {
  Gauge,
  Activity,
  ArrowDown,
  ArrowUp,
  Share2,
  Wifi,
  MonitorSmartphone,
  Globe,
  GitBranch,
  LayoutDashboard,
  Loader2,
  RefreshCw,
  RotateCw,
} from "lucide-react";
import { api, type IpInfo, type ProxyPortUsage } from "../../../services/api";
import NetworkModeControls from "../components/NetworkModeControls";
import PortConflictDialog from "../components/PortConflictDialog";
import ProxyRuntimeStatus from "../components/ProxyRuntimeStatus";
import ProxyTabs from "../components/ProxyTabs";
import { useIpv6ToggleGuard } from "../components/useIpv6ToggleGuard";
import { useProxyStore } from "../state";
import { proxyConfiguration } from "../configuration";
import { usePolling } from "../../../shared/hooks/usePolling";
import { subscribeTraffic } from "../services/streams";
import { formatTrafficBytes } from "../services/subscriptionTraffic";
import { usePageActive } from "../../../shared/hooks/usePageActive";
import { useNavigate } from "react-router-dom";
import PageHeader from "../../../shared/ui/PageHeader";

interface TrafficSample {
  up: number;
  down: number;
}

/** 流量卡沿用两位小数（GB）；其它档位与订阅卡共用同一实现。 */
const fmtBytes = (n: number) => formatTrafficBytes(n, 2);

const MAX_POINTS = 60;

export default function Overview() {
  const config = useProxyStore((s) => s.config);
  const refreshCore = useProxyStore((s) => s.refreshCore);
  const coreStatus = useProxyStore((s) => s.coreStatus);
  const coreError = useProxyStore((s) => s.coreError);
  const coreRunning = coreStatus?.running === true;
  const coreTransitioning =
    coreStatus?.state === "starting" ||
    coreStatus?.state === "stopping" ||
    coreStatus?.state === "reconfiguring";
  const managed = !!config?.network_mode;
  const active = usePageActive();
  const navigate = useNavigate();
  const [portConflict, setPortConflict] = useState<ProxyPortUsage | null>(null);
  const promptedPort = useRef<string | null>(null);

  // 进入代理页时核对一次端口占用：占用者是自身内核则正常，其他程序则弹窗提示。
  useEffect(() => {
    let cancelled = false;
    api
      .proxyPortUsage()
      .then((usage) => {
        if (cancelled || !usage.occupied || usage.ownedByCore) return;
        const key = `${usage.port}:${usage.pid ?? 0}`;
        if (promptedPort.current === key) return;
        promptedPort.current = key;
        setPortConflict(usage);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

  const [samples, setSamples] = useState<TrafficSample[]>([]);
  const samplesRef = useRef(samples);
  const redraw = useRef<() => void>(() => {});
  samplesRef.current = samples;
  const [wsLive, setWsLive] = useState(false);
  const [totals, setTotals] = useState<{ up: number; down: number } | null>(
    null,
  );
  const [ipInfo, setIpInfo] = useState<IpInfo | null>(null);
  const [ipLoading, setIpLoading] = useState(false);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  // 系统代理以注册表真实状态为准（防止 UI 与系统脱节）
  const sysReal = coreStatus?.system_proxy ?? null;
  const tunReal = coreStatus?.tun ?? null;
  // 开关先动、内核后动：重建会话要 1–3 秒，不能让旋钮一直停在旧状态。
  const [tunOptimistic, setTunOptimistic] = useState<boolean | null>(null);
  const tunOn = tunOptimistic ?? tunReal;
  const [sysError, setSysError] = useState<string | null>(null);
  const [tunError, setTunError] = useState<string | null>(null);
  const [ipv6Error, setIpv6Error] = useState<string | null>(null);
  const ipv6Guard = useIpv6ToggleGuard();
  const switchRunning = useRef(false);
  const ipRequest = useRef(0);
  const [pendingSwitch, setPendingSwitch] = useState<
    "sys" | "tun" | "ipv6" | null
  >(null);
  const [restarting, setRestarting] = useState(false);
  const [restartError, setRestartError] = useState<string | null>(null);
  const [restartMessage, setRestartMessage] = useState<string | null>(null);
  const controlsBusy =
    pendingSwitch !== null || restarting || coreTransitioning;

  const restartCore = async () => {
    if (switchRunning.current || !config || !coreStatus || coreTransitioning)
      return;
    switchRunning.current = true;
    setRestarting(true);
    setRestartError(null);
    setRestartMessage(null);
    try {
      await proxyConfiguration.restartCore();
      setRestartMessage("内核已重启");
    } catch (error) {
      // 详情由后端写进右上角消息中心，这里只留一行结论，避免长文案撑开页面。
      setRestartError("重启内核失败，详情见右上角消息中心");
      void useProxyStore
        .getState()
        .refreshErrors()
        .catch(() => undefined);
      void error;
    } finally {
      switchRunning.current = false;
      setRestarting(false);
    }
  };

  const toggleSys = async () => {
    if (switchRunning.current || managed || !config || sysReal === null) return;
    switchRunning.current = true;
    setPendingSwitch("sys");
    setSysError(null);
    const next = !sysReal;
    try {
      await proxyConfiguration.setSystemProxy(next);
    } catch (e) {
      setSysError(String(e));
    } finally {
      switchRunning.current = false;
      setPendingSwitch(null);
    }
  };

  const toggleTun = async () => {
    if (switchRunning.current || managed || !config || tunReal === null) return;
    switchRunning.current = true;
    setPendingSwitch("tun");
    setTunError(null);
    const next = !tunReal;
    // 先按意图翻转，命令返回后用权威状态覆盖；失败则回退。
    setTunOptimistic(next);
    try {
      await proxyConfiguration.setTun(next);
    } catch (e) {
      setTunError(String(e));
    } finally {
      switchRunning.current = false;
      setPendingSwitch(null);
      setTunOptimistic(null);
    }
  };

  const applyIpv6 = async (next: boolean) => {
    switchRunning.current = true;
    setPendingSwitch("ipv6");
    setIpv6Error(null);
    try {
      await proxyConfiguration.patch({ ipv6: next });
    } catch (e) {
      setIpv6Error(`IPv6 切换失败：${String(e)}`);
    } finally {
      switchRunning.current = false;
      setPendingSwitch(null);
    }
  };

  // 默认关闭；关闭一次点击生效，开启要两次点击并在说明框里确认。
  const toggleIpv6 = async () => {
    if (switchRunning.current || managed || !config || !coreStatus) return;
    if (config.ipv6) {
      ipv6Guard.reset();
      await applyIpv6(false);
      return;
    }
    if (!ipv6Guard.armed) {
      ipv6Guard.arm();
      return;
    }
    if (!(await ipv6Guard.confirmEnable())) return;
    await applyIpv6(true);
  };

  useEffect(() => {
    if (!config || !active || !coreRunning) {
      setWsLive(false);
      return;
    }
    return subscribeTraffic(config, {
      onConnected: setWsLive,
      onValue: (sample) =>
        setSamples((previous) => [
          ...previous.slice(-(MAX_POINTS - 1)),
          sample,
        ]),
    });
  }, [config?.external_controller_port, config?.secret, active, coreRunning]);

  useEffect(() => {
    setSamples([]);
    setTotals(null);
  }, [coreRunning, coreStatus?.started_at]);

  // 原生侧只返回累计值，避免把完整连接列表传过 IPC。
  usePolling(
    async (isCurrent) => {
      try {
        const d = await api.connectionTotals();
        if (isCurrent())
          setTotals({ up: d.uploadTotal ?? 0, down: d.downloadTotal ?? 0 });
      } catch {
        /* 内核不在线时忽略 */
      }
    },
    2000,
    true,
    coreRunning,
    coreStatus?.started_at ?? 0,
  );

  // IP 信息
  const loadIp = useCallback(async () => {
    const request = ++ipRequest.current;
    setIpLoading(true);
    try {
      const result = await api.getIpInfo();
      if (request === ipRequest.current) setIpInfo(result);
    } catch {
      if (request === ipRequest.current) setIpInfo(null);
    } finally {
      if (request === ipRequest.current) setIpLoading(false);
    }
  }, []);

  useEffect(() => {
    // 网络变化后丢弃旧出口信息及未完成的旧请求。
    setIpInfo(null);
    if (!active || !coreRunning) {
      setIpLoading(false);
      return;
    }
    void loadIp();
    return () => {
      ++ipRequest.current;
    };
  }, [
    active,
    coreRunning,
    loadIp,
    config?.mode,
    config?.network_mode,
    coreStatus?.started_at,
    coreStatus?.tun,
  ]);

  // 平滑连接真实采样点，控制点限制在相邻值之间，避免曲线超出实际峰值。
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const draw = () => {
      const w = canvas.clientWidth;
      const h = canvas.clientHeight;
      if (!w || !h) return;
      const dpr = window.devicePixelRatio || 1;
      const width = Math.round(w * dpr);
      const height = Math.round(h * dpr);
      if (canvas.width !== width) canvas.width = width;
      if (canvas.height !== height) canvas.height = height;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);

      const styles = getComputedStyle(canvas);
      const colorUp = styles.getPropertyValue("--speed-up").trim() || "#d58a57";
      const colorDown =
        styles.getPropertyValue("--speed-down").trim() || "#5798b3";
      const grid =
        styles.getPropertyValue("--speed-grid").trim() ||
        "rgba(128,128,128,.12)";
      const pad = 8;
      const baseline = h - pad;
      ctx.strokeStyle = grid;
      ctx.lineWidth = 0.7;
      ctx.setLineDash([3, 5]);
      for (let i = 1; i < 4; i++) {
        const y = pad + ((h - pad * 2) / 4) * i;
        ctx.beginPath();
        ctx.moveTo(pad, y);
        ctx.lineTo(w - pad, y);
        ctx.stroke();
      }
      ctx.setLineDash([]);
      const currentSamples = samplesRef.current;
      if (!currentSamples.length) return;
      const max =
        Math.max(...currentSamples.map((s) => Math.max(s.up, s.down)), 1024) *
        1.15;

      const drawLine = (key: "up" | "down", color: string) => {
        const points = currentSamples.map((sample, i) => ({
          x:
            pad +
            ((MAX_POINTS - currentSamples.length + i) / (MAX_POINTS - 1)) *
              (w - pad * 2),
          y: baseline - (Math.max(0, sample[key]) / max) * (h - pad * 2),
        }));
        const trace = () => {
          ctx.moveTo(points[0].x, points[0].y);
          for (let i = 1; i < points.length; i++) {
            const previous = points[i - 1];
            const point = points[i];
            const middle = (previous.x + point.x) / 2;
            ctx.bezierCurveTo(
              middle,
              previous.y,
              middle,
              point.y,
              point.x,
              point.y,
            );
          }
        };
        const last = points[points.length - 1];
        const fill = ctx.createLinearGradient(0, 0, 0, h);
        fill.addColorStop(0, `${color}32`);
        fill.addColorStop(1, `${color}00`);
        ctx.beginPath();
        trace();
        ctx.lineTo(last.x, baseline);
        ctx.lineTo(points[0].x, baseline);
        ctx.closePath();
        ctx.fillStyle = fill;
        ctx.fill();
        ctx.beginPath();
        trace();
        ctx.strokeStyle = color;
        ctx.lineWidth = 2.2;
        ctx.lineJoin = "round";
        ctx.lineCap = "round";
        ctx.stroke();
        ctx.beginPath();
        ctx.arc(last.x, last.y, 3, 0, Math.PI * 2);
        ctx.fillStyle = color;
        ctx.fill();
      };
      drawLine("down", colorDown);
      drawLine("up", colorUp);
    };
    redraw.current = draw;
    draw();
    const resize = new ResizeObserver(draw);
    resize.observe(canvas);
    const theme = new MutationObserver(draw);
    theme.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-theme"],
    });
    return () => {
      resize.disconnect();
      theme.disconnect();
      redraw.current = () => {};
    };
  }, []);

  useEffect(() => {
    redraw.current();
  }, [samples]);

  const latest = samples[samples.length - 1];
  const upTotal = totals?.up ?? 0;
  const downTotal = totals?.down ?? 0;
  const totalSum = upTotal + downTotal;
  const upPct = totalSum > 0 ? (upTotal / totalSum) * 100 : 0;
  const downPct = totalSum > 0 ? 100 - upPct : 0;

  return (
    <div className="overview-page">
      <PageHeader
        icon={LayoutDashboard}
        title="首页概览"
        badge={coreStatus?.version}
        meta={<ProxyRuntimeStatus />}
        actions={
          <>
            <button
              type="button"
              onClick={() => void restartCore()}
              disabled={controlsBusy || !config || !coreStatus}
              aria-busy={restarting}
              data-tooltip="按当前配置重启内核，现有连接会短暂中断"
              className="proxy-action"
            >
              <RotateCw className={restarting ? "animate-spin" : ""} />
              {restarting ? "重启中…" : "重启内核"}
            </button>
            <button
              type="button"
              disabled={restarting || pendingSwitch !== null}
              onClick={() => {
                void loadIp();
                void refreshCore(true).catch(() => undefined);
              }}
              className="proxy-action"
            >
              <RefreshCw /> 刷新
            </button>
          </>
        }
        tabs={<ProxyTabs />}
        note={
          restartMessage ? (
            <span className="is-ready" role="status">
              {restartMessage}
            </span>
          ) : undefined
        }
      />

      {(restartError || coreError || coreStatus?.last_error) && (
        <p className="text-xs text-red-400" role="alert">
          {restartError || coreError || coreStatus?.last_error}
        </p>
      )}
      {/* 第一排：速度 + 开关 */}
      <div className="overview-grid overview-top-grid">
        <section
          className="overview-card overview-speed-card"
          aria-label="网络速度统计"
        >
          <div className="overview-speed-summary">
            <div className="overview-card-heading">
              <h2>
                <span className="overview-card-icon">
                  <Activity size={16} />
                </span>
                网络速度
              </h2>
              <span className="overview-live" data-live={wsLive}>
                <i aria-hidden="true" />
                {wsLive
                  ? "实时更新"
                  : !coreStatus
                    ? "状态未知"
                    : coreRunning
                      ? "连接中…"
                      : "内核未运行"}
              </span>
            </div>
            <div className="overview-speed-readings">
              <div className="overview-speed-reading overview-download">
                <span>
                  <ArrowDown size={14} />
                  下载速度
                </span>
                <strong>
                  {fmtBytes(latest?.down ?? 0)}
                  <small>/s</small>
                </strong>
              </div>
              <div className="overview-speed-reading overview-upload">
                <span>
                  <ArrowUp size={14} />
                  上传速度
                </span>
                <strong>
                  {fmtBytes(latest?.up ?? 0)}
                  <small>/s</small>
                </strong>
              </div>
            </div>
          </div>
          <div className="overview-chart-wrap">
            <span className="overview-chart-tag">近 60 秒</span>
            <canvas
              ref={canvasRef}
              className="overview-chart"
              aria-label="实时上下行速率曲线"
            />
            {!samples.length && (
              <span className="overview-chart-empty">等待流量数据</span>
            )}
          </div>
          <div className="overview-chart-caption">
            <span>较早</span>
            <span>现在</span>
          </div>
        </section>

        <div className="overview-switches">
          <SwitchCard
            icon={<Share2 className="h-4 w-4" />}
            title="系统代理"
            hint={
              managed
                ? "由网络方案管理，先切自定义"
                : `127.0.0.1:${config?.mixed_port ?? 17897}`
            }
            on={sysReal}
            onToggle={toggleSys}
            busyHint={sysError}
            busy={controlsBusy}
            locked={managed || !config}
          />
          <SwitchCard
            icon={<Wifi className="h-4 w-4" />}
            title="虚拟网卡 (TUN)"
            hint={
              managed
                ? "由网络方案管理 · inet6-address 常驻开启"
                : tunOn
                  ? "服务托管 · 全局接管 · inet6-address 常驻开启"
                  : coreStatus?.service_managed
                    ? "服务内核已就绪 · 开启即接管"
                    : "首次开启会自动准备 TUN 服务"
            }
            on={tunOn}
            onToggle={toggleTun}
            pending={tunOptimistic !== null}
            pendingHint={tunOn ? "正在开启 TUN…" : "正在关闭 TUN…"}
            busyHint={tunError}
            busy={controlsBusy}
            locked={managed || !config}
          />
          <SwitchCard
            icon={<Globe className="h-4 w-4" />}
            title="IPv6"
            hint={
              config?.network_mode
                ? "网络方案固定关闭，先切自定义"
                : ipv6Guard.armed
                  ? "再点一次查看说明并确认开启"
                  : "默认关闭 · 开启前会先说明影响"
            }
            on={config ? config.ipv6 : null}
            onToggle={toggleIpv6}
            armed={ipv6Guard.armed}
            busyHint={ipv6Error}
            busy={controlsBusy}
            locked={managed || !config || !coreStatus}
          />
          {ipv6Guard.dialog}
        </div>
      </div>

      {/* 第二排：模式 + 流量统计 */}
      <div className="overview-grid overview-middle-grid">
        <section className="overview-card" aria-label="网络方案与路由">
          <div className="overview-card-heading">
            <h2>
              <span className="overview-card-icon">
                <GitBranch size={16} />
              </span>
              网络方案与路由
            </h2>
          </div>
          <NetworkModeControls />
        </section>

        <section
          className="overview-card overview-traffic-card"
          aria-label="流量统计"
        >
          <div className="overview-card-heading">
            <h2>
              <span className="overview-card-icon">
                <Gauge size={16} />
              </span>
              流量统计
            </h2>
            <span className="overview-traffic-caption">本次会话</span>
          </div>
          <div className="overview-traffic-body">
            <Donut up={upTotal} down={downTotal} total={totalSum} />
            <div className="overview-traffic-legend">
              <div className="overview-traffic-row overview-upload">
                <span>
                  <i aria-hidden="true" />
                  上传累计
                </span>
                <strong>{fmtBytes(upTotal)}</strong>
                <small>{upPct.toFixed(1)}%</small>
              </div>
              <div className="overview-traffic-row overview-download">
                <span>
                  <i aria-hidden="true" />
                  下载累计
                </span>
                <strong>{fmtBytes(downTotal)}</strong>
                <small>{downPct.toFixed(1)}%</small>
              </div>
              <div className="overview-traffic-bar" aria-hidden="true">
                <i
                  className="overview-traffic-bar-up"
                  style={{ flexGrow: upPct }}
                />
                <i
                  className="overview-traffic-bar-down"
                  style={{ flexGrow: downPct }}
                />
              </div>
              <p className="overview-traffic-note">
                上传与下载合计 · 仅统计当前内核会话
              </p>
            </div>
          </div>
        </section>
      </div>

      {/* 第三排：IP */}
      <div className="overview-grid overview-ip-grid">
        <section className="overview-card overview-ip-card">
          <div className="overview-card-heading">
            <h2>
              <span className="overview-card-icon">
                <MonitorSmartphone size={16} />
              </span>
              内网 IP
            </h2>
          </div>
          <div
            className="overview-ip-address"
            data-empty={!ipInfo?.local_ip}
            title={ipInfo?.local_ip ?? undefined}
          >
            {ipInfo?.local_ip ?? "—"}
          </div>
        </section>
        <section className="overview-card overview-ip-card overview-exit-card">
          <div className="overview-card-heading">
            <h2>
              <span className="overview-card-icon">
                <Globe size={16} />
              </span>
              出口 IP
            </h2>
            {ipInfo?.exitCountry && (
              <span className="overview-country">
                {ipInfo.exitCountryName || ipInfo.exitCountry}
              </span>
            )}
          </div>
          <div
            className="overview-ip-address"
            data-empty={!ipInfo?.exitIp}
            title={ipInfo?.exitIp ?? undefined}
          >
            <span>{ipInfo?.exitIp ?? "—"}</span>
            {ipLoading && (
              <Loader2 className="h-3.5 w-3.5 animate-spin text-zinc-500" />
            )}
          </div>
        </section>
      </div>
      {portConflict && (
        <PortConflictDialog
          usage={portConflict}
          onClose={() => setPortConflict(null)}
          onOpenSettings={() => {
            setPortConflict(null);
            navigate("/settings");
          }}
        />
      )}
    </div>
  );
}

function SwitchCard({
  icon,
  title,
  hint,
  on,
  onToggle,
  armed = false,
  pending = false,
  pendingHint,
  busyHint,
  busy,
  locked,
}: {
  icon: React.ReactNode;
  title: string;
  hint: string;
  on: boolean | null;
  onToggle: () => void;
  /** 已进入待确认态（第一次点击之后、说明框之前）。 */
  armed?: boolean;
  /** 已按用户意图先行翻转，等待内核跟上。 */
  pending?: boolean;
  pendingHint?: string;
  busyHint?: string | null;
  busy: boolean;
  locked: boolean;
}) {
  return (
    <div
      className="overview-card overview-switch-card"
      data-error={!!busyHint}
      data-on={on}
      data-armed={armed}
      data-pending={pending}
      data-tooltip={busyHint ?? undefined}
    >
      <div className="overview-switch-copy min-w-0">
        <div className="overview-card-heading">
          <h2>
            <span className="overview-card-icon">{icon}</span>
            {title}
          </h2>
          <span className="overview-switch-state" data-on={on}>
            {on === null ? "状态未知" : on ? "已开启" : "已关闭"}
          </span>
        </div>
        <p
          className="overview-switch-hint"
          role={busyHint ? "alert" : undefined}
        >
          {pending ? (
            <span className="overview-switch-pending">
              <Loader2 className="h-3 w-3 animate-spin" aria-hidden="true" />
              {pendingHint ?? "正在切换…"}
            </span>
          ) : (
            (busyHint ?? hint)
          )}
        </p>
      </div>
      <button
        onClick={onToggle}
        disabled={busy || locked || on === null}
        aria-busy={busy}
        aria-label={`切换${title}`}
        role="switch"
        aria-checked={on ?? false}
        title={
          on === null
            ? "请先刷新代理实际状态"
            : locked
              ? "当前设置暂不可调整"
              : undefined
        }
        className={`relative h-6 w-11 shrink-0 rounded-full transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${
          on ? "bg-emerald-600" : "bg-zinc-700"
        }`}
      >
        <span
          className={`absolute left-0.5 top-0.5 h-5 w-5 rounded-full bg-white shadow transition-transform ${
            on ? "translate-x-5" : "translate-x-0"
          }`}
        />
      </button>
    </div>
  );
}

/** 圆环中心放不下长文本：按量级切换小数位，数字部分最多 5 个字符。 */
function splitTraffic(bytes: number): { value: string; unit: string } {
  if (!Number.isFinite(bytes) || bytes <= 0) return { value: "0", unit: "B" };
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  const step = Math.min(
    units.length - 1,
    Math.max(0, Math.floor(Math.log(bytes) / Math.log(1024))),
  );
  const scaled = bytes / 1024 ** step;
  const digits = step === 0 ? 0 : scaled >= 100 ? 0 : scaled >= 10 ? 1 : 2;
  return { value: scaled.toFixed(digits), unit: units[step] };
}

function Donut({
  up,
  down,
  total,
}: {
  up: number;
  down: number;
  total: number;
}) {
  const R = 44;
  const C = 2 * Math.PI * R;
  const upPct = total > 0 ? Math.max(0, Math.min(100, (up / total) * 100)) : 0;
  const downPct =
    total > 0 ? Math.max(0, Math.min(100, (down / total) * 100)) : 0;
  const upArc = (C * upPct) / 100;
  const downArc = (C * downPct) / 100;
  // 只有一段时用圆头收边；两段都画时用平头，避免两端在接缝处叠色。
  const cap = upArc > 0 && downArc > 0 ? "butt" : "round";
  const { value, unit } = splitTraffic(total);
  return (
    <div className="overview-donut">
      <svg
        viewBox="0 0 120 120"
        role="img"
        aria-label={
          total > 0
            ? `总流量 ${value} ${unit}，上传占比 ${upPct.toFixed(1)}%，下载占比 ${downPct.toFixed(1)}%`
            : "暂无累计流量"
        }
      >
        <circle
          cx="60"
          cy="60"
          r={R}
          fill="none"
          stroke="var(--chart-track)"
          strokeWidth="10"
        />
        {total > 0 && upArc > 0 && (
          <circle
            cx="60"
            cy="60"
            r={R}
            fill="none"
            stroke="var(--speed-up)"
            strokeWidth="10"
            strokeLinecap={cap}
            strokeDasharray={`${upArc} ${C - upArc}`}
            transform="rotate(-90 60 60)"
          />
        )}
        {total > 0 && downArc > 0 && (
          <circle
            cx="60"
            cy="60"
            r={R}
            fill="none"
            stroke="var(--speed-down)"
            strokeWidth="10"
            strokeLinecap={cap}
            strokeDasharray={`${downArc} ${C - downArc}`}
            strokeDashoffset={-upArc}
            transform="rotate(-90 60 60)"
          />
        )}
      </svg>
      <div className="overview-donut-copy" aria-hidden="true">
        <span className="overview-donut-caption">总流量</span>
        <strong>
          {value}
          <small>{unit}</small>
        </strong>
      </div>
    </div>
  );
}
