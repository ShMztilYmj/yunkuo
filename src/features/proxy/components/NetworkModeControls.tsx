import { useEffect, useId, useRef, useState } from "react";
import { Link } from "react-router-dom";
import {
  Loader2,
  ShieldCheck,
  Split,
  Undo2,
  Globe2,
  ListFilter,
  ArrowRight,
  SlidersHorizontal,
} from "lucide-react";
import { api, type NetworkMode, type RoutingMode } from "../../../services/api";
import { useProxyStore } from "../state";
import { proxyConfiguration, type ProxyConfiguration } from "../configuration";
import { notifyEnvironmentInvalidated } from "../../../shared/events/environment";

type NetworkScheme = NetworkMode | "custom";
const schemeLabels: Record<NetworkScheme, string> = {
  custom: "自定义",
  daily: "日常分流",
  strict: "AI 严格保护",
};
const routingLabels: Record<RoutingMode, string> = {
  global: "全局",
  rule: "规则",
  direct: "直连",
};
const configuredScheme = (config: ProxyConfiguration | null): NetworkScheme =>
  config?.network_mode ?? "custom";
const configuredRouting = (config: ProxyConfiguration | null): RoutingMode =>
  config?.mode === "global" || config?.mode === "direct" ? config.mode : "rule";

export default function NetworkModeControls() {
  const radioGroup = useId();
  const detailsId = useId();
  const [detailsOpen, setDetailsOpen] = useState(false);
  const config = useProxyStore((state) => state.config);
  const privacy = useProxyStore((state) => state.privacyStatus);
  const coreStatus = useProxyStore((state) => state.coreStatus);
  const coreRunning = coreStatus?.running === true;
  const operationRunning = useRef(false);
  const [scheme, setScheme] = useState<NetworkScheme>(() =>
    configuredScheme(config),
  );
  const [customRouting, setCustomRouting] = useState<RoutingMode>(() =>
    configuredRouting(config),
  );
  const [busy, setBusy] = useState(false);
  const [acknowledged, setAcknowledged] = useState(false);
  const [backup, setBackup] = useState<{
    available: boolean;
    createdAt: string | null;
  } | null>(null);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  useEffect(() => {
    setScheme(configuredScheme(config));
    setCustomRouting(configuredRouting(config));
    setAcknowledged(false);
  }, [config?.network_mode, config?.mode]);
  useEffect(() => {
    let active = true;
    api
      .networkModeBackupStatus()
      .then((value) => {
        if (active) setBackup(value);
      })
      .catch(() => {
        if (active) setBackup(null);
      });
    return () => {
      active = false;
    };
  }, [config?.network_mode, config?.mode]);
  const armed = privacy?.armed === true;
  const healthy =
    armed && privacy?.healthy === true && privacy?.tunnelOn === true;
  const routing =
    scheme === "daily"
      ? "rule"
      : scheme === "strict"
        ? "global"
        : customRouting;
  const pending =
    !!config &&
    (scheme !== configuredScheme(config) ||
      routing !== configuredRouting(config));
  const needsRelease = scheme !== "strict" && armed;
  const selectScheme = (next: NetworkScheme) => {
    if (next === "custom") setCustomRouting(routing);
    setScheme(next);
    setAcknowledged(false);
    setMessage("");
    setError("");
  };
  const applyLabel =
    scheme === "strict"
      ? "启用或重新启用严格保护"
      : scheme === "daily"
        ? needsRelease
          ? "解除严格保护并切换日常分流"
          : "应用日常分流"
        : `${needsRelease ? "解除严格保护并切换" : "应用"}自定义 · ${routingLabels[routing]}路由`;
  const act = async (action: "apply" | "restore" | "release") => {
    if (
      operationRunning.current ||
      !config ||
      ((action !== "apply" || needsRelease) && armed && !acknowledged)
    )
      return;
    operationRunning.current = true;
    setBusy(true);
    setError("");
    setMessage("");
    notifyEnvironmentInvalidated();
    try {
      if (action === "release") {
        await proxyConfiguration.releaseProtection();
        setMessage(
          "保护已解除，系统代理与 TUN 已关闭。再次启动 AI 前请启用严格保护并重新检测。",
        );
      } else {
        if (action === "restore")
          await proxyConfiguration.restoreNetworkMode(acknowledged);
        else if (scheme === "custom")
          await proxyConfiguration.setRoutingMode(routing, acknowledged);
        else await proxyConfiguration.setNetworkMode(scheme, acknowledged);
        setMessage(
          action === "restore"
            ? "已恢复切换前的网络配置；原先启用的严格保护会重新启用。请重新检测。"
            : scheme === "daily"
              ? "日常分流配置已应用。请在节点页的“全局节点”里选择出口；国内直连，其他请求走该组。"
              : scheme === "strict"
                ? "严格保护已启用。请重新执行环境检测后启动 AI。"
                : `已应用自定义方案与${routingLabels[routing]}路由；系统代理与 TUN 开关保持当前设置，DNS 按当前配置解析。`,
        );
      }
      setAcknowledged(false);
    } catch (reason) {
      setError(String(reason));
    } finally {
      await api
        .networkModeBackupStatus()
        .then(setBackup)
        .catch(() => setBackup(null));
      operationRunning.current = false;
      setBusy(false);
    }
  };
  const protectionStatus =
    privacy == null && config?.network_mode === "strict"
      ? "严格保护状态未知，请先刷新状态"
      : healthy
        ? "严格保护运行中"
        : armed
          ? "严格阻断已保留，需重新启用恢复代理联网"
          : config?.network_mode === "strict"
            ? "已配置严格方案，保护未启用"
            : null;
  return (
    <div
      className="network-mode-controls network-mode-compact"
      aria-label="网络方案与路由"
      aria-busy={busy}
    >
      <div className="network-mode-section-heading">
        <strong>网络方案</strong>
        <small>选择后点击应用</small>
      </div>
      <div
        className="network-mode-options"
        role="radiogroup"
        aria-label="网络方案"
      >
        {(
          [
            ["custom", "自定义", "自行选择路由与接管", SlidersHorizontal],
            ["daily", "日常分流", "国内直连 · 国外代理", Split],
            ["strict", "AI 严格保护", "全局代理 · WFP 断线阻断", ShieldCheck],
          ] as const
        ).map(([mode, label, hint, Icon]) => (
          <label
            key={mode}
            className="network-mode-option"
            data-selected={scheme === mode}
          >
            <input
              type="radio"
              name={`network-scheme-${radioGroup}`}
              value={mode}
              checked={scheme === mode}
              disabled={busy}
              onChange={() => selectScheme(mode)}
            />
            <Icon size={16} aria-hidden="true" />
            <span>
              <strong>{label}</strong>
              <small>{hint}</small>
            </span>
          </label>
        ))}
      </div>
      <div className="network-mode-section-heading">
        <strong>路由方式</strong>
        <small>{scheme === "custom" ? "内核选路" : "由方案固定"}</small>
      </div>
      <div
        className="network-mode-routing-options"
        role="radiogroup"
        aria-label="路由方式"
      >
        {(
          [
            ["global", "全局", "已接管流量走代理", Globe2],
            ["rule", "规则", "按订阅规则分流", ListFilter],
            ["direct", "直连", "业务连接直连", ArrowRight],
          ] as const
        ).map(([mode, label, hint, Icon]) => (
          <label
            key={mode}
            className="network-mode-option"
            data-selected={routing === mode}
            data-locked={scheme !== "custom"}
          >
            <input
              type="radio"
              name={`network-routing-${radioGroup}`}
              value={mode}
              checked={routing === mode}
              disabled={busy || scheme !== "custom"}
              onChange={() => {
                setCustomRouting(mode);
                setAcknowledged(false);
                setMessage("");
                setError("");
              }}
            />
            <Icon size={15} aria-hidden="true" />
            <span>
              <strong>{label}</strong>
              <small>{hint}</small>
            </span>
          </label>
        ))}
      </div>
      {scheme !== "custom" && (
        <p className="network-mode-detail network-mode-routing-hint">
          {schemeLabels[scheme]}使用{routingLabels[routing]}
          路由；需要调整时先选择“自定义”。
        </p>
      )}
      <p
        className="network-mode-status"
        role="status"
        aria-label="当前网络配置"
        data-running={coreRunning}
      >
        {config
          ? `当前：${schemeLabels[configuredScheme(config)]} · ${routingLabels[configuredRouting(config)]}路由 · ${!coreStatus ? "内核状态未知" : coreRunning ? "内核运行中" : "内核未运行"}`
          : "正在读取网络配置…"}
      </p>
      {pending && (
        <p className="network-mode-draft" role="status">
          待应用：{schemeLabels[scheme]} · {routingLabels[routing]}路由
        </p>
      )}
      {protectionStatus && (
        <p
          className="network-mode-status"
          role="status"
          data-protected={healthy}
        >
          {protectionStatus}
          {armed && (
            <>
              {" · "}
              <Link
                className="network-mode-link"
                to="/settings?tab=proxy#protection-checkup"
              >
                保护体检
              </Link>
            </>
          )}
        </p>
      )}
      {privacy?.error && armed && (
        <p className="network-mode-detail">{privacy.error}</p>
      )}
      {armed && (
        <label className="network-mode-ack">
          <input
            type="checkbox"
            checked={acknowledged}
            disabled={busy}
            onChange={(event) => setAcknowledged(event.target.checked)}
          />
          我已关闭 AI 客户端，允许解除或重建严格保护
        </label>
      )}
      <div className="network-mode-actions">
        <button
          className="btn-primary rounded-lg px-3 py-2 text-xs text-white disabled:opacity-50"
          disabled={busy || !config || (needsRelease && !acknowledged)}
          onClick={() => void act("apply")}
        >
          {busy ? <Loader2 size={13} className="animate-spin" /> : null}
          {applyLabel}
        </button>
        <button
          type="button"
          className="network-mode-link"
          aria-expanded={detailsOpen}
          aria-controls={detailsId}
          onClick={() => setDetailsOpen((value) => !value)}
        >
          {detailsOpen ? "收起详情" : "详情与恢复"}
        </button>
      </div>
      <div
        id={detailsId}
        hidden={!detailsOpen}
        role="region"
        aria-label="网络方案详情与恢复"
        className="space-y-3 rounded-xl border border-zinc-800 p-3"
      >
        <p className="network-mode-detail">
          {scheme === "daily"
            ? "AI 域名优先走代理，国内域名/IP 和局域网直连。已接管的流量在节点失败时不回退直连，AI 的 UDP/QUIC 保守拒绝。此模式未提供云阔代理运行时退出后的整机阻断；软件内 AI 启动仍要求严格保护。"
            : scheme === "strict"
              ? "覆盖宿主 Windows 的普通出站连接。代理、界面或服务退出后保留阻断；可能影响局域网和其他软件联网。"
              : routing === "global"
                ? "仅已接管的业务流量统一使用全局代理选择，节点可在代理中心调整。系统代理与 TUN 由各自开关控制，切换此模式不会自动开启接管。"
                : routing === "rule"
                  ? "使用订阅中的原有规则决定代理、直连或拒绝；订阅只有一条兜底 Match 时，规则模式 ≈ 全局模式。"
                  : "已接管的业务连接直接出站。系统代理与 TUN 开关保持当前设置。"}
        </p>
        {scheme === "custom" && (
          <p className="network-mode-detail">
            自定义 DNS 按订阅配置，缺省 DoH 直连；严格 DNS 由方案统一管理。
            切换全局、规则或直连在内核内原地生效：不重启内核，现有连接不会被掐断。
          </p>
        )}
        <div className="network-mode-actions">
          <button
            className="network-mode-secondary"
            disabled={busy || !backup?.available || (armed && !acknowledged)}
            onClick={() => void act("restore")}
          >
            <Undo2 size={13} />
            恢复切换前网络配置
          </button>
          {armed && (
            <button
              className="network-mode-secondary"
              disabled={busy || !acknowledged}
              onClick={() => void act("release")}
            >
              解除保护并恢复联网
            </button>
          )}
        </div>
        <p className="network-mode-detail">
          {backup?.available
            ? `切换前备份已保存${backup.createdAt ? ` · ${new Date(backup.createdAt).toLocaleString()}` : ""}。恢复只还原网络字段，订阅原文件保持不变。`
            : "首次应用日常或严格方案前自动保存网络配置备份；选择自定义后可调整内核路由。"}{" "}
          <Link className="network-mode-link" to="/proxies">
            选择出口节点
          </Link>
        </p>
      </div>
      {message && (
        <p className="network-mode-detail" role="status">
          {message}
        </p>
      )}
      {error && (
        <p className="network-mode-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
