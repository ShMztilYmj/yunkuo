import { useCallback, useEffect, useRef, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { Navigate, useLocation } from "react-router-dom";
import { CheckCircle2, RefreshCw, Settings2 } from "lucide-react";
import { api, type TzStatus } from "../../services/api";
import { errorMessage } from "../../services/api/invoke";
import { useAppStore } from "../../store";
import { useProxyStore } from "../../features/proxy/state";
import PageHeader from "../../shared/ui/PageHeader";
import AboutCard from "./settings/AboutCard";
import AiModulesCard from "./settings/AiModulesCard";
import ConfigLoadingState from "./settings/ConfigLoadingState";
import GateRulesCard from "./settings/GateRulesCard";
import IntelKeyCard from "./settings/IntelKeyCard";
import GeneralCard from "./settings/GeneralCard";
import ProxyGuardCard from "./settings/ProxyGuardCard";
import ProxyKernelCard from "./settings/ProxyKernelCard";
import ProtectionCheckupCard from "./settings/ProtectionCheckupCard";
import SettingsRail from "./settings/SettingsRail";
import TimezoneStatusCard from "./settings/TimezoneStatusCard";
import TimezoneSyncCard from "./settings/TimezoneSyncCard";
import TunServiceCard from "./settings/TunServiceCard";
import { settingsSection } from "./settings/sections";
import type { SettingsTab } from "./settings/types";
import { updateSetting, type SettingsUpdater } from "../configuration";

const SETTINGS_TABS: readonly SettingsTab[] = [
  "proxy",
  "timezone",
  "detect",
  "ai",
  "general",
  "about",
];

const isSettingsTab = (value: string | null): value is SettingsTab =>
  value != null && (SETTINGS_TABS as readonly string[]).includes(value);

/** 旧地址兼容：早期的「Claude 启动」分类已并入「检测与门控」。 */
const requestedSettingsTab = (value: string | null): SettingsTab | null => {
  if (value === "launch") return "detect";
  return isSettingsTab(value) ? value : null;
};

export default function Settings() {
  const [appVersion, setAppVersion] = useState("读取中");
  useEffect(() => {
    getVersion()
      .then(setAppVersion)
      .catch(() => setAppVersion("读取失败"));
  }, []);
  const config = useAppStore((s) => s.config);
  const configError = useAppStore((s) => s.configError);
  const configLoading = useAppStore((s) => s.configLoading);
  const refreshConfig = useAppStore((s) => s.refreshConfig);
  const refreshCore = useAppStore((s) => s.refreshCore);
  const core = useProxyStore((s) => s.coreStatus);
  const location = useLocation();
  const [tab, setTab] = useState<SettingsTab>(() => {
    const requested = new URLSearchParams(location.search).get("tab");
    return requestedSettingsTab(requested) ?? "proxy";
  });
  useEffect(() => {
    const requested = new URLSearchParams(location.search).get("tab");
    const next = requestedSettingsTab(requested);
    if (next) setTab(next);
  }, [location.search]);
  // 从保护状态行跳到“保护体检”时直接落到卡片，避免用户再滚动查找。
  useEffect(() => {
    if (location.hash !== "#protection-checkup") return;
    const timer = window.setTimeout(() => {
      document
        .getElementById("protection-checkup")
        ?.scrollIntoView({ block: "start", behavior: "smooth" });
    }, 80);
    return () => window.clearTimeout(timer);
  }, [location.hash, tab, config]);
  const [tzStatus, setTzStatus] = useState<TzStatus | null>(null);
  const [tzBusy, setTzBusy] = useState(false);
  const [tzMsg, setTzMsg] = useState<string | null>(null);
  const [tzError, setTzError] = useState<string | null>(null);
  const [tunMsg, setTunMsg] = useState<string | null>(null);
  const [manualTz, setManualTz] = useState("");
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [reloading, setReloading] = useState(false);
  const savedTimer = useRef<number | undefined>(undefined);

  const loadTz = useCallback(async () => {
    try {
      setTzStatus(await api.tzStatus());
      setTzError(null);
    } catch (e) {
      // 原因必须留在界面上：拿不到时区表时下拉会是空的，只说"失败"等于没说。
      setTzError(`读取系统时区失败：${errorMessage(e)}`);
    }
  }, []);

  useEffect(() => {
    if (tab === "timezone") void loadTz();
  }, [loadTz, tab]);

  useEffect(
    () => () => {
      if (savedTimer.current) window.clearTimeout(savedTimer.current);
    },
    [],
  );

  if (new URLSearchParams(location.search).get("tab") === "hud")
    return <Navigate to="/ai-stats?tab=hud" replace />;

  if (!config) {
    return (
      <ConfigLoadingState
        configLoading={configLoading}
        configError={configError}
        onReload={() => void refreshConfig().catch(() => {})}
      />
    );
  }

  const upd: SettingsUpdater = (key, value) => {
    setSaveError(null);
    return updateSetting(key, value)
      .then(() => {
        refreshCore().catch(() => undefined);
        setSaved(true);
        if (savedTimer.current) window.clearTimeout(savedTimer.current);
        savedTimer.current = window.setTimeout(() => setSaved(false), 2200);
      })
      .catch((error) => {
        // 原生 alert 会阻塞 WebView；错误放进页头说明行。
        setSaveError(`设置保存失败：${String(error)}`);
      });
  };

  const reload = async () => {
    if (reloading) return;
    setReloading(true);
    try {
      await refreshConfig();
    } catch {
      // 读取失败会写进 configError，由页头说明行呈现。
    } finally {
      setReloading(false);
    }
  };

  /** 切换分类时回到工作区顶部：长列表滚到一半再切组时，新内容不应从中间开始。 */
  const changeTab = (next: SettingsTab) => {
    if (next === tab) return;
    setTab(next);
    document.querySelector(".workspace-content")?.scrollTo({ top: 0 });
  };

  const syncTz = async () => {
    setTzBusy(true);
    setTzMsg(null);
    try {
      const r = await api.syncTimezone();
      setTzMsg(r.message);
      await loadTz();
    } catch (e) {
      setTzMsg(String(e));
    } finally {
      setTzBusy(false);
    }
  };

  const restoreTz = async () => {
    setTzBusy(true);
    setTzMsg(null);
    try {
      await api.restoreTimezone();
      setTzMsg("已还原为原始时区");
      await loadTz();
    } catch (e) {
      setTzMsg(String(e));
    } finally {
      setTzBusy(false);
    }
  };

  const applyManualTz = async () => {
    if (!manualTz) return;
    setTzBusy(true);
    setTzMsg(null);
    try {
      await api.setTimezoneManual(manualTz);
      setTzMsg(`已切换系统时区：${manualTz}`);
      await loadTz();
    } catch (e) {
      setTzMsg(String(e));
    } finally {
      setTzBusy(false);
    }
  };

  const section = settingsSection(tab);
  const coreRunning = core?.running === true;

  const headerNote = saveError ? (
    <span className="settings-save-state" data-tone="error" role="alert">
      {saveError}
    </span>
  ) : configError ? (
    <span className="settings-save-state" data-tone="error" role="alert">
      读取配置失败：{configError}
    </span>
  ) : saved ? (
    <span className="settings-save-state" data-tone="saved" role="status">
      <CheckCircle2 aria-hidden="true" />
      已保存到本地配置
    </span>
  ) : (
    <span>更改会即时写入本地配置，无需手动保存</span>
  );

  return (
    <div className="settings-page">
      <PageHeader
        icon={Settings2}
        title="设置"
        meta={
          <span className="settings-meta">
            <span data-tone={core ? (coreRunning ? "ok" : "warn") : undefined}>
              内核
              <strong>
                {core
                  ? coreRunning
                    ? (core.version ?? "运行中")
                    : "已停止"
                  : "读取中"}
              </strong>
            </span>
            <span
              data-tone={
                core ? (core.system_proxy ? "ok" : undefined) : undefined
              }
            >
              系统代理
              <strong>
                {core ? (core.system_proxy ? "已开启" : "已关闭") : "未知"}
              </strong>
            </span>
            <span data-tone={core ? (core.tun ? "ok" : undefined) : undefined}>
              TUN
              <strong>
                {core ? (core.tun ? "已开启" : "已关闭") : "未知"}
              </strong>
            </span>
            <span data-tone={config.tz_auto_sync ? "ok" : undefined}>
              时区同步
              <strong>{config.tz_auto_sync ? "自动" : "手动"}</strong>
            </span>
            <span>
              门控
              <strong>阈值 {config.gate_threshold} 分</strong>
            </span>
          </span>
        }
        actions={
          <button
            type="button"
            className="proxy-action"
            disabled={reloading}
            aria-busy={reloading}
            onClick={() => void reload()}
          >
            <RefreshCw className={reloading ? "animate-spin" : ""} />
            {reloading ? "读取中…" : "重新读取"}
          </button>
        }
        note={headerNote}
      />

      <div className="settings-body">
        <SettingsRail tab={tab} onChange={changeTab} appVersion={appVersion} />

        <div className="settings-content" key={tab}>
          <header className="settings-section-head">
            <div>
              <h2>{section.title}</h2>
              <p>{section.description}</p>
            </div>
          </header>

          {tab === "proxy" && (
            <div className="settings-stack">
              <ProxyKernelCard config={config} onUpdate={upd} />
              <ProxyGuardCard config={config} onUpdate={upd} />
              <TunServiceCard message={tunMsg} onMessage={setTunMsg} />
              <ProtectionCheckupCard />
            </div>
          )}

          {tab === "timezone" && (
            <div className="settings-stack">
              <TimezoneStatusCard tzStatus={tzStatus} />
              <TimezoneSyncCard
                config={config}
                onUpdate={upd}
                tzStatus={tzStatus}
                tzBusy={tzBusy}
                tzMsg={tzMsg}
                tzError={tzError}
                manualTz={manualTz}
                onManualTzChange={setManualTz}
                onSync={syncTz}
                onRestore={restoreTz}
                onApplyManual={applyManualTz}
              />
            </div>
          )}

          {tab === "detect" && (
            <div className="settings-stack">
              <GateRulesCard config={config} onUpdate={upd} />
              <IntelKeyCard config={config} onUpdate={upd} />
            </div>
          )}

          {tab === "ai" && (
            <div className="settings-stack">
              <AiModulesCard config={config} onUpdate={upd} />
            </div>
          )}

          {tab === "general" && (
            <div className="settings-stack">
              <GeneralCard config={config} onUpdate={upd} />
            </div>
          )}

          {tab === "about" && (
            <div className="settings-stack">
              <AboutCard appVersion={appVersion} />
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
