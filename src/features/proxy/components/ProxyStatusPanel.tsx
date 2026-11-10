import { Globe, Zap } from "lucide-react";
import { usePolling } from "../../../shared/hooks/usePolling";
import { useProxyStore } from "../state";
import {
  profileTraffic,
  formatTrafficBytes,
  subscriptionTrafficView,
} from "../services/subscriptionTraffic";

/**
 * 代理牌：只报状态，不放任何按钮。
 * 要看详情或改方案，走侧栏的代理入口；这里点开就能一眼看完。
 */
export default function ProxyStatusPanel() {
  const config = useProxyStore((s) => s.config);
  const coreStatus = useProxyStore((s) => s.coreStatus);
  const coreError = useProxyStore((s) => s.coreError);
  const currentProfile = useProxyStore((s) => s.currentProfile);
  const currentQuotaView = currentProfile
    ? profileTraffic(currentProfile)
    : null;
  const quota = currentQuotaView?.traffic
    ? subscriptionTrafficView(currentQuotaView.traffic)
    : null;
  const refreshProfile = useProxyStore((s) => s.refreshProfile);
  usePolling(() => refreshProfile(), 60_000);
  const coreRunning = coreStatus?.running ?? false;
  const managed = !!config?.network_mode;

  const headline =
    config?.network_mode === "daily"
      ? "日常分流"
      : config?.network_mode === "strict"
        ? "AI 严格保护"
        : config?.mode === "global"
          ? "全局"
          : config?.mode === "direct"
            ? "直连"
            : "规则";
  const meta = [
    !coreStatus
      ? "内核状态未知"
      : coreRunning
        ? `内核运行中 ${coreStatus.version ?? ""}`.trim()
        : "内核未运行",
    coreStatus ? (coreStatus.elevated ? "已提权" : "普通权限") : null,
  ]
    .filter(Boolean)
    .join(" · ");
  const flags = [
    { key: "system_proxy", label: "系统代理", on: coreStatus?.system_proxy },
    { key: "tun", label: "TUN", on: coreStatus?.tun },
    { key: "ipv6", label: "IPv6", on: config?.ipv6 },
  ] as const;

  return (
    <section aria-label="代理状态" className="proxy-status">
      <header className="proxy-status-head">
        <Globe size={14} />
        <span>代理</span>
        <span className={`proxy-state ${coreRunning ? "is-on" : ""}`}>
          <i />
          {coreStatus ? (coreRunning ? "运行中" : "已停止") : "状态未知"}
        </span>
      </header>

      <div className="proxy-status-body">
        <div className="proxy-status-figure">
          <span className="proxy-status-suit" aria-hidden="true">
            <Zap size={26} strokeWidth={1.5} />
          </span>
          <strong>{headline}</strong>
          <p className="proxy-status-meta">{meta}</p>
        </div>

        <ul className="proxy-flags">
          {flags.map(({ key, label, on }) => (
            <li key={key} className={on ? "is-on" : ""} title={label}>
              <i />
              {label}
            </li>
          ))}
        </ul>

        {(coreError || coreStatus?.last_error) && (
          <p className="proxy-status-error" role="alert">
            {coreError || coreStatus?.last_error}
          </p>
        )}
        {managed && (
          <p className="proxy-status-note">开关由网络方案统一管理。</p>
        )}

        {currentProfile && (
          <p className="proxy-status-sub">
            {currentProfile.name}
            {quota?.known
              ? ` · 剩余 ${formatTrafficBytes(quota.remaining)}`
              : ""}
          </p>
        )}
      </div>
    </section>
  );
}
