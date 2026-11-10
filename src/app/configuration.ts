import type { AppConfig } from "../services/api/config";
import { configurationDomain } from "../shared/state/configuration";
import {
  PROXY_PATCH_FIELDS,
  proxyConfiguration,
} from "../features/proxy/configuration";
import {
  DIAGNOSTICS_PATCH_FIELDS,
  diagnosticsConfiguration,
} from "../features/diagnostics/configuration";
import { AI_PATCH_FIELDS, aiConfiguration } from "../features/ai/configuration";
import { useAppStore } from "../store";

const DESKTOP_FIELDS = [
  "enable_auto_launch",
  "enable_silent_start",
  "close_to_tray",
] as const;
const desktop = configurationDomain(DESKTOP_FIELDS, DESKTOP_FIELDS);
export const desktopConfiguration = {
  view: desktop.view,
  patch: (patch: Parameters<typeof desktop.patch>[0]) =>
    useAppStore.getState().patch(desktop.patch(patch)),
};

type EditableKey =
  | (typeof PROXY_PATCH_FIELDS)[number]
  | (typeof DIAGNOSTICS_PATCH_FIELDS)[number]
  | (typeof AI_PATCH_FIELDS)[number]
  | (typeof DESKTOP_FIELDS)[number];
export type SettingsUpdater = <Key extends EditableKey>(
  key: Key,
  value: AppConfig[Key],
) => Promise<void>;

/** 设置页只负责装配，写操作仍经过所属模块的白名单和同一配置队列。 */
export const updateSetting: SettingsUpdater = (key, value) => {
  const patch = { [key]: value };
  if ((PROXY_PATCH_FIELDS as readonly string[]).includes(key))
    return proxyConfiguration.patch(patch);
  if ((DIAGNOSTICS_PATCH_FIELDS as readonly string[]).includes(key))
    return diagnosticsConfiguration.patch(patch);
  if ((AI_PATCH_FIELDS as readonly string[]).includes(key))
    return aiConfiguration.patch(patch);
  return desktopConfiguration.patch(patch);
};
