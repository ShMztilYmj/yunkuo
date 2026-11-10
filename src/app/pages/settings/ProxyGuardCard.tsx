import { useAsyncAction } from "../../../shared/hooks/useAsyncAction";
import { proxyConfiguration } from "../../../features/proxy/configuration";
import { ShieldCheck } from "lucide-react";
import type { AppConfig } from "../../../services/api";
import Card from "./Card";
import Note from "./Note";
import Row from "./Row";
import Switch from "./Switch";
import { useProxyStore } from "../../../features/proxy/state";
import type { SettingsUpdater } from "./types";

/** 系统代理与守护：系统代理开关、内核退出还原、绕过列表、TUN 模式。 */
export default function ProxyGuardCard({
  config,
}: {
  config: AppConfig;
  onUpdate: SettingsUpdater;
}) {
  const core = useProxyStore((s) => s.coreStatus);
  const coreError = useProxyStore((s) => s.coreError);
  const { busy, error, run: change } = useAsyncAction();
  const managed = config.network_mode != null;
  const sysOn = core?.system_proxy === true;
  const tunOn = core?.tun === true;
  const known =
    typeof core?.system_proxy === "boolean" && typeof core?.tun === "boolean";
  const badge = managed
    ? "网络方案托管"
    : !known
      ? "状态未知"
      : sysOn && tunOn
        ? "完全接管"
        : sysOn || tunOn
          ? "部分接管"
          : "未接管";
  const tone = managed
    ? "warn"
    : !known
      ? "idle"
      : sysOn && tunOn
        ? "ok"
        : sysOn || tunOn
          ? "warn"
          : "idle";

  return (
    <Card
      title="系统代理与守护"
      icon={<ShieldCheck />}
      description="接管 Windows 系统代理与虚拟网卡，并在内核异常退出时兜底还原。"
      badge={badge}
      badgeTone={tone}
    >
      <Row label="系统代理" hint={`当前指向 127.0.0.1:${config.mixed_port}`}>
        <Switch
          v={sysOn}
          label="切换系统代理"
          disabled={busy || managed || typeof core?.system_proxy !== "boolean"}
          on={(b) => void change(() => proxyConfiguration.setSystemProxy(b))}
        />
      </Row>
      {/* 内核退出由进程句柄事件触发，不再是周期检查；这个开关只决定
          内核异常退出时要不要把系统代理还原回去。 */}
      <Row
        label="异常退出时还原系统代理"
        hint="内核崩溃或退出时用备份恢复，避免断网"
      >
        <Switch
          v={config.enable_proxy_guard}
          label="切换内核退出还原"
          disabled={busy}
          on={(b) =>
            void change(() =>
              proxyConfiguration.patch({ enable_proxy_guard: b }),
            )
          }
        />
      </Row>
      <Row label="TUN 模式" hint="虚拟网卡全局接管；首次开启会准备 TUN 服务">
        <Switch
          v={tunOn}
          label="切换 TUN 模式"
          disabled={busy || managed || typeof core?.tun !== "boolean"}
          on={(b) => void change(() => proxyConfiguration.setTun(b))}
        />
      </Row>
      <Row
        label="绕过列表"
        hint="分号分隔的地址或域名，匹配的流量不经过代理"
        wide
      >
        <input
          className="settings-input is-mono"
          aria-label="代理绕过列表"
          key={`bypass-${config.proxy_bypass}`}
          defaultValue={config.proxy_bypass}
          spellCheck={false}
          autoComplete="off"
          disabled={busy}
          onBlur={(e) => {
            const input = e.currentTarget;
            const value = input.value;
            if (value !== config.proxy_bypass) {
              void change(() =>
                proxyConfiguration.patch({ proxy_bypass: value }),
              ).then(() => {
                input.value =
                  useProxyStore.getState().config?.proxy_bypass ??
                  config.proxy_bypass;
              });
            }
          }}
        />
      </Row>
      {managed && (
        <Note>
          接管开关由网络方案管理。要手动调整系统代理或
          TUN，请先在代理中心应用自定义方案。
        </Note>
      )}
      {!managed &&
        (typeof core?.system_proxy !== "boolean" ||
          typeof core?.tun !== "boolean") && (
          <Note tone="warn">
            {coreError || "代理实际状态尚未确认，请刷新后再调整接管开关。"}
          </Note>
        )}
      {error && (
        <Note tone="error" role="alert">
          {error}
        </Note>
      )}
    </Card>
  );
}
