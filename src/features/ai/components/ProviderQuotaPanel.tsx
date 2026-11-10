import { quotaResetText } from "../services/aiStats";
import {
  balanceLabel,
  formatAmount,
  quotaSummary,
  quotaSyncText,
  quotaTone,
  type ProviderBilling,
  type ProviderQuotaView,
} from "../services/providerAccounts";

/**
 * 额度展示的唯一组件：订阅制给窗口读数与进度条，按量付费给余额。
 *
 * 服务商页的详情卡片、悬浮条与工具卡共用这一份，样式只在 provider-accounts.css
 * 里定义，两处不会再出现「同一个额度长得不一样」。`compact` 只换排版：
 * 工具卡上一行一个窗口、读数改成「剩余」，服务商落款压在最后一行。
 */
export default function ProviderQuotaPanel({
  quota,
  title,
  badge,
  readAt = null,
  compact = false,
}: {
  quota: ProviderQuotaView;
  /** 悬浮条与工具卡没有外层身份行，需要自己标出服务商名。 */
  title?: string;
  /** 当前正在使用的服务商标记。 */
  badge?: string;
  readAt?: string | null;
  /** 工具卡上的紧凑形态：窗口一行一条，读数只留「剩余」。 */
  compact?: boolean;
}) {
  const now = Date.now() / 1000;
  if (compact)
    return (
      <div className="provider-quota is-compact" aria-label="额度">
        {quota.billing === "balance" ? (
          <CompactBalance quota={quota} />
        ) : quota.windows.length ? (
          <div className="provider-quota-windows">
            {quota.windows.map((window, index) => {
              // 非有限读数按 0 画条：宁可空条，也不输出 NaN 宽度。
              const percent = Number.isFinite(window.usedPercent)
                ? window.usedPercent
                : 0;
              const used = Math.min(100, Math.max(0, percent));
              // 账号快照超过半小时就没有参考价值，读数退回「等更新」而不是假的新数。
              const stale =
                quota.from === "account" &&
                (!quota.capturedAt || now - quota.capturedAt > 1800);
              const tone = stale ? "muted" : quotaTone(percent);
              const remaining = Math.round(
                window.remainingPercent ?? Math.max(0, 100 - percent),
              );
              const value = stale
                ? "快照已过期"
                : percent > 100
                  ? "已超额"
                  : `剩余 ${remaining}%`;
              return (
                <div
                  className="provider-quota-row"
                  key={`${window.label}:${index}`}
                  data-tone={tone}
                >
                  <div className="provider-quota-compact-line">
                    <span
                      className="provider-quota-compact-label"
                      title={window.label}
                    >
                      {window.label}
                    </span>
                    <strong
                      className="provider-quota-compact-value"
                      data-tone={tone}
                    >
                      {value}
                    </strong>
                  </div>
                  <div
                    className="provider-quota-track"
                    role="progressbar"
                    aria-label={`${window.label}已用额度`}
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-valuenow={Math.round(used)}
                    aria-valuetext={value}
                  >
                    <i data-tone={tone} style={{ width: `${used}%` }} />
                  </div>
                </div>
              );
            })}
          </div>
        ) : (
          <p className="provider-quota-compact-empty">
            {compactEmptyText(quota)}
          </p>
        )}
        {title && (
          <QuotaProviderLine
            provider={title}
            badge={badge}
            hint={quotaSyncText(quota, readAt)}
          />
        )}
      </div>
    );
  return (
    <div className="provider-quota" aria-label="额度">
      {title && (
        <div className="provider-quota-head">
          <strong>{title}</strong>
          {badge && <em className="provider-quota-badge">{badge}</em>}
          <em className="provider-quota-mode">{billingLabel(quota.billing)}</em>
        </div>
      )}
      {quota.billing === "balance" ? (
        // 按量付费没有窗口，只显示余额：这是「还能用多少钱」，不是百分比。
        <div className="provider-quota-balance-block">
          {quota.balances.length ? (
            <dl className="provider-quota-amounts">
              {quota.balances.map((balance) => (
                <div key={balance.label}>
                  <dt>{balanceLabel(balance.label)}</dt>
                  <dd>{formatAmount(balance.amount, balance.currency)}</dd>
                </div>
              ))}
            </dl>
          ) : (
            <p className="provider-quota-empty">
              {quota.capturedAt
                ? "余额暂不可读 · 请检查官方账号登录"
                : "余额未读取 · 登录官方账号后同步"}
            </p>
          )}
        </div>
      ) : (
        <>
          <div className="provider-quota-windows">
            {quota.windows.map((window, index) => {
              const percent = window.usedPercent;
              const tone = quotaTone(percent);
              const shown = Math.min(100, Math.max(0, Math.round(percent)));
              const reset = window.resetsAt
                ? resetLabel(window.resetsAt - now)
                : null;
              return (
                <div
                  className="provider-quota-row"
                  key={`${window.label}:${index}`}
                  data-tone={tone}
                >
                  <div className="provider-quota-line">
                    <span className="provider-quota-label">{window.label}</span>
                    <strong className="provider-quota-value" data-tone={tone}>
                      {shown}
                      <small>%</small>
                    </strong>
                    <span className="provider-quota-unit">已用</span>
                    {reset && (
                      <span className="provider-quota-reset">{reset}</span>
                    )}
                  </div>
                  <div
                    className="provider-quota-track"
                    role="progressbar"
                    aria-label={`${window.label}已用额度`}
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-valuenow={shown}
                  >
                    <i data-tone={tone} style={{ width: `${shown}%` }} />
                  </div>
                </div>
              );
            })}
          </div>
          {quota.billing === "plan" ? (
            <p className="provider-quota-empty">
              {quota.status === "ok"
                ? "套餐已开通 · 用量由服务商缓存"
                : quota.status === "unconfigured"
                  ? "套餐未开通或状态未知"
                  : "套餐状态未知"}
            </p>
          ) : quota.billing === "unsupported" && !quota.windows.length ? (
            <p className="provider-quota-empty">该来源暂不支持额度查询</p>
          ) : (
            !quota.windows.length && (
              <p className="provider-quota-empty">
                {quota.from === "account" ? "暂无额度快照" : "暂无额度读数"}
              </p>
            )
          )}
        </>
      )}
      <p className="provider-quota-sync">{quotaSyncText(quota, readAt)}</p>
    </div>
  );
}

/**
 * 服务商落款：说明这组读数属于哪一家、这家算不算「使用中」。
 *
 * 工具卡的紧凑额度条与「还没有读数」的空状态共用这一行，保证两种状态的
 * 位置、字号和徽标完全一致。
 */
export function QuotaProviderLine({
  provider,
  badge,
  note,
  hint,
}: {
  provider: string;
  badge?: string;
  /** 没有读数时的短说明，如「暂无额度快照」。 */
  note?: string;
  /** 悬停补全：同步状态、采集时刻等不该占版的细节。 */
  hint?: string;
}) {
  return (
    <p className="provider-quota-caption" data-tooltip={hint || undefined}>
      <span className="provider-quota-caption-provider">{provider}</span>
      {badge && <em className="provider-quota-badge">{badge}</em>}
      {note && <span className="provider-quota-caption-note">{note}</span>}
    </p>
  );
}

/** 按量付费的紧凑形态：余额本身就是一句话，不必再排成表格。 */
function CompactBalance({ quota }: { quota: ProviderQuotaView }) {
  return (
    <p className="provider-quota-compact-balance">
      {quotaSummary(quota) ??
        (quota.capturedAt ? "余额暂不可读" : "余额未读取")}
    </p>
  );
}

/** 紧凑形态的空读数措辞：和详情面板同源，避免两处说法不一样。 */
function compactEmptyText(quota: ProviderQuotaView): string {
  if (quota.billing === "plan") return "套餐已开通 · 用量由服务商缓存";
  if (quota.billing === "unsupported") return "该来源暂不支持额度查询";
  return quota.from === "account" ? "暂无额度快照" : "暂无额度读数";
}

/** 计费方式标签：悬浮条标题行给一眼可辨的类型。 */
function billingLabel(billing: ProviderBilling): string {
  return billing === "balance"
    ? "按量付费"
    : billing === "plan"
      ? "套餐"
      : billing === "subscription"
        ? "订阅"
        : "未接入";
}

/** 重置倒计时：超过一天的窗口按天说，短窗口沿用紧凑的「h m」写法。 */
function resetLabel(seconds: number): string {
  const remaining = Math.max(0, seconds);
  if (remaining <= 0) return quotaResetText(0, true, "short");
  if (remaining >= 86400) return `${Math.ceil(remaining / 86400)} 天后重置`;
  return quotaResetText(remaining, false, "short");
}
