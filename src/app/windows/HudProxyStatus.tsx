import { useState } from "react";
import { createPortal } from "react-dom";
import ProxyBot from "../../features/proxy/components/ProxyBot";
import HudRing from "./HudRing";
import {
  api,
  type AppConfig,
  type CoreStatus,
  type ProfileItem,
} from "../../services/api";
import {
  formatTrafficBytes,
  profileTraffic,
  subscriptionTrafficView,
} from "../../features/proxy/services/subscriptionTraffic";
import { usePageActive } from "../../shared/hooks/usePageActive";
import { usePolling } from "../../shared/hooks/usePolling";
import { useTauriEvent } from "../../shared/hooks/useTauriEvent";

interface Snapshot {
  core: CoreStatus | null;
  config: AppConfig | null;
  profile: ProfileItem | null;
  profileFailed: boolean;
}

export type HudNetworkDetail = "network" | null;

interface Props {
  ready: boolean;
  expanded: boolean;
  detail: HudNetworkDetail;
  onDetailChange: (detail: HudNetworkDetail) => void;
  detailsHost: HTMLDivElement | null;
}

function proxyView(snapshot: Snapshot | null): {
  state: "on" | "off" | "unknown";
  text: string;
  detail: string;
} {
  if (!snapshot)
    return {
      state: "unknown",
      text: "代理检测中…",
      detail: "正在读取本软件代理状态",
    };
  const { core, config } = snapshot;
  // Bot 跟随系统代理开关；只开 TUN 或仅运行内核，也保持休眠造型。
  if (core?.system_proxy === false)
    return {
      state: "off",
      text: "系统代理已关闭",
      detail: `系统代理已关闭；TUN ${typeof core?.tun !== "boolean" ? "未知" : core.tun ? "开" : "关"}；${core ? (core.running ? "内核运行中" : "内核已停止") : "内核状态未知"}`,
    };
  if (!core)
    return {
      state: "unknown",
      text: "代理状态未知",
      detail: "本软件代理状态读取失败，稍后自动重试",
    };
  if (!core.running)
    return { state: "off", text: "代理未启动", detail: "本软件代理内核未运行" };
  if (!config)
    return {
      state: "unknown",
      text: "内核运行 · 配置未知",
      detail: "内核正在运行，但代理配置读取失败",
    };
  const detail = `本软件内核 ${core.version ?? ""}；系统代理${typeof core.system_proxy !== "boolean" ? "未知" : core.system_proxy ? "开" : "关"}，TUN ${typeof core?.tun !== "boolean" ? "未知" : core.tun ? "开" : "关"}`;
  if (core.system_proxy)
    return {
      state: "on",
      text: "代理已开启",
      detail: config.mode === "direct" ? `${detail}；当前为直连路由` : detail,
    };
  return {
    state: "unknown",
    text: "系统代理状态未知",
    detail: "未能确认系统代理开关状态",
  };
}

export default function HudProxyStatus({
  ready,
  expanded,
  detail,
  onDetailChange,
  detailsHost,
}: Props) {
  const active = usePageActive() && ready;
  const [coreStatus, setCoreStatus] = useState<CoreStatus | null>(null);
  const [metadata, setMetadata] = useState<Omit<Snapshot, "core"> | null>(null);
  const [revision, setRevision] = useState(0);
  const [metadataRevision, setMetadataRevision] = useState(0);
  const snapshot: Snapshot | null = metadata
    ? { ...metadata, core: coreStatus }
    : null;
  // 网络事件只推进 revision，具体读取仍交给下面的轮询；页面不活跃时 hook 不订阅。
  const changed = () => {
    setRevision((value) => value + 1);
    setMetadataRevision((value) => value + 1);
  };
  useTauriEvent("core-event", changed, active);
  useTauriEvent("config-changed", changed, active);

  // 内核状态单独刷新；配置/订阅元数据变化较少，不跟随每轮状态重复读文件。
  // 两条轮询都在隐藏时暂停，网络事件与展开详情会立即刷新。
  usePolling(
    async (isCurrent) => {
      const core = await api.coreStatus().catch(() => null);
      if (isCurrent()) setCoreStatus(core);
    },
    30_000,
    true,
    ready,
    `${revision}:${expanded && detail === "network"}`,
  );
  usePolling(
    async (isCurrent) => {
      const [config, profile] = await Promise.allSettled([
        api.getConfig(),
        api.currentProfile(),
      ]);
      if (isCurrent())
        setMetadata({
          config: config.status === "fulfilled" ? config.value : null,
          profile: profile.status === "fulfilled" ? profile.value : null,
          profileFailed: profile.status === "rejected",
        });
    },
    60_000,
    true,
    ready,
    `${metadataRevision}:${expanded && detail === "network"}`,
  );

  const proxy = proxyView(snapshot);
  const profile = snapshot?.profile;
  const mergedTraffic = profile ? profileTraffic(profile).traffic : null;
  const traffic = mergedTraffic ? subscriptionTrafficView(mergedTraffic) : null;
  const unlimited = Boolean(
    profile?.quota && profile.quota.total === 0 && profile.quota.expire > 0,
  );
  const expired =
    traffic != null &&
    traffic.expire > 0 &&
    traffic.expire * 1000 <= Date.now();
  const label = !snapshot
    ? "订阅读取中…"
    : snapshot.profileFailed
      ? "订阅读取失败"
      : !profile
        ? "未选择订阅"
        : traffic?.known
          ? `剩余 ${formatTrafficBytes(traffic.remaining)}`
          : unlimited
            ? "不限量套餐"
            : profile.type === "local"
              ? "本地配置 · 未上报额度"
              : "订阅未提供流量";
  const tone =
    expired || (traffic?.known && traffic.pct >= 90)
      ? "danger"
      : traffic?.known && traffic.pct >= 75
        ? "warning"
        : "";
  const remainingPercent =
    traffic?.known && !expired
      ? Math.max(0, Math.min(100, (traffic.remaining / traffic.total) * 100))
      : null;
  const config = snapshot?.config;
  const core = snapshot?.core;
  const mode =
    config?.network_mode === "daily"
      ? "日常分流 · 规则路由"
      : config?.network_mode === "strict"
        ? "AI 严格保护 · 全局路由"
        : config?.mode === "global"
          ? "自定义 · 全局路由"
          : config?.mode === "direct"
            ? "自定义 · 直连路由"
            : config?.mode === "rule"
              ? "自定义 · 规则路由"
              : "未知";
  const dateLabel = (value: string | number) => {
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? "未知" : date.toLocaleString("zh-CN");
  };
  const combinedLabel = `本软件代理：${proxy.text}；订阅流量：${label}${expired ? "，订阅已到期" : ""}`;
  const stateLabel =
    proxy.state === "on"
      ? "运行中"
      : proxy.state === "off"
        ? "休眠中"
        : "状态未知";

  return (
    <>
      <div className="hud-network" role="group" aria-label="代理与订阅状态">
        <button
          className="hud-network-trigger hud-proxy-state"
          data-state={proxy.state}
          aria-label={combinedLabel}
          data-tooltip="代理与订阅"
          data-tooltip-detail={`${proxy.text}\n${label}${expired ? " · 订阅已到期" : ""}\n查看当前代理和订阅流量详情`}
          aria-expanded={expanded && detail === "network"}
          aria-controls={expanded && detail ? "hud-network-detail" : undefined}
          onMouseEnter={() => onDetailChange("network")}
          onFocus={() => onDetailChange("network")}
          onClick={() => onDetailChange("network")}
        >
          <HudRing
            className={`hud-status-ring hud-traffic ${tone} ${remainingPercent == null ? "muted" : "good"}`}
            value={remainingPercent}
            role={remainingPercent == null ? "img" : "meter"}
            aria-label="订阅剩余比例"
            aria-valuemin={remainingPercent == null ? undefined : 0}
            aria-valuemax={remainingPercent == null ? undefined : 100}
            aria-valuenow={remainingPercent ?? undefined}
            aria-valuetext={
              remainingPercent == null
                ? undefined
                : `${label}，剩余 ${Math.round(remainingPercent)}%`
            }
          >
            <ProxyBot state={proxy.state} active={active} />
          </HudRing>
        </button>
      </div>
      {expanded &&
        detail &&
        detailsHost &&
        createPortal(
          <section
            id="hud-network-detail"
            className={`hud-network-info ${tone}`}
            role="region"
            aria-label="代理与流量详细信息"
          >
            <header className="hud-network-info-header">
              <ProxyBot
                state={proxy.state}
                size={56}
                active={active}
                wakeOnMount={false}
              />
              <div>
                <h2>
                  代理与流量
                  <span
                    className="hud-proxy-status-label"
                    data-state={proxy.state}
                  >
                    {stateLabel}
                  </span>
                </h2>
                <p>
                  {proxy.text} ·{" "}
                  <span>
                    {core
                      ? `${core.running ? "内核运行" : "内核已停止"}${core.version ? ` · ${core.version}` : ""}`
                      : "内核状态未知"}
                  </span>
                </p>
              </div>
            </header>
            <dl className="hud-network-proxy-facts">
              <div>
                <dt>系统代理</dt>
                <dd>
                  {typeof core?.system_proxy === "boolean"
                    ? core.system_proxy
                      ? "已开启"
                      : "已关闭"
                    : "未知"}
                </dd>
              </div>
              <div>
                <dt>TUN 模式</dt>
                <dd>
                  {typeof core?.tun === "boolean"
                    ? core.tun
                      ? "已开启"
                      : "已关闭"
                    : "未知"}
                </dd>
              </div>
              <div>
                <dt>分流模式</dt>
                <dd>{mode}</dd>
              </div>
            </dl>
            <div className="hud-network-balance">
              <span>
                {expired ? "订阅已到期" : "订阅剩余"}
                {remainingPercent != null && (
                  <span>{Math.round(remainingPercent)}%</span>
                )}
              </span>
              <strong>{expired ? "已到期" : label}</strong>
            </div>
            {remainingPercent != null && (
              <div className="hud-network-balance-track">
                <i style={{ width: `${remainingPercent}%` }} />
              </div>
            )}
            <dl className="hud-network-facts">
              <div>
                <dt>当前{profile?.type === "local" ? "配置" : "订阅"}</dt>
                <dd>{profile?.name ?? label}</dd>
              </div>
              {traffic?.known && (
                <>
                  <div>
                    <dt>已用流量</dt>
                    <dd>{formatTrafficBytes(traffic.used)}</dd>
                  </div>
                  <div>
                    <dt>流量总额</dt>
                    <dd>{formatTrafficBytes(traffic.total)}</dd>
                  </div>
                </>
              )}
              <div>
                <dt>到期时间</dt>
                <dd>
                  {traffic?.expire
                    ? dateLabel(traffic.expire * 1000)
                    : "未提供"}
                </dd>
              </div>
              <div>
                <dt>订阅更新</dt>
                <dd>
                  {profile?.updated_at
                    ? dateLabel(profile.updated_at)
                    : "未提供"}
                </dd>
              </div>
            </dl>
            <p className="hud-network-info-note">
              {config?.mixed_port && (
                <>本机端口 127.0.0.1:{config.mixed_port}。 </>
              )}
              流量为订阅商最近返回的读数，更新订阅后刷新，不是实时网速。
            </p>
          </section>,
          detailsHost,
        )}
    </>
  );
}
