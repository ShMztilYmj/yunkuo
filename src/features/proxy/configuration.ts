import { networkApi } from "../../services/api/network";
import { configApi } from "../../services/api/config";
import { coreApi } from "../../services/api/core";
import type { NetworkMode, RoutingMode } from "../../services/api/config";
import { configurationDomain } from "../../shared/state/configuration";
import { useAppStore } from "../../store";

export const PROXY_CONFIG_FIELDS = [
  "proxy_engine",
  "mixed_port",
  "external_controller_port",
  "secret",
  "enable_system_proxy",
  "enable_proxy_guard",
  "proxy_guard_interval",
  "proxy_bypass",
  "enable_tun",
  "mode",
  "network_mode",
  "daily_proxy",
  "log_level",
  "allow_lan",
  "ipv6",
] as const;
export const PROXY_PATCH_FIELDS = [
  "proxy_engine",
  "mixed_port",
  "external_controller_port",
  "enable_system_proxy",
  "enable_proxy_guard",
  "proxy_guard_interval",
  "proxy_bypass",
  "enable_tun",
  "mode",
  "log_level",
  "allow_lan",
  "ipv6",
] as const;
const refreshRuntime = async (operation: () => Promise<void>) => {
  try {
    // mutateConfig 已发布命令返回的权威配置，成功后无需再读一次磁盘配置。
    await operation();
  } catch (error) {
    // 例如严格保护重新启用失败：配置可能已提交，失败时回读一次。
    await useAppStore
      .getState()
      .refreshConfig()
      .catch(() => undefined);
    throw error;
  } finally {
    await useAppStore
      .getState()
      .refreshCore(true)
      .catch(() => undefined);
  }
};
const domain = configurationDomain(PROXY_CONFIG_FIELDS, PROXY_PATCH_FIELDS);
export type ProxyConfiguration = NonNullable<ReturnType<typeof domain.view>>;
export type ProxyConfigurationPatch = Parameters<typeof domain.patch>[0];

/** 网络模式必须通过对应操作切换；secret只能读取，不作为通用patch字段。 */
export const proxyConfiguration = {
  view: domain.view,
  patch: (patch: ProxyConfigurationPatch) => {
    // 字段白名单在进入异步队列前校验：越界字段直接抛出，不会排进写队列。
    const validated = domain.patch(patch);
    return refreshRuntime(() => useAppStore.getState().patch(validated));
  },
  setSystemProxy: (enable: boolean) =>
    refreshRuntime(() =>
      useAppStore.getState().mutateConfig(() => networkApi.setSysproxy(enable)),
    ),
  setTun: (enable: boolean) =>
    refreshRuntime(() =>
      useAppStore.getState().mutateConfig(() => networkApi.setTun(enable)),
    ),
  /** 重启内核：按当前配置重建会话，连接会短暂中断；不改变任何持久化配置。 */
  restartCore: () => refreshRuntime(() => coreApi.coreRestart()),
  setNetworkMode: (mode: NetworkMode, allowRelease: boolean) =>
    refreshRuntime(() =>
      useAppStore
        .getState()
        .mutateConfig(() => networkApi.setNetworkMode(mode, allowRelease)),
    ),
  setRoutingMode: (mode: RoutingMode, allowRelease: boolean) =>
    refreshRuntime(() =>
      useAppStore
        .getState()
        .mutateConfig(() => networkApi.setRoutingMode(mode, allowRelease)),
    ),
  restoreNetworkMode: (allowRelease: boolean) =>
    refreshRuntime(() =>
      useAppStore
        .getState()
        .mutateConfig(() => networkApi.restoreNetworkMode(allowRelease)),
    ),
  releaseProtection: () =>
    refreshRuntime(() =>
      useAppStore.getState().mutateConfig(async () => {
        await networkApi.privacyRelease();
        return configApi.getConfig();
      }),
    ),
};
