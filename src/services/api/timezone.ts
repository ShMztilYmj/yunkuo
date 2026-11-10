import { call } from "./invoke";

// ---------- 时区同步（按出口 IP 自动对齐系统时区） ----------

export interface TzSyncInfo {
  ip: string;
  country: string;
  country_code: string;
  iana_tz: string;
  windows_tz: string;
  at: string;
}

export interface TzEntry {
  key_name: string;
  std_name: string;
  dlt_name: string;
  display: string;
  bias: number;
  std_bias: number;
  dlt_bias: number;
}

export interface TzBackup {
  key_name: string;
  display: string;
  backed_up_at: string;
}

export interface CurrentTz {
  key_name: string;
  display: string;
  iana_guess: string;
  bias_minutes: number;
}

export interface TzStatus {
  current: CurrentTz;
  backup: TzBackup | null;
  last_sync: TzSyncInfo | null;
  available: TzEntry[];
}

export interface TzSyncResult {
  changed: boolean;
  message: string;
  info: TzSyncInfo | null;
}

export const timezoneApi = {
  syncTimezone: () => call<TzSyncResult>("sync_timezone_by_ip"),
  restoreTimezone: () => call<void>("restore_timezone"),
  setTimezoneManual: (name: string) =>
    call<void>("set_timezone_manual", { name }),
  tzStatus: () => call<TzStatus>("get_timezone_status"),
};
