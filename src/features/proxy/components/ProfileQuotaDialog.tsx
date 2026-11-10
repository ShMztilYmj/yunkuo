import { useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { CalendarClock, Loader2, X } from "lucide-react";
import { api, type ProfileItem } from "../../../services/api";
import DatePicker, { type DatePreset } from "../../../shared/ui/DatePicker";

const GB = 1024 ** 3;

/** 套餐到期基本是「从今天起 N 个月」，给几个常用档位，省去翻日历。 */
const EXPIRY_PRESETS: DatePreset[] = [
  { label: "1 个月", months: 1 },
  { label: "3 个月", months: 3 },
  { label: "半年", months: 6 },
  { label: "1 年", months: 12 },
];

/** 距今天还有多少天：正数未到期、0 今天、负数已过期。 */
function daysUntil(value: string): number | null {
  if (!value) return null;
  const at = new Date(`${value}T00:00:00`).getTime();
  if (Number.isNaN(at)) return null;
  const now = new Date();
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  return Math.round((at - today.getTime()) / 86_400_000);
}

/** 字节 → 输入框里的 GB 数字，最多两位小数并去掉尾随 0。 */
function bytesToGb(bytes: number) {
  return Number((bytes / GB).toFixed(2));
}

/** `YYYY-MM-DD` → 当天 23:59:59（本地时区）的 Unix 秒；空值返回 0，非法返回 null。 */
function dateToExpire(value: string): number | null {
  if (!value) return 0;
  const at = new Date(`${value}T23:59:59`).getTime();
  return Number.isNaN(at) ? null : Math.floor(at / 1000);
}

/** Unix 秒 → 日期选择器需要的 `YYYY-MM-DD`（本地时区）。 */
function expireToDate(expire: number) {
  if (!(expire > 0)) return "";
  const date = new Date(expire * 1000);
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(
    date.getDate(),
  )}`;
}

export interface ProfileQuotaDialogProps {
  profile: ProfileItem;
  onCancel: () => void;
  /** 保存成功后回调，由调用方重拉列表；弹窗自己关闭。 */
  onSaved: () => void;
}

/**
 * 编辑配置：改名称，并在服务商没有上报额度时手填总量与到期。
 *
 * 本地导入和未返回 `subscription-userinfo` 的订阅拿不到任何额度，
 * 卡片此前只能说「没有额度」；用户自己知道套餐是多少，这里让他们补上。
 * 服务商上报的数值始终优先，手填值只在缺数据时兜底，所以不提供「覆盖」。
 */
export default function ProfileQuotaDialog({
  profile,
  onCancel,
  onSaved,
}: ProfileQuotaDialogProps) {
  const dialog = useRef<HTMLDialogElement>(null);
  const nameInput = useRef<HTMLInputElement>(null);
  const titleId = useId();
  const formId = useId();

  const hasQuota = Boolean(profile.quota);
  const currentTotal = profile.quota?.total ?? 0;
  const currentExpire = profile.quota?.expire ?? 0;
  // 早期数据用 total = 0 表示不限量；没有额度时不能沿用这个默认值，
  // 否则「不限量 + 不到期」会被判成没改动而不落盘。
  const currentUnlimited =
    hasQuota && (Boolean(profile.quota?.unlimited) || currentTotal <= 0);

  const [name, setName] = useState(profile.name);
  const [unlimited, setUnlimited] = useState(currentUnlimited);
  // 已有额度且没写到期 → 视为不到期；第一次填的配置默认留空，让用户显式选一边。
  const [perpetual, setPerpetual] = useState(hasQuota && currentExpire === 0);
  const [totalGb, setTotalGb] = useState(
    currentTotal > 0 ? String(bytesToGb(currentTotal)) : "",
  );
  const [expireInput, setExpireInput] = useState(expireToDate(currentExpire));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useLayoutEffect(() => {
    const node = dialog.current;
    if (!node) return;
    if (!node.open) node.showModal();
    nameInput.current?.focus({ preventScroll: true });
    nameInput.current?.select();
    return () => {
      if (node.open) node.close();
    };
  }, []);

  const dismiss = () => {
    if (!busy) onCancel();
  };

  const save = async () => {
    if (busy) return;
    const trimmed = name.trim();
    if (!trimmed) {
      setError("名称不能为空");
      return;
    }
    if (trimmed.length > 60) {
      setError("名称最多 60 个字符");
      return;
    }
    const gb = Number(totalGb);
    if (!unlimited && (!Number.isFinite(gb) || gb <= 0)) {
      setError("请填写总量（GB），或勾选不限量");
      return;
    }
    if (!perpetual && !expireInput) {
      setError("请选择到期日，或勾选「不到期」");
      return;
    }
    const expireAt = perpetual ? 0 : dateToExpire(expireInput);
    if (expireAt === null) {
      setError("到期日期格式不正确");
      return;
    }

    const next = {
      unlimited,
      total: unlimited ? 0 : Math.round(gb * GB),
      expire: expireAt,
    };
    const quotaChanged =
      next.unlimited !== currentUnlimited ||
      next.total !== currentTotal ||
      next.expire !== currentExpire;
    const nameChanged = trimmed !== profile.name;

    setBusy(true);
    setError(null);
    try {
      if (quotaChanged) {
        await api.setProfileQuota(profile.uid, next);
      }
      if (nameChanged) {
        await api.patchProfile(profile.uid, trimmed, undefined);
      }
      onSaved();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const clearQuota = async () => {
    if (busy || !profile.quota) return;
    setBusy(true);
    setError(null);
    try {
      await api.setProfileQuota(profile.uid, null);
      onSaved();
    } catch (reason) {
      setError(String(reason));
      setBusy(false);
    }
  };

  // 只有服务商真的给了总量，手填值才永远用不上；total=0 的订阅仍可补一个。
  const providerQuota = Boolean(profile.traffic && profile.traffic.total > 0);

  // 到期日旁边补一句人话：还剩几天 / 已过期，省得用户自己数。
  const remaining = perpetual ? null : daysUntil(expireInput);
  const expiryNote =
    remaining === null
      ? null
      : remaining < 0
        ? { tone: "danger" as const, text: `已过期 ${-remaining} 天` }
        : remaining === 0
          ? { tone: "warn" as const, text: "今天到期" }
          : {
              tone: remaining <= 14 ? ("warn" as const) : ("muted" as const),
              text: `还剩 ${remaining} 天`,
            };

  return createPortal(
    <dialog
      ref={dialog}
      className="confirm-dialog is-neutral profile-quota-dialog"
      aria-labelledby={titleId}
      onCancel={(event) => {
        event.preventDefault();
        dismiss();
      }}
      onClick={(event) => {
        if (event.target === event.currentTarget) dismiss();
      }}
    >
      <div className="confirm-card" role="document">
        <header className="confirm-head">
          <span className="confirm-icon" aria-hidden="true">
            <CalendarClock size={16} />
          </span>
          <div className="confirm-heading">
            <span className="confirm-eyebrow">订阅管理</span>
            <h2 id={titleId}>编辑配置</h2>
          </div>
          <button
            type="button"
            className="confirm-close"
            aria-label="关闭"
            disabled={busy}
            onClick={dismiss}
          >
            <X size={14} />
          </button>
        </header>

        <p className="confirm-desc">
          {providerQuota
            ? "这条订阅由服务商上报额度，下面的额度只在拿不到数据时才会用到。"
            : "这条配置没有额度数据。填上套餐总量与到期日，卡片就能显示剩余时间与剩余流量。"}
        </p>

        <form
          id={formId}
          className="profile-add-form"
          onSubmit={(event) => {
            event.preventDefault();
            void save();
          }}
        >
          <label className="profile-quota-field">
            <span>名称</span>
            <input
              ref={nameInput}
              value={name}
              maxLength={60}
              autoComplete="off"
              placeholder="例如：住宅IP 210G"
              aria-label="配置名称"
              onChange={(event) => setName(event.target.value)}
            />
          </label>

          <div className="profile-add-row">
            <label className="profile-quota-field profile-add-grow">
              <span>总量（GB）</span>
              <input
                value={unlimited ? "" : totalGb}
                disabled={unlimited}
                inputMode="decimal"
                autoComplete="off"
                placeholder="例如 210"
                aria-label="套餐总量（GB）"
                onChange={(event) => setTotalGb(event.target.value)}
              />
            </label>
            <div className="profile-quota-field profile-add-grow">
              <span>到期日</span>
              <DatePicker
                value={perpetual ? "" : expireInput}
                disabled={perpetual}
                ariaLabel="到期日"
                placeholder={perpetual ? "不到期" : "选择日期"}
                presets={EXPIRY_PRESETS}
                dimPast
                onChange={(next) => {
                  setExpireInput(next);
                  if (next) setPerpetual(false);
                }}
              />
            </div>
          </div>

          <div className="profile-quota-checks">
            <label className="profile-quota-check">
              <input
                type="checkbox"
                checked={unlimited}
                onChange={(event) => setUnlimited(event.target.checked)}
              />
              <span>不限量套餐</span>
            </label>
            <label className="profile-quota-check">
              <input
                type="checkbox"
                checked={perpetual}
                onChange={(event) => {
                  setPerpetual(event.target.checked);
                  if (event.target.checked) setExpireInput("");
                }}
              />
              <span>不到期</span>
            </label>
            {expiryNote && (
              <span
                className="profile-quota-expire-note"
                data-tone={expiryNote.tone}
              >
                {expiryNote.text}
              </span>
            )}
          </div>

          <p className="profile-add-hint">
            {providerQuota
              ? "服务商上报的数值优先，手填的额度不会覆盖它。"
              : "客户端看不到真实用量，这里只用于显示「还剩多少」；本月已用仍以服务商后台为准。"}
          </p>
        </form>

        {error && (
          <p className="confirm-error" role="alert">
            {error}
          </p>
        )}

        <footer className="confirm-actions">
          <span className="confirm-hint">
            {hasQuota ? (
              <button
                type="button"
                className="profile-quota-clear"
                disabled={busy}
                onClick={() => void clearQuota()}
              >
                清除额度
              </button>
            ) : (
              <>
                <kbd>Esc</kbd> 取消
              </>
            )}
          </span>
          <div className="confirm-buttons">
            <button
              type="button"
              className="confirm-btn is-plain"
              disabled={busy}
              onClick={dismiss}
            >
              取消
            </button>
            <button
              type="submit"
              form={formId}
              className="confirm-btn is-solid"
              disabled={busy}
            >
              {busy ? (
                <Loader2 className="confirm-spin" size={13} />
              ) : (
                <CalendarClock size={13} />
              )}
              {busy ? "正在保存…" : "保存"}
            </button>
          </div>
        </footer>
      </div>
    </dialog>,
    document.body,
  );
}
