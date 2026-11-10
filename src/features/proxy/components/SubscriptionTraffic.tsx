import type { TrafficInfo } from "../../../services/api";
import {
  formatTrafficBytes as fmtBytes,
  subscriptionTrafficView,
} from "../services/subscriptionTraffic";

/** 订阅页面与快捷面板共用服务商提供的流量和到期信息。 */
export default function SubscriptionTraffic({
  traffic,
  compact = false,
  manual = false,
}: {
  traffic: TrafficInfo;
  compact?: boolean;
  /** 数字来自手填额度：没有总量时说明「不限量」而不是「未提供」。 */
  manual?: boolean;
}) {
  const { known, used, total, expire, remaining, pct } =
    subscriptionTrafficView(traffic);
  if (!known && expire <= 0)
    return compact ? (
      <p className="mt-1.5 text-[10px] text-zinc-500">
        {manual ? "不限量 · 手填额度" : "订阅未提供额度信息"}
      </p>
    ) : null;
  const barColor =
    pct >= 90 ? "bg-red-500" : pct >= 75 ? "bg-amber-500" : "bg-emerald-500";
  const daysLeft =
    expire > 0 ? Math.ceil((expire * 1000 - Date.now()) / 86400000) : null;
  const usage = `已用 ${fmtBytes(used)} / ${fmtBytes(total)}（${pct}%）`;

  return (
    <div
      className={compact ? "mt-2" : "mt-2.5 border-t border-zinc-800/70 pt-2.5"}
    >
      {known && (
        <>
          <div className="flex items-center justify-between gap-2 text-[10px] text-zinc-500">
            {compact ? (
              <>
                <span
                  className={
                    "text-[11px] font-medium " +
                    (pct >= 90 ? "text-red-400" : "text-zinc-200")
                  }
                >
                  剩余 {fmtBytes(remaining)}
                </span>
                <span>已用 {pct}%</span>
              </>
            ) : (
              <>
                <span>{usage}</span>
                <span className={pct >= 90 ? "font-medium text-red-400" : ""}>
                  剩余 {fmtBytes(remaining)}
                </span>
              </>
            )}
          </div>
          <div
            role="progressbar"
            aria-label="订阅流量已用比例"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={pct}
            aria-valuetext={`${usage}，剩余 ${fmtBytes(remaining)}`}
            data-tooltip={usage}
            className="mt-1 h-1.5 overflow-hidden rounded-full bg-zinc-800"
          >
            <div
              className={`subscription-traffic-fill h-full rounded-full ${barColor}`}
              style={{ width: `${pct}%` }}
            />
          </div>
          {compact && (
            <p className="mt-1 text-[10px] text-zinc-500">
              已用 {fmtBytes(used)} / {fmtBytes(total)}
            </p>
          )}
        </>
      )}
      {expire > 0 && (
        <div className="mt-1.5 text-[10px] text-zinc-600">
          到期：{new Date(expire * 1000).toLocaleDateString("zh-CN")}
          {daysLeft != null && (
            <span className={daysLeft <= 7 ? "text-red-400" : "text-zinc-600"}>
              {daysLeft <= 0 ? "（已过期）" : `（剩余 ${daysLeft} 天）`}
            </span>
          )}
        </div>
      )}
    </div>
  );
}
