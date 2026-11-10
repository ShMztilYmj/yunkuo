import type { ProfileItem, TrafficInfo } from "../../../services/api";

/** 字节数缩写；`gbDigits` 让流量卡与订阅卡各保留原有精度。 */
export function formatTrafficBytes(bytes: number, gbDigits = 1): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(gbDigits)} GB`;
}

/**
 * 服务商上报优先，没有时退回用户手填的额度。
 *
 * `manual = true` 表示这组数字是手填的：总量与到期真实，已用量无从得知，
 * 所以调用方不能拿它算百分比，只能显示剩余与到期。
 * `expire = 0` 表示不到期，`total = 0` 表示不限量——两者可以同时成立。
 */
export function profileTraffic(
  profile: Pick<ProfileItem, "traffic" | "quota">,
): { traffic: TrafficInfo | null; manual: boolean } {
  if (profile.traffic) return { traffic: profile.traffic, manual: false };
  const quota = profile.quota;
  if (!quota) return { traffic: null, manual: false };
  // 早期数据用 total=0 表示不限量，没有 unlimited 字段；两种写法都要认。
  const unlimited = quota.unlimited || quota.total <= 0;
  return {
    traffic: {
      upload: 0,
      download: 0,
      total: unlimited ? 0 : Math.max(0, quota.total),
      expire: Math.max(0, quota.expire),
    },
    manual: true,
  };
}

/** total=0 或缺少有效计数时，不能推断为零剩余或无限流量。 */
export function subscriptionTrafficView(traffic: TrafficInfo) {
  const validBytes = (value: number) => Number.isFinite(value) && value >= 0;
  const sum = traffic.upload + traffic.download;
  const known =
    [traffic.upload, traffic.download, traffic.total, sum].every(validBytes) &&
    traffic.total > 0;
  const used = known ? sum : 0;
  return {
    known,
    used,
    total: traffic.total,
    remaining: known ? Math.max(0, traffic.total - used) : 0,
    pct: known ? Math.min(100, Math.round((used / traffic.total) * 100)) : 0,
    expire:
      Number.isFinite(traffic.expire) && traffic.expire > 0
        ? traffic.expire
        : 0,
  };
}
