import { call } from "./invoke";

// ---------- 维护：诊断 / 升级 / 服务安装 / 默认浏览器 ----------

export interface MaintenanceStatus {
  appVersion: string;
  kernelVersion: string | null;
  serviceVersion: string | null;
  coreRunning: boolean;
  privacyArmed: boolean | null;
  privacyHealthy: boolean;
  hasProfile: boolean;
  profileCount: number;
  shortcutConfigured: boolean;
  browserAvailable: boolean;
  canUndoUpgrade: boolean;
}

export interface InstallerInfo {
  sha256: string;
  fileName: string;
  details: { version: string; signature: string; bytes: number };
}

export interface ServiceStatusInfo {
  installed: boolean;
  running: boolean;
  coreRunning: boolean;
  pid: number;
  compatible: boolean;
  error: string | null;
}

export interface DefaultBrowserStatus {
  /// HTTP/HTTPS 当前关联，分别显示避免部分切换被掩盖
  current: string | null;
  https: string | null;
  /// 是否保存了可还原的原始默认关联
  switched: boolean;
}

export const maintenanceApi = {
  defaultBrowserStatus: () =>
    call<DefaultBrowserStatus>("default_browser_status"),

  maintenanceStatus: () => call<MaintenanceStatus>("maintenance_status"),
  exportDiagnostics: (path: string) =>
    call<void>("export_diagnostics", { path }),
  restoreConfigBackup: (
    path: string | null,
    password: string,
    undo = false,
    beforeUpgrade = false,
  ) =>
    call<void>("restore_config_backup", {
      path,
      password,
      undo,
      beforeUpgrade,
    }),
  inspectInstaller: (path: string, sha256: string) =>
    call<InstallerInfo>("inspect_installer", { path, sha256 }),
  launchVerifiedInstaller: (path: string, sha256: string) =>
    call<void>("launch_verified_installer", { path, sha256 }),

  serviceStatus: () => call<ServiceStatusInfo>("service_status"),
  serviceInstall: () => call<void>("service_install"),
  serviceUninstall: () => call<void>("service_uninstall"),
};
