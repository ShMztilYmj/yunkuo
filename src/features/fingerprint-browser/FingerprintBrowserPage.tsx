import { useCallback, useEffect, useState } from "react";
import { Link, Navigate, useSearchParams } from "react-router-dom";
import {
  AlertTriangle,
  ArrowUpRight,
  Check,
  Fingerprint,
  FolderOpen,
  Globe2,
  Loader2,
  Save,
  ShieldCheck,
  Shuffle,
  Trash2,
} from "lucide-react";
import {
  browserEnvironment as environment,
  type BrowserEnvironment,
  type BrowserConfig,
  type BrowserDiagnostics,
  type BrowserRequests,
  type Region,
} from "./services/browser";
import { usePolling } from "../../shared/hooks/usePolling";
import { useConfirm } from "../../shared/ui/useConfirm";
import PageHeader from "../../shared/ui/PageHeader";
import BrowserDiagnosticsSummary from "./components/BrowserDiagnostics";
import BrowserBot, { type BrowserBotState } from "./components/BrowserBot";

const button = "env-button";
const primary = `${button} btn-primary`;
const input = "env-input";

/** 单屏页面里的启动反馈只保留一个状态位，详细原因放在 title 里。 */
type LaunchState =
  | { phase: "checking"; openClaude: boolean }
  | { phase: "failed"; openClaude: boolean; reason: string };

function useOperation() {
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    setMessage(null);
    try {
      await action();
    } catch (e) {
      setMessage(String(e));
    } finally {
      setBusy(false);
    }
  };
  return { busy, message, setMessage, run };
}

function BrowserPanel({ diagnostics }: { diagnostics?: BrowserDiagnostics }) {
  const [launchState, setLaunchState] = useState<LaunchState | null>(null);
  const [state, setState] = useState<BrowserEnvironment | null>(null);
  const [config, setConfig] = useState<BrowserConfig | null>(null);
  const [presets, setPresets] = useState<Region[]>([]);
  const [requests, setRequests] = useState<BrowserRequests>({
    pendingCount: 0,
    notice: null,
  });
  const op = useOperation();
  const { confirm, dialog } = useConfirm();
  const load = useCallback(async () => {
    const next = await environment.browser();
    setState(next);
    return next;
  }, []);
  useEffect(() => {
    let active = true;
    environment
      .regions()
      .then((value) => {
        if (active) setPresets(value);
      })
      .catch((e) => {
        if (active) op.setMessage(String(e));
      });
    return () => {
      active = false;
    };
  }, []);
  usePolling(async (isCurrent) => {
    const [browser, request] = await Promise.allSettled([
      environment.browser(),
      environment.browserRequests(),
    ]);
    if (!isCurrent()) return;
    if (browser.status === "fulfilled") {
      setState(browser.value);
      // 后续轮询保留尚未保存的编辑。
      setConfig((previous) => previous ?? browser.value.config);
    } else op.setMessage(String(browser.reason));
    if (request.status === "fulfilled") setRequests(request.value);
    else op.setMessage(String(request.reason));
  }, 3000);
  const save = (rotate = false) =>
    op.run(async () => {
      if (!config) return;
      setLaunchState(null);
      await environment.saveBrowser(config, rotate);
      const next = await load();
      setConfig(next.config);
      op.setMessage(
        rotate
          ? "已更换当前资料的指纹种子，下次启动浏览器时生效。"
          : "浏览器配置已保存，下次启动浏览器时生效。",
      );
    });
  const launch = (open = false) =>
    op.run(async () => {
      setLaunchState({ phase: "checking", openClaude: open });
      try {
        await environment.launchBrowser(open);
      } catch (error) {
        setLaunchState({
          phase: "failed",
          openClaude: open,
          reason: String(error),
        });
        return;
      }
      setLaunchState(null);
      op.setMessage(
        open
          ? "已请求在当前独立资料中打开 Claude。"
          : "浏览器已打开 ippure.com，可在网页查看当前 IP 与浏览器环境。",
      );
      await load();
    });
  const clear = (all: boolean) =>
    op.run(async () => {
      if (
        !(await confirm(
          all
            ? {
                tone: "danger",
                icon: Trash2,
                eyebrow: "指纹浏览器 · 完整重置",
                title: "永久删除当前资料的全部数据？",
                description:
                  "密码、书签、历史、扩展和网站登录会被永久删除，没有备份；其他身份的资料保留。",
                note: {
                  tone: "danger",
                  text: "删除后无法撤销，账号需要重新登录。",
                },
                confirmLabel: "永久删除",
              }
            : {
                tone: "warning",
                icon: Trash2,
                eyebrow: "指纹浏览器 · 清理",
                title: "清除当前资料的缓存？",
                description:
                  "只清缓存，保留密码、书签与网站登录。清理前请先关闭指纹浏览器。",
                confirmLabel: "清理缓存",
              },
        ))
      )
        return;
      await environment.clearBrowser(all);
      op.setMessage("所选浏览器数据已清理。");
      await load();
    });
  const dirty =
    config !== null &&
    JSON.stringify(config) !== JSON.stringify(state?.config ?? null);
  const checking = launchState?.phase === "checking";
  const failed = launchState?.phase === "failed" ? launchState : null;
  const botState: BrowserBotState = !state?.installed
    ? "unavailable"
    : checking
      ? "checking"
      : failed
        ? "attention"
        : dirty
          ? "idle"
          : state?.running
            ? "passed"
            : "idle";
  const selectedPreset =
    presets.find(
      (preset) =>
        preset.language === config?.language &&
        preset.timezone === config?.timezone,
    )?.id ?? "";
  const reasons = failed
    ? [
        ...new Set(
          failed.reason
            .replace(/^Error:\s*/, "")
            .split(/[；\r\n]+/)
            .map((reason) => reason.trim())
            .filter(Boolean),
        ),
      ]
    : [];
  const networkBlocked = /TUN|虚拟网卡|系统代理|出站规则|代理内核/.test(
    failed?.reason ?? "",
  );
  const status = failed
    ? {
        tone: "danger",
        text: reasons.length
          ? reasons.join("；")
          : "未收到具体原因，请稍后重试。",
      }
    : requests.notice
      ? { tone: "warn", text: requests.notice }
      : op.message
        ? { tone: "info", text: op.message }
        : checking
          ? {
              tone: "info",
              text: launchState?.openClaude
                ? "正在检查系统代理与虚拟网卡…"
                : "正在检查浏览器启动条件…",
            }
          : null;
  return (
    <div className="environment-page fp-page">
      <PageHeader
        icon={Fingerprint}
        title="指纹浏览器"
        meta={
          <>
            <FolderOpen size={12} aria-hidden="true" />
            <span>
              当前资料 <strong>{state?.slot ?? "读取中"}</strong>
            </span>
            <i className="proxy-meta-dot" aria-hidden="true" />
            <span>Fingerprint Chromium</span>
            <i className="proxy-meta-dot" aria-hidden="true" />
            <span>每个身份一份独立资料</span>
          </>
        }
        actions={
          <span
            className={`env-badge ${state?.installed ? "is-good" : state ? "is-warning" : ""}`}
          >
            <i />
            {state ? (state.installed ? "引擎已准备" : "引擎未准备") : "读取中"}
          </span>
        }
        note={
          status ? (
            <span
              className={
                status.tone === "danger"
                  ? "is-error"
                  : status.tone === "warn"
                    ? "is-warning"
                    : undefined
              }
            >
              {status.text}
            </span>
          ) : undefined
        }
      />
      <div className="fp-grid">
        <section className="fp-card fp-config" aria-label="浏览器环境">
          <div className="fp-card-head">
            <h2>浏览器环境</h2>
            {config && (
              <span className={`env-save-state ${dirty ? "is-dirty" : ""}`}>
                {dirty ? (
                  <>
                    <i />
                    配置有修改
                  </>
                ) : (
                  <>
                    <Check size={11} />
                    配置已保存
                  </>
                )}
              </span>
            )}
          </div>
          {state && !state.installed && (
            <div className="fp-notice">
              <AlertTriangle size={12} />
              <span>
                未找到可用浏览器引擎，请检查安装资源或重新安装完整软件包。
              </span>
            </div>
          )}
          {requests.pendingCount > 0 && (
            <div className="fp-notice">
              <AlertTriangle size={12} />
              <span>待处理外部链接 {requests.pendingCount} 个</span>
              <button
                className="fp-notice-action"
                disabled={op.busy || !state?.installed || dirty}
                onClick={() =>
                  op.run(async () => {
                    await environment.openPending();
                    setRequests(await environment.browserRequests());
                    op.setMessage("待处理链接已在当前身份的指纹浏览器中打开。");
                  })
                }
              >
                打开
              </button>
            </div>
          )}
          {config ? (
            <div className="fp-fields">
              <label className="fp-field fp-preset">
                <span>
                  <Globe2 size={12} />
                  地区预设
                </span>
                <select
                  aria-label="地区预设"
                  className={input}
                  value={selectedPreset}
                  onChange={(event) => {
                    const preset = presets.find(
                      (preset) => preset.id === event.target.value,
                    );
                    if (preset)
                      setConfig({
                        ...config,
                        language: preset.language,
                        timezone: preset.timezone,
                      });
                  }}
                >
                  <option value="" disabled>
                    自定义语言与时区
                  </option>
                  {presets.map((preset) => (
                    <option key={preset.id} value={preset.id}>
                      {preset.name}
                    </option>
                  ))}
                </select>
              </label>
              <label className="fp-field">
                <span>浏览器语言</span>
                <select
                  aria-label="浏览器语言"
                  className={input}
                  value={config.language}
                  onChange={(event) =>
                    setConfig({ ...config, language: event.target.value })
                  }
                >
                  {[
                    "en-US",
                    "en-GB",
                    "en-CA",
                    "en-AU",
                    "en-SG",
                    "ja-JP",
                    "ko-KR",
                    "de-DE",
                    "fr-FR",
                    "zh-TW",
                  ].map((language) => (
                    <option key={language}>{language}</option>
                  ))}
                </select>
              </label>
              <label className="fp-field fp-tz">
                <span>IANA 时区</span>
                <input
                  aria-label="IANA 时区"
                  className={input}
                  value={config.timezone}
                  onChange={(event) =>
                    setConfig({ ...config, timezone: event.target.value })
                  }
                />
              </label>
              <label className="fp-field">
                <span>指纹种子</span>
                <input
                  aria-label="指纹种子"
                  type="number"
                  min="1"
                  max="4294967295"
                  className={`${input} env-seed-input`}
                  value={config.seed}
                  onChange={(event) =>
                    setConfig({ ...config, seed: Number(event.target.value) })
                  }
                />
              </label>
            </div>
          ) : (
            <div className="fp-loading" role="status">
              <Loader2 size={14} className="animate-spin" />
              正在读取浏览器配置…
            </div>
          )}
          <div className="fp-actions">
            <button
              className={button}
              disabled={op.busy || !config}
              onClick={() => save()}
            >
              <Save size={12} />
              保存配置
            </button>
            <button
              className={button}
              disabled={op.busy || !config}
              onClick={() => save(true)}
            >
              <Shuffle size={12} />
              换指纹
            </button>
            <small className="fp-preset-hint">出口 IP 由代理节点决定</small>
          </div>
          <div className="fp-identity" aria-label="身份资料隔离说明">
            <div className="fp-identity-body">
              <span className="fp-identity-icon" aria-hidden="true">
                <Fingerprint size={21} />
              </span>
              <div className="fp-identity-copy">
                <strong>独立身份资料</strong>
                <p>
                  语言、时区与指纹种子只作用于当前资料；Cookie、缓存、登录和代理出口与其他身份互不影响。
                </p>
              </div>
            </div>
            <div className="fp-identity-tags" aria-hidden="true">
              <span>
                <Check size={10} />
                独立 Cookie
              </span>
              <span>
                <Check size={10} />
                独立指纹
              </span>
              <span>
                <Check size={10} />
                独立出口
              </span>
            </div>
            <div className="fp-identity-facts">
              <div>
                <span>语言</span>
                <strong>{config?.language ?? "—"}</strong>
              </div>
              <div>
                <span>IANA 时区</span>
                <strong>{config?.timezone ?? "—"}</strong>
              </div>
              <div>
                <span>指纹种子</span>
                <strong>{config?.seed ?? "—"}</strong>
              </div>
            </div>
          </div>
          <div className="fp-launch">
            <button
              disabled={op.busy || !state?.installed || dirty}
              aria-busy={checking && !launchState.openClaude}
              aria-label={
                checking && !launchState.openClaude ? "正在检查…" : "启动浏览器"
              }
              data-tooltip={dirty ? "请先保存配置" : undefined}
              className={`${primary} fp-launch-main`}
              onClick={() => launch()}
            >
              <BrowserBot
                state={botState}
                size={36}
                animated={!dirty && (!op.busy || checking)}
              />
              <span className="fp-launch-copy">
                <strong>
                  {checking && !launchState.openClaude
                    ? "正在检查…"
                    : "启动浏览器"}
                </strong>
                <small aria-hidden="true">独立资料 · ippure.com</small>
              </span>
              <ArrowUpRight size={16} className="fp-launch-arrow" />
            </button>
            <button
              className={`${button} fp-launch-alt`}
              disabled={op.busy || !state?.installed || dirty}
              aria-busy={checking && launchState.openClaude}
              data-tooltip={
                dirty ? "请先保存配置" : "在当前独立资料中打开 Claude"
              }
              onClick={() => launch(true)}
            >
              {checking && launchState.openClaude ? "正在核验…" : "打开 Claude"}
              <ArrowUpRight size={12} />
            </button>
          </div>
          {status && (
            <div
              role={failed ? "alert" : "status"}
              className={`fp-status is-${status.tone}`}
            >
              {failed || status.tone === "warn" ? (
                <AlertTriangle size={11} />
              ) : (
                <ShieldCheck size={11} />
              )}
              <span title={status.text}>{status.text}</span>
              {failed && networkBlocked && <Link to="/overview">代理设置</Link>}
              {failed && <Link to="/detect">检测</Link>}
            </div>
          )}
        </section>
        <aside className="fp-side" aria-label="环境参考与数据管理">
          <section className="fp-card fp-detect" aria-label="环境检测参考">
            <div className="fp-card-head">
              <h2>环境检测参考</h2>
              <Link className="fp-link" to="/detect">
                前往检测
                <ArrowUpRight size={11} />
              </Link>
            </div>
            <BrowserDiagnosticsSummary diagnostics={diagnostics} />
          </section>
          <section className="fp-card fp-data" aria-label="浏览器数据">
            <div className="fp-card-head">
              <h2>浏览器数据</h2>
            </div>
            <div className="fp-data-actions">
              <button
                className={`${button} fp-small`}
                disabled={op.busy}
                onClick={() => clear(false)}
              >
                仅清缓存
              </button>
              <button
                className={`${button} env-button-danger fp-small`}
                disabled={op.busy}
                onClick={() => clear(true)}
              >
                完整重置
              </button>
            </div>
            <p className="fp-note">
              仅清缓存保留密码、书签与登录；完整重置会删除当前身份资料，无备份。
            </p>
          </section>
        </aside>
      </div>
      <p className="fp-foot">
        <ShieldCheck size={11} />
        <span>
          启动前会核验系统代理、虚拟网卡与出站模式；修改配置、换指纹或清理数据前请先关闭指纹浏览器。
        </span>
      </p>
      {dialog}
    </div>
  );
}

export default function FingerprintBrowserPage({
  diagnostics,
}: {
  diagnostics?: BrowserDiagnostics;
}) {
  const [searchParams] = useSearchParams();
  if (searchParams.get("tab") === "identity")
    return <Navigate to="/ai?tool=claude&view=accounts" replace />;
  return <BrowserPanel diagnostics={diagnostics} />;
}
