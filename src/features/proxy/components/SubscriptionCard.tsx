import { useLayoutEffect, useRef, useState } from "react";
import {
  Clock,
  FilePlus2,
  Gauge,
  Link2,
  Pencil,
  RefreshCw,
  Trash2,
} from "lucide-react";
import type { ProfileItem, TrafficInfo } from "../../../services/api";
import {
  formatTrafficBytes as fmtBytes,
  profileTraffic,
  subscriptionTrafficView,
} from "../services/subscriptionTraffic";

/** 订阅链接只露出主机名，完整地址留给 title / tooltip。 */
function endpointLabel(profile: ProfileItem) {
  if (profile.type !== "remote") return "本地导入 / 节点串";
  try {
    return new URL(profile.url).host || profile.url;
  } catch {
    return profile.url.replace(/^https?:\/\//, "").split("/")[0] || profile.url;
  }
}

function expireLabel(expire: number) {
  const date = new Date(expire * 1000);
  const day = `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(
    2,
    "0",
  )}-${String(date.getDate()).padStart(2, "0")}`;
  const days = Math.ceil((expire * 1000 - Date.now()) / 86400000);
  if (days <= 0) return { text: `到期 ${day} · 已过期`, alert: true };
  return {
    text: `到期 ${day} · 剩 ${days} 天`,
    alert: days <= 7,
  };
}

/** 手动额度只显示到期日，不显示剩余天数——卡片空间优先留给名称。 */
function dayLabel(expire: number) {
  const date = new Date(expire * 1000);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(
    2,
    "0",
  )}-${String(date.getDate()).padStart(2, "0")}`;
}

/** 更新时间只留「月-日 时:分」，完整时间放在 title 里。 */
function updatedLabel(value: string | null) {
  if (!value) return "尚未更新";
  const match = value.match(/^(\d{4})-(\d{2})-(\d{2})[T ](\d{2}):(\d{2})/);
  return match
    ? `${match[2]}-${match[3]} ${match[4]}:${match[5]} 更新`
    : `${value} 更新`;
}

/** 服务商上报的额度：一行百分比 + 进度条 + 已用/总量 + 到期。 */
function CardTraffic({ traffic }: { traffic: TrafficInfo }) {
  const { known, used, total, remaining, pct, expire } =
    subscriptionTrafficView(traffic);
  const expiry = expire > 0 ? expireLabel(expire) : null;

  if (!known && !expiry)
    return <p className="proxy-card-quiet">订阅未提供额度信息</p>;

  return (
    <div className="proxy-card-traffic">
      {known && (
        <>
          <div className="proxy-card-traffic-head">
            <span>已用 {pct}%</span>
            <span data-alert={pct >= 90 || undefined}>
              剩余 {fmtBytes(remaining)}
            </span>
          </div>
          <div
            role="progressbar"
            aria-label="订阅流量已用比例"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={pct}
            aria-valuetext={`已用 ${fmtBytes(used)} / ${fmtBytes(total)}（${pct}%），剩余 ${fmtBytes(remaining)}`}
            data-tooltip={`已用 ${fmtBytes(used)} / ${fmtBytes(total)}`}
            className="proxy-card-bar"
            data-tone={pct >= 90 ? "alert" : pct >= 75 ? "warn" : undefined}
          >
            <span style={{ width: `${pct}%` }} />
          </div>
          <p className="proxy-card-traffic-foot">
            {fmtBytes(used)} / {fmtBytes(total)}
          </p>
        </>
      )}
      {expiry && (
        <p className="proxy-card-expire" data-alert={expiry.alert || undefined}>
          {expiry.text}
        </p>
      )}
    </div>
  );
}

export interface SubscriptionCardProps {
  profile: ProfileItem;
  active: boolean;
  busy: boolean;
  onUse: () => void;
  onUpdate: () => void;
  onInterval: (hours: number) => void;
  /** 名称就地改名：只写名称，不打开弹窗。 */
  onRename: (name: string) => Promise<void>;
  /** 打开「编辑配置」：改名称、补额度与到期。 */
  onEdit: () => void;
  onDelete: () => void;
}

/**
 * 一条订阅一张扑克牌大小的卡片：尺寸只由网格列宽与 5:7 比例决定，
 * 内容多少不影响外框，整页卡片始终等大。
 */
export default function SubscriptionCard({
  profile,
  active,
  busy,
  onUse,
  onUpdate,
  onInterval,
  onRename,
  onEdit,
  onDelete,
}: SubscriptionCardProps) {
  const remote = profile.type === "remote";
  const endpoint = endpointLabel(profile);
  // 服务商上报优先；没有时退回用户手填的额度。
  const { traffic, manual } = profileTraffic(profile);
  const expired =
    traffic != null &&
    traffic.expire > 0 &&
    traffic.expire * 1000 <= Date.now();

  /** 改名是就地编辑：点名称直接在卡片里出现输入框，回车/失焦即保存，不弹窗。 */
  const [renaming, setRenaming] = useState(false);
  const [draft, setDraft] = useState(profile.name);
  const [renameBusy, setRenameBusy] = useState(false);
  const [renameError, setRenameError] = useState<string | null>(null);
  const nameInput = useRef<HTMLInputElement>(null);
  /** 本次编辑是否已有结论：失焦与回车/取消都会触发提交，用它去重。 */
  const settled = useRef(true);

  useLayoutEffect(() => {
    if (!renaming) return;
    const node = nameInput.current;
    if (!node) return;
    node.focus({ preventScroll: true });
    node.select();
  }, [renaming]);

  const beginRename = () => {
    if (busy || renameBusy) return;
    settled.current = false;
    setDraft(profile.name);
    setRenameError(null);
    setRenaming(true);
  };

  const stopRename = () => {
    settled.current = true;
    setRenaming(false);
    setRenameError(null);
  };

  /** 失焦即提交：回车与点卡片任意处都走这条路径；校验不过就留在编辑态。 */
  const commitRename = async () => {
    if (settled.current) return;
    const trimmed = draft.trim();
    if (!trimmed || trimmed.length > 60) {
      setRenameError(!trimmed ? "名称不能为空" : "名称最多 60 个字符");
      nameInput.current?.focus({ preventScroll: true });
      return;
    }
    if (trimmed === profile.name) {
      stopRename();
      return;
    }
    settled.current = true;
    setRenameBusy(true);
    setRenameError(null);
    try {
      await onRename(trimmed);
      setRenaming(false);
    } catch (error) {
      setRenaming(false);
      setRenameError(String(error));
    } finally {
      setRenameBusy(false);
    }
  };

  return (
    <article
      className={`proxy-sub-card${active ? " is-active" : ""}`}
      aria-label={profile.name}
    >
      <header className="proxy-card-top">
        <span className="proxy-card-kind">
          <span className="proxy-card-glyph" aria-hidden="true">
            {remote ? <Link2 size={13} /> : <FilePlus2 size={13} />}
          </span>
          {remote ? "订阅" : "本地"}
        </span>
        {active && (
          <span className="proxy-card-live">
            <i aria-hidden="true" />
            使用中
          </span>
        )}
      </header>

      <div
        className="proxy-card-name-row"
        data-editing={renaming || undefined}
        data-saving={renameBusy || undefined}
      >
        {renaming ? (
          <input
            ref={nameInput}
            className="proxy-card-name-input"
            value={draft}
            maxLength={60}
            autoComplete="off"
            spellCheck={false}
            disabled={renameBusy}
            aria-label="配置名称"
            onChange={(event) => {
              setDraft(event.target.value);
              setRenameError(null);
            }}
            onBlur={() => void commitRename()}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                event.currentTarget.blur();
              } else if (event.key === "Escape") {
                event.preventDefault();
                stopRename();
              }
            }}
          />
        ) : (
          <>
            <button
              type="button"
              className="proxy-card-name-hit"
              disabled={busy}
              data-tooltip="点击直接改名"
              aria-label={`重命名 ${profile.name}`}
              onClick={beginRename}
            />
            <h3 className="proxy-card-name" title={profile.name}>
              {profile.name}
            </h3>
            <Pencil
              className="proxy-card-name-pencil"
              size={10}
              aria-hidden="true"
            />
          </>
        )}
      </div>
      {renameError && (
        <p className="proxy-card-name-error" role="alert">
          {renameError}
        </p>
      )}
      <p
        className="proxy-card-url"
        title={remote ? profile.url || undefined : profile.file || undefined}
      >
        {endpoint}
      </p>
      <p className="proxy-card-updated" title={profile.updated_at ?? undefined}>
        {updatedLabel(profile.updated_at)}
      </p>

      <div className="proxy-card-spacer" />

      {manual && traffic ? (
        <button
          type="button"
          className="proxy-card-quota is-manual"
          disabled={busy}
          data-tooltip="点击修改额度与到期"
          aria-label={`修改 ${profile.name} 的额度与到期`}
          onClick={onEdit}
        >
          <span className="proxy-card-quota-row">
            <span>{traffic.total > 0 ? "手动额度" : "不限量"}</span>
            {traffic.total > 0 && (
              <span data-alert={expired || undefined}>
                剩余 {fmtBytes(traffic.total)}
              </span>
            )}
          </span>
          <span
            className="proxy-card-quota-foot"
            data-alert={expired || undefined}
          >
            {traffic.expire > 0
              ? `到期 ${dayLabel(traffic.expire)}`
              : "永久有效"}
            {expired ? " · 已过期" : ""}
          </span>
        </button>
      ) : traffic ? (
        <CardTraffic traffic={traffic} />
      ) : (
        <button
          type="button"
          className="proxy-card-quota is-empty"
          disabled={busy}
          data-tooltip="填写套餐总量与到期"
          aria-label={`设置 ${profile.name} 的额度与到期`}
          onClick={onEdit}
        >
          <span className="proxy-card-quota-row">
            <span>未上报额度</span>
            <span className="proxy-card-quota-action">设置</span>
          </span>
          <span className="proxy-card-quota-foot">总量与到期可手动填写</span>
        </button>
      )}

      <footer className="proxy-card-foot">
        <div className="proxy-card-foot-row">
          {remote ? (
            <label className="proxy-card-interval">
              <Clock size={11} aria-hidden="true" />
              <select
                aria-label={`${profile.name} 自动更新间隔`}
                value={String(profile.update_interval || 0)}
                disabled={busy}
                onChange={(event) => onInterval(Number(event.target.value))}
              >
                <option value="0">手动</option>
                <option value="6">6h</option>
                <option value="12">12h</option>
                <option value="24">24h</option>
                <option value="72">72h</option>
              </select>
            </label>
          ) : (
            <span className="proxy-card-interval is-static">本机文件</span>
          )}
          <div className="proxy-card-icons">
            {remote && (
              <button
                type="button"
                className="proxy-card-icon"
                aria-label={`更新订阅 ${profile.name}`}
                data-tooltip="立即更新订阅"
                disabled={busy}
                onClick={onUpdate}
              >
                <RefreshCw size={12} />
              </button>
            )}
            {(!profile.traffic || profile.traffic.total <= 0) && (
              <button
                type="button"
                className="proxy-card-icon"
                aria-label={`设置 ${profile.name} 的额度与到期`}
                data-tooltip="设置额度与到期"
                disabled={busy}
                onClick={onEdit}
              >
                <Gauge size={12} />
              </button>
            )}
            <button
              type="button"
              className="proxy-card-icon is-danger"
              aria-label={`删除配置 ${profile.name}`}
              data-tooltip="删除这个配置"
              onClick={onDelete}
            >
              <Trash2 size={12} />
            </button>
          </div>
        </div>
        <button
          type="button"
          className="proxy-card-use"
          disabled={busy}
          aria-label={`${active ? "重新应用" : "使用"} ${profile.name}`}
          onClick={onUse}
        >
          {active ? "重新应用" : "使用此配置"}
        </button>
      </footer>
    </article>
  );
}
