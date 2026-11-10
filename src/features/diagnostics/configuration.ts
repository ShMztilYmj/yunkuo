import { configurationDomain } from "../../shared/state/configuration";
import { useAppStore } from "../../store";

export const DIAGNOSTICS_CONFIG_FIELDS = [
  "tz_auto_sync",
  "tz_restore_on_proxy_off",
  "gate_threshold",
  "require_claude_reachable",
  "blocked_countries",
  "ipapiis_key",
  "last_tz_sync",
  // 只读：判断 IPv6 修复项该不该交给内核开关。
  "network_mode",
] as const;
export const DIAGNOSTICS_PATCH_FIELDS = [
  "tz_auto_sync",
  "tz_restore_on_proxy_off",
  "gate_threshold",
  "require_claude_reachable",
  "blocked_countries",
  "ipapiis_key",
] as const;
const domain = configurationDomain(
  DIAGNOSTICS_CONFIG_FIELDS,
  DIAGNOSTICS_PATCH_FIELDS,
);
export type DiagnosticsConfiguration = NonNullable<
  ReturnType<typeof domain.view>
>;
const selectConfiguration = (state: {
  config: Parameters<typeof domain.view>[0];
}) => state.config;
/** 页面读取配置的只读视图；投影同一个 store，不复制状态。 */
export function useDiagnosticsConfiguration(): DiagnosticsConfiguration | null {
  return domain.view(useAppStore(selectConfiguration));
}
export const diagnosticsConfiguration = {
  view: domain.view,
  patch: (patch: Parameters<typeof domain.patch>[0]) =>
    useAppStore.getState().patch(domain.patch(patch)),
};
