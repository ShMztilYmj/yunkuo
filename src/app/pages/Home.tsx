import { useState, type ReactNode } from "react";
import { Link } from "react-router-dom";
import {
  AlertTriangle,
  ArrowRight,
  ArrowUpRight,
  Clock3,
  Fingerprint,
  Globe,
  Home as HomeIcon,
  Loader2,
  Network,
  RefreshCw,
  ScanLine,
  Share2,
  ShieldCheck,
  Zap,
} from "lucide-react";
import {
  api,
  type MaintenanceStatus,
  type TzSyncResult,
} from "../../services/api";
import type { BrowserEnvironment } from "../../features/fingerprint-browser/services/browser";
import { browserEnvironment } from "../../features/fingerprint-browser/services/browser";
import { usePolling } from "../../shared/hooks/usePolling";
import { useAppStore } from "../../store";
import { useEnvironmentVerdict } from "../../features/diagnostics/hooks/useEnvironmentVerdict";
import AiSummary from "../../features/ai/components/AiSummary";
import { invalidateAiUsage } from "../../features/ai/services/aiUsage";
import PageHeader from "../../shared/ui/PageHeader";

type Tone = "ready" | "attention" | "pending" | "muted";
type ModuleAccent = "emerald" | "sky" | "violet" | "amber";

const PROXY_MODE: Record<string, string> = {
  global: "全局代理",
  rule: "规则路由",
  direct: "直连",
};

/** 首页只关心「多久以前」，不展开完整时间；跨天后退回日期。 */
function relativeTime(value: number | null | undefined) {
  if (!value) return "";
  const diff = Math.max(0, Date.now() - value);
  if (diff < 60_000) return "刚刚";
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} 分钟前`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)} 小时前`;
  return new Date(value).toLocaleDateString("zh-CN", {
    month: "numeric",
    day: "numeric",
  });
}

/**
 * 首页是工作台，不是宣传页：
 * 先给一条一眼能读完的环境结论，再给 AI 用量和四个真正会去操作的功能入口。
 * 所有状态都取真实 store / IPC，映射关系只在当前文件里维护。
 */
export default function Home() {
  const config = useAppStore((s) => s.config);
  const coreStatus = useAppStore((s) => s.coreStatus);
  const currentProfile = useAppStore((s) => s.currentProfile);
  const detect = useAppStore((s) => s.detect);
  const [tzSyncing, setTzSyncing] = useState(false);
  const [tzResult, setTzResult] = useState<TzSyncResult | null>(null);
  const [browser, setBrowser] = useState<BrowserEnvironment | null>(null);
  const [browserError, setBrowserError] = useState(false);
  const [maintenance, setMaintenance] = useState<MaintenanceStatus | null>(
    null,
  );
  const [maintenanceError, setMaintenanceError] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [refreshTick, setRefreshTick] = useState(0);
  const { pass, failures, scanned, scanning } = useEnvironmentVerdict();

  const native = detect.native;
  const coreRunning = coreStatus?.running === true;
  /* 第二段看的是系统代理有没有真的接管（原生检测读注册表，不用 UI 开关自证）。 */
  const sysproxyOn = native?.sysproxyOn === true;
  const sysproxyTarget = native?.sysproxyHost
    ? `${native.sysproxyHost}${native.sysproxyPort ? `:${native.sysproxyPort}` : ""}`
    : null;
  const tzHealthy = native?.timezoneMatches === true && native.tzIsCn === false;
  const country = native?.exitCountryName ?? native?.exitCountry ?? null;
  const readiness = scanning
    ? "检测中"
    : pass
      ? "环境就绪"
      : scanned
        ? "需要修复"
        : "尚未检测";
  const readinessTone: Tone =
    scanning || !scanned ? "pending" : pass ? "ready" : "attention";
  const metaReadiness = scanning
    ? "检测中"
    : pass
      ? "就绪"
      : scanned
        ? "待修复"
        : "未检测";
  const coreTone: Tone = coreStatus
    ? coreRunning
      ? "ready"
      : "attention"
    : "pending";
  const sysproxyTone: Tone = sysproxyOn
    ? "ready"
    : scanned
      ? "attention"
      : "pending";
  const exitTone: Tone =
    native?.countryAllowed === true
      ? "ready"
      : native
        ? "attention"
        : "pending";
  const tzTone: Tone = tzHealthy
    ? "ready"
    : native?.currentTzIana
      ? "attention"
      : "pending";
  const stageTones = [coreTone, sysproxyTone, exitTone, tzTone];
  const readyStages = stageTones.filter((tone) => tone === "ready").length;

  usePolling(
    async (isCurrent) => {
      try {
        const value = await browserEnvironment.browser();
        if (isCurrent()) {
          setBrowser(value);
          setBrowserError(false);
        }
      } catch {
        if (isCurrent()) setBrowserError(true);
      }
    },
    15_000,
    true,
  );

  usePolling(
    async (isCurrent) => {
      try {
        const value = await api.maintenanceStatus();
        if (isCurrent()) {
          setMaintenance(value);
          setMaintenanceError(false);
        }
      } catch {
        if (isCurrent()) setMaintenanceError(true);
      }
    },
    30_000,
    true,
  );

  /** 页头的「刷新状态」只重读本机快照，不改变代理或触发完整环境检测。 */
  const refreshHome = async () => {
    if (refreshing) return;
    setRefreshing(true);
    try {
      const store = useAppStore.getState();
      const [, , nextMaintenance, nextBrowser] = await Promise.allSettled([
        store.refreshCore(true),
        store.refreshProfile(),
        api.maintenanceStatus(),
        browserEnvironment.browser(),
      ]);
      if (nextMaintenance.status === "fulfilled") {
        setMaintenance(nextMaintenance.value);
        setMaintenanceError(false);
      } else {
        setMaintenanceError(true);
      }
      if (nextBrowser.status === "fulfilled") {
        setBrowser(nextBrowser.value);
        setBrowserError(false);
      } else {
        setBrowserError(true);
      }
      invalidateAiUsage();
      setRefreshTick((tick) => tick + 1);
    } finally {
      setRefreshing(false);
    }
  };

  const syncTz = async () => {
    setTzSyncing(true);
    setTzResult(null);
    try {
      setTzResult(await api.syncTimezone());
    } catch (error) {
      setTzResult({ changed: false, message: String(error), info: null });
    } finally {
      setTzSyncing(false);
    }
  };

  const environmentTitle = scanning
    ? "正在核对访问环境"
    : !scanned
      ? "还没有环境结论"
      : pass
        ? "整条访问链路已就绪"
        : "环境还有待处理项";
  const environmentCopy = scanning
    ? "正在读取代理、出口、时区与系统状态，请稍候。"
    : !scanned
      ? "运行一次环境检测，确认代理链路、出口地区与系统时区一致。"
      : pass
        ? "出口、系统代理与系统时区保持一致，可以继续打开 AI 工具。"
        : failures.length > 1
          ? `${failures[0]}，另有 ${failures.length - 1} 项需要处理。`
          : (failures[0] ?? "按检测页的修复建议逐项处理后再试。");
  const primaryDetectLabel = scanning
    ? "查看检测进度"
    : pass
      ? "查看检测报告"
      : scanned
        ? `处理 ${failures.length} 项待办`
        : "开始环境检测";
  const scanNote = detect.at
    ? `上次检测 ${relativeTime(detect.at)}`
    : "本会话尚未完成检测";
  const footerNote = tzSyncing
    ? "正在按出口 IP 同步时区…"
    : tzResult
      ? tzResult.message
      : "按出口 IP 同步 Windows 系统时区";
  const footerTone = tzResult
    ? tzResult.changed
      ? "ready"
      : "pending"
    : undefined;

  const proxyMode = PROXY_MODE[config?.mode ?? "rule"] ?? "规则路由";
  const proxyStatus = coreStatus
    ? coreRunning
      ? "运行中"
      : "已停止"
    : "读取中";
  const proxyTone: Tone = coreStatus
    ? coreRunning
      ? "ready"
      : "attention"
    : "pending";
  const proxyMeta = coreRunning
    ? `${proxyStatus} · ${proxyMode} · ${
        coreStatus?.tun
          ? "TUN 已开启"
          : coreStatus?.system_proxy
            ? "系统代理已开启"
            : "未接管系统流量"
      }`
    : `${proxyStatus} · 启动后接管系统流量`;

  const detectStatus = scanning
    ? "检测中"
    : pass
      ? "已通过"
      : scanned
        ? "待处理"
        : "未检测";
  const detectMain = scanning
    ? "正在检测…"
    : pass
      ? "环境已通过"
      : scanned
        ? `${failures.length} 项待处理`
        : "尚未检测";
  const detectMeta = `${detectStatus}${
    detect.at ? ` · 上次检测 ${relativeTime(detect.at)}` : ""
  }`;

  const browserStatus = browser
    ? !browser.installed
      ? "引擎缺失"
      : browser.running
        ? "使用中"
        : "已就绪"
    : browserError
      ? "状态未知"
      : "读取中";
  const browserTone: Tone = browser
    ? browser.installed
      ? "ready"
      : "attention"
    : browserError
      ? "muted"
      : "pending";
  const browserMain = browser
    ? browser.installed
      ? `资料 ${browser.slot}`
      : "浏览器引擎未就绪"
    : "正在读取浏览器资料";
  const browserMeta = browser
    ? `${browserStatus} · ${browser.config.language} · ${browser.config.timezone}`
    : browserError
      ? "打开浏览器页查看安装与配置"
      : "独立身份与浏览器资料";

  const setupChecks = maintenance
    ? [
        { label: "导入订阅", done: maintenance.hasProfile },
        {
          label: "代理接管",
          done: maintenance.coreRunning && maintenance.privacyHealthy,
        },
        { label: "环境检测", done: scanned && pass },
        { label: "启动入口", done: maintenance.shortcutConfigured },
      ]
    : [];
  const setupDone = setupChecks.filter((item) => item.done).length;
  const nextSetup = setupChecks.find((item) => !item.done);
  const maintenanceTone: Tone = maintenance
    ? nextSetup
      ? "attention"
      : "ready"
    : maintenanceError
      ? "muted"
      : "pending";
  const maintenanceStatus = !maintenance
    ? maintenanceError
      ? "状态未知"
      : "读取中"
    : nextSetup
      ? "待完善"
      : "已就绪";
  const maintenanceMain = maintenance
    ? `本机准备 ${setupDone}/${setupChecks.length}`
    : "维护与升级";
  const maintenanceMeta = maintenance
    ? nextSetup
      ? `下一步：${nextSetup.label}`
      : "主要配置均已就绪"
    : "配置、验收与升级清单";

  const serviceTone: Tone = coreStatus
    ? coreRunning
      ? "ready"
      : "attention"
    : "pending";
  const coreDetail = coreStatus
    ? [coreStatus.version, coreStatus.service_managed ? "服务托管" : "应用托管"]
        .filter(Boolean)
        .join(" · ")
    : "正在读取内核版本";
  const sysproxyDetail = sysproxyOn
    ? (sysproxyTarget ?? "系统流量已接管")
    : native
      ? "打开代理中心完成接管"
      : "完成检测后确认";
  const exitDetail =
    native?.countryAllowed === false
      ? "当前地区受限"
      : native?.countryAllowed === true
        ? "地区允许访问"
        : native
          ? "地区待确认"
          : "完成检测后确认";
  const tzDetail = tzHealthy
    ? "与出口时区一致"
    : native?.exitTimezone
      ? `出口时区 ${native.exitTimezone}`
      : "完成检测后确认";

  return (
    <div className="home-page">
      <PageHeader
        icon={HomeIcon}
        title="工作台"
        badge={
          maintenance?.appVersion ? `v${maintenance.appVersion}` : undefined
        }
        badgeTooltip="应用版本"
        meta={
          <span className="home-meta">
            <span
              data-tone={
                serviceTone === "ready"
                  ? "ok"
                  : serviceTone === "attention"
                    ? "warn"
                    : undefined
              }
            >
              本地服务
              <strong>
                {coreStatus ? (coreRunning ? "已连接" : "未连接") : "读取中"}
              </strong>
            </span>
            <span
              data-tone={
                readinessTone === "ready"
                  ? "ok"
                  : readinessTone === "attention"
                    ? "warn"
                    : undefined
              }
            >
              环境
              <strong>{metaReadiness}</strong>
            </span>
            <span>
              代理
              <strong>
                {coreStatus ? (coreRunning ? "运行中" : "已停止") : "读取中"}
              </strong>
            </span>
            <span>
              出口
              <strong>{country ?? "未知"}</strong>
            </span>
          </span>
        }
        actions={
          <button
            type="button"
            className="proxy-action"
            disabled={refreshing}
            aria-busy={refreshing}
            aria-label="刷新本机状态"
            onClick={() => void refreshHome()}
          >
            <RefreshCw
              className={refreshing ? "animate-spin" : ""}
              aria-hidden="true"
            />
            {refreshing ? "刷新中…" : "刷新状态"}
          </button>
        }
        note={
          <>
            <span>代理、访问环境、AI 工具与浏览器资料，都在这一页汇总。</span>
            <span>{scanNote}</span>
          </>
        }
      />

      <div className="home-main-grid">
        <section
          className="home-env-panel home-panel"
          data-tone={readinessTone}
          aria-labelledby="home-env-heading"
        >
          <header className="home-panel-head">
            <div className="home-panel-title">
              <span className="home-panel-icon" aria-hidden="true">
                <ShieldCheck size={15} />
              </span>
              <div>
                <h2 id="home-env-heading">访问环境</h2>
                <p>代理链路、出口与系统一致性</p>
              </div>
            </div>
            <span
              className="home-state-pill"
              data-tone={readinessTone}
              role="status"
            >
              <i aria-hidden="true" />
              {readiness}
            </span>
          </header>

          <div className="home-env-body">
            <div className="home-env-verdict">
              <div
                className="home-env-health"
                data-tone={readinessTone}
                role="img"
                aria-label={`环境检查 ${readyStages}/${stageTones.length} 项就绪`}
              >
                <svg viewBox="0 0 128 128" aria-hidden="true">
                  <circle
                    className="home-env-health-track"
                    cx="64"
                    cy="64"
                    r="56"
                  />
                  {stageTones.map((tone, index) => (
                    <circle
                      key={index}
                      className={`home-env-health-seg is-${tone}`}
                      cx="64"
                      cy="64"
                      r="56"
                      pathLength="100"
                      strokeDasharray="22 78"
                      strokeDashoffset={-index * 25}
                    />
                  ))}
                </svg>
                <span className="home-env-health-center">
                  {scanning ? (
                    <Loader2
                      className="home-env-health-icon animate-spin"
                      aria-hidden="true"
                    />
                  ) : (
                    <ShieldCheck
                      className="home-env-health-icon"
                      aria-hidden="true"
                    />
                  )}
                  <strong>
                    {readyStages}
                    <span>/{stageTones.length}</span>
                  </strong>
                  <small>{scanning ? "检查中" : "项就绪"}</small>
                </span>
              </div>
              <div className="home-env-copy">
                <h3>{environmentTitle}</h3>
                <p>{environmentCopy}</p>
                <div className="home-env-tags">
                  <span className="home-env-tag">
                    <Globe size={11} aria-hidden="true" />
                    <strong>{country ?? "出口未知"}</strong>
                  </span>
                  <span
                    className="home-env-tag"
                    data-tooltip={native?.exitIp ?? undefined}
                  >
                    <Network size={11} aria-hidden="true" />
                    <strong>{native?.exitIp ?? "IP 未知"}</strong>
                  </span>
                  <span className="home-env-tag">
                    <ShieldCheck size={11} aria-hidden="true" />
                    <strong>
                      {native?.tunOn
                        ? "TUN 已开启"
                        : native?.sysproxyOn
                          ? "系统代理已开启"
                          : "未接管系统流量"}
                    </strong>
                  </span>
                </div>
              </div>
            </div>

            <div className="home-stage-grid">
              <Link
                to="/overview"
                className="home-stage"
                data-tone={coreTone}
                aria-label={`代理内核：${proxyStatus}，打开代理中心`}
              >
                <span className="home-stage-icon" aria-hidden="true">
                  <Zap size={13} />
                </span>
                <span className="home-stage-copy">
                  <small>代理内核</small>
                  <strong>{coreStatus ? proxyStatus : "正在读取"}</strong>
                  <em className="home-stage-desc">{coreDetail}</em>
                </span>
                <i className="home-stage-dot" aria-hidden="true" />
              </Link>
              <Link
                to="/overview"
                className="home-stage"
                data-tone={sysproxyTone}
                aria-label={`系统代理：${
                  sysproxyOn ? "已开启" : sysproxyDetail
                }，打开代理中心`}
              >
                <span className="home-stage-icon" aria-hidden="true">
                  <Share2 size={13} />
                </span>
                <span className="home-stage-copy">
                  <small>系统代理</small>
                  <strong>
                    {sysproxyOn ? "已开启" : native ? "未开启" : "待检测"}
                  </strong>
                  <em className="home-stage-desc">{sysproxyDetail}</em>
                </span>
                <i className="home-stage-dot" aria-hidden="true" />
              </Link>
              <Link
                to="/detect"
                className="home-stage"
                data-tone={exitTone}
                aria-label={`出口地区：${country ?? "未知"}，打开检测`}
              >
                <span className="home-stage-icon" aria-hidden="true">
                  <Globe size={13} />
                </span>
                <span className="home-stage-copy">
                  <small>出口地区</small>
                  <strong data-tooltip={country ?? undefined}>
                    {country ?? "等待检测"}
                    {native?.countryAllowed === false ? " · 受限" : ""}
                  </strong>
                  <em className="home-stage-desc">{exitDetail}</em>
                </span>
                <i className="home-stage-dot" aria-hidden="true" />
              </Link>
              <Link
                to="/detect"
                className="home-stage"
                data-tone={tzTone}
                aria-label={`系统时区：${
                  native?.currentTzIana ?? "未知"
                }，打开检测`}
              >
                <span className="home-stage-icon" aria-hidden="true">
                  <Clock3 size={13} />
                </span>
                <span className="home-stage-copy">
                  <small>系统时区</small>
                  <strong data-tooltip={native?.currentTzIana ?? undefined}>
                    {native?.currentTzIana ?? "等待检测"}
                  </strong>
                  <em className="home-stage-desc">{tzDetail}</em>
                </span>
                <i className="home-stage-dot" aria-hidden="true" />
              </Link>
            </div>
          </div>

          {scanned && !pass && (
            <div className="home-env-alert">
              <AlertTriangle size={13} aria-hidden="true" />
              <span
                className="home-env-alert-text"
                data-tooltip={failures.join("\n")}
              >
                {failures[0]}
              </span>
              {failures.length > 1 && (
                <span className="home-env-alert-count">
                  +{failures.length - 1}
                </span>
              )}
              <Link to="/detect">
                查看全部
                <ArrowUpRight size={12} aria-hidden="true" />
              </Link>
            </div>
          )}

          <footer className="home-panel-foot">
            <span
              className="home-foot-note"
              data-tone={footerTone}
              role={tzResult ? "status" : undefined}
              data-tooltip={footerNote}
            >
              <Clock3 size={12} aria-hidden="true" />
              <span>{footerNote}</span>
            </span>
            <div className="home-env-actions">
              <button
                type="button"
                className="home-text-button"
                disabled={tzSyncing}
                aria-label="按出口 IP 同步系统时区"
                aria-busy={tzSyncing}
                onClick={() => void syncTz()}
              >
                {tzSyncing ? (
                  <Loader2 size={12} className="animate-spin" />
                ) : (
                  <Clock3 size={12} />
                )}
                {tzSyncing ? "同步中…" : "同步时区"}
              </button>
              <Link
                to="/detect"
                className="home-button home-button-primary home-button-small"
              >
                {primaryDetectLabel}
                <ArrowRight size={13} aria-hidden="true" />
              </Link>
            </div>
          </footer>
        </section>

        <AiSummary refreshToken={refreshTick} />
      </div>

      <section className="home-module-grid" aria-label="工作模块">
        <ModuleCard
          to="/overview"
          icon={<Globe size={17} />}
          accent="emerald"
          title="代理中心"
          tone={proxyTone}
          status={proxyStatus}
          value={currentProfile?.name ?? "尚未导入订阅"}
          meta={proxyMeta}
        />
        <ModuleCard
          to="/detect"
          icon={<ScanLine size={17} />}
          accent="sky"
          title="检测与修复"
          tone={readinessTone}
          status={detectStatus}
          value={detectMain}
          meta={detectMeta}
        />
        <ModuleCard
          to="/fingerprint-browser"
          icon={<Fingerprint size={17} />}
          accent="violet"
          title="指纹浏览器"
          tone={browserTone}
          status={browserStatus}
          value={browserMain}
          meta={browserMeta}
        />
        <ModuleCard
          to="/maintenance"
          icon={<ShieldCheck size={17} />}
          accent="amber"
          title="首次使用与维护"
          tone={maintenanceTone}
          status={maintenanceStatus}
          value={maintenanceMain}
          meta={maintenanceMeta}
        />
      </section>
    </div>
  );
}

function ModuleCard({
  to,
  icon,
  accent,
  title,
  tone,
  status,
  value,
  meta,
}: {
  to: string;
  icon: ReactNode;
  accent: ModuleAccent;
  title: string;
  tone: Tone;
  status: string;
  value: string;
  meta: string;
}) {
  return (
    <Link
      to={to}
      className="home-module-card"
      data-accent={accent}
      aria-label={`${title}：${value}，${status}，查看详情`}
    >
      <span className="home-module-icon" aria-hidden="true">
        {icon}
      </span>
      <span className="home-module-body">
        <span className="home-module-head">
          <strong>{title}</strong>
          <span className="home-module-status" data-tone={tone}>
            <i aria-hidden="true" />
            {status}
          </span>
          <ArrowUpRight
            size={13}
            className="home-module-arrow"
            aria-hidden="true"
          />
        </span>
        <span className="home-module-value" data-tooltip={value}>
          {value}
        </span>
        <span className="home-module-meta" data-tooltip={meta}>
          {meta}
        </span>
      </span>
    </Link>
  );
}
