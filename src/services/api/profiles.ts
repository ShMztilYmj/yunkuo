import type { TrafficInfo } from "./core";
import { call } from "./invoke";

// ---------- 订阅 / 配置文件 ----------

/**
 * 手动额度：服务商不上报时由用户自己填。
 *
 * `unlimited` 表示套餐不限量，只看到期；`expire = 0` 表示不到期。
 * 「不限量 + 不到期」是合法组合，所以是否为空不能只看数值。
 * 服务商返回的 `traffic` 始终优先，这里只在没有数据时兜底。
 */
export interface ManualQuota {
  /** 不限量套餐：总量无从填写 */
  unlimited: boolean;
  /** 总量（字节）；unlimited 时忽略 */
  total: number;
  /** Unix 秒，0 = 不到期 */
  expire: number;
}

export interface ProfileItem {
  uid: string;
  type: "remote" | "local";
  name: string;
  url: string;
  file: string;
  updated_at: string | null;
  update_interval: number;
  traffic?: TrafficInfo | null;
  quota?: ManualQuota | null;
}

export const profilesApi = {
  listProfiles: () => call<ProfileItem[]>("list_profiles"),
  currentProfile: () => call<ProfileItem | null>("current_profile"),
  createProfileRedirect: (url: string, name?: string) =>
    call<ProfileItem>("create_profile", {
      url,
      localPath: null,
      name: name ?? null,
    }),
  createProfileLocal: (path: string, name?: string) =>
    call<ProfileItem>("create_profile", {
      url: null,
      localPath: path,
      name: name ?? null,
    }),
  /** 节点串导入：host:port:token 或 socks5://user:pass@host:port */
  createProfileNode: (
    node: string,
    protocol?: "socks5" | "http",
    name?: string,
  ) =>
    call<ProfileItem>("create_profile_node", {
      node,
      protocol: protocol ?? null,
      name: name ?? null,
    }),
  updateProfile: (uid: string) => call<void>("update_profile", { uid }),
  deleteProfile: (uid: string) => call<void>("delete_profile", { uid }),
  patchProfile: (uid: string, name?: string, updateInterval?: number) =>
    call<void>("patch_profile", {
      uid,
      name: name ?? null,
      updateInterval: updateInterval ?? null,
    }),
  /** 写手动额度；传 null 表示清除。 */
  setProfileQuota: (uid: string, quota: ManualQuota | null) =>
    call<void>("set_profile_quota", { uid, quota }),
  selectProfile: (uid: string) => call<void>("select_profile", { uid }),
};
