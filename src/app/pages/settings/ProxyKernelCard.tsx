import { Server } from "lucide-react";
import { useEffect, useState } from "react";
import type { AppConfig } from "../../../services/api";
import { useProxyStore } from "../../../features/proxy/state";
import Card from "./Card";
import Row from "./Row";
import Switch from "./Switch";
import type { SettingsUpdater } from "./types";

function PortInput({
  value,
  label,
  onCommit,
}: {
  value: number;
  label: string;
  onCommit: (value: number) => Promise<void>;
}) {
  const [draft, setDraft] = useState(String(value));
  const [error, setError] = useState("");
  useEffect(() => {
    setDraft(String(value));
    setError("");
  }, [value]);
  return (
    <div>
      <input
        type="number"
        aria-label={label}
        aria-invalid={!!error}
        min={1}
        max={65535}
        className="settings-input is-port"
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={async () => {
          const port = Number(draft);
          if (
            !draft.trim() ||
            !Number.isInteger(port) ||
            port < 1 ||
            port > 65535
          ) {
            setError("端口需为 1–65535 的整数");
            setDraft(String(value));
            return;
          }
          setError("");
          if (port !== value) await onCommit(port);
        }}
      />
      {error && (
        <p role="status" className="settings-field-error">
          {error}
        </p>
      )}
    </div>
  );
}

/** 端口与内核：混合代理端口、运行模式、日志等级、局域网 / IPv6。 */
export default function ProxyKernelCard({
  config,
  onUpdate,
}: {
  config: AppConfig;
  onUpdate: SettingsUpdater;
}) {
  const core = useProxyStore((s) => s.coreStatus);
  const managed = !!config.network_mode;
  const tone = core == null ? "idle" : core.running ? "ok" : "warn";
  const badge =
    core == null ? "状态未知" : core.running ? "内核运行中" : "内核已停止";

  return (
    <Card
      title="端口与内核"
      icon={<Server />}
      description="监听端口、出站模式与日志级别；修改端口后会重建内核会话。"
      badge={badge}
      badgeTone={tone}
    >
      <Row label="代理引擎" hint="随应用托管运行，不需要单独安装">
        <span className="settings-status">
          mihomo{core?.version ? ` ${core.version}` : ""}
        </span>
      </Row>
      <Row label="混合代理端口" hint="HTTP 与 SOCKS 共用同一端口">
        <PortInput
          label="混合代理端口"
          value={config.mixed_port}
          onCommit={(port) => onUpdate("mixed_port", port)}
        />
      </Row>
      <Row
        label="运行模式"
        hint={
          managed
            ? "由网络方案管理；应用自定义方案后可调整"
            : "全局模式忽略规则，全部流量走当前节点"
        }
      >
        <select
          value={config.mode}
          disabled={managed}
          onChange={(e) => onUpdate("mode", e.target.value)}
          aria-label="运行模式"
          className="settings-select"
        >
          <option value="rule">规则模式</option>
          <option value="global">全局模式</option>
          <option value="direct">直连模式</option>
        </select>
      </Row>
      <Row label="日志等级" hint="debug 会明显增加日志体积，仅在排查问题时使用">
        <select
          value={config.log_level}
          onChange={(e) => onUpdate("log_level", e.target.value)}
          aria-label="日志等级"
          className="settings-select settings-select--compact"
        >
          {(
            [
              ["debug", "调试"],
              ["info", "信息"],
              ["warning", "警告"],
              ["error", "错误"],
              ["silent", "静默"],
            ] as const
          ).map(([value, label]) => (
            <option key={value} value={value}>
              {label}
            </option>
          ))}
        </select>
      </Row>
      <Row label="允许局域网连接" hint="同网段设备可经本机端口上网">
        {managed ? (
          <span className="settings-value">由网络模式管理 · 关闭</span>
        ) : (
          <Switch
            v={config.allow_lan}
            label="切换局域网连接"
            on={(b) => onUpdate("allow_lan", b)}
          />
        )}
      </Row>
      <Row label="IPv6" hint="为内核解析与出站连接启用 IPv6">
        {managed ? (
          <span className="settings-value">由网络模式管理 · 关闭</span>
        ) : (
          <Switch
            v={config.ipv6}
            label="切换 IPv6"
            on={(b) => onUpdate("ipv6", b)}
          />
        )}
      </Row>
    </Card>
  );
}
