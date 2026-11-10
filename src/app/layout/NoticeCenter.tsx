import {
  CircleAlert,
  Inbox,
  Info,
  Mail,
  Settings2,
  TriangleAlert,
  X,
} from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useLocation, useNavigate } from "react-router-dom";
import type { NoticeLevel } from "../../services/api/core";
import { useAppStore } from "../../store";

/**
 * 标题栏右侧的消息中心。
 *
 * 内核与联动产生的未处理提示原先以横幅压在工作区顶部，挡住页面又只能看最后一条；
 * 现在收敛成一个邮箱入口：有未处理时红点计数，点开才是完整列表，可单条关闭或一次清空。
 *
 * 列表按等级分开：错误（要处理）、警告（已降级但仍能用）、提示（仅告知）。
 * 角标、条目左侧色条和等级标签共用一套颜色，头部可以只看某一级；
 * 没有的等级不占位置，某一级被清空后自动回到「全部」。
 *
 * 有的提示内核会一直上报、关不掉：这时同一条点第二次会弹确认框，
 * 确认后只在本会话列表里隐藏（问题本身仍需处理，收尾会写明）。
 */

interface LevelMeta {
  id: NoticeLevel;
  label: string;
  hint: string;
  icon: typeof Info;
}

interface Notice {
  id: number;
  level: NoticeLevel;
  message: string;
}

const LEVELS: readonly LevelMeta[] = [
  { id: "error", label: "错误", hint: "需要处理", icon: CircleAlert },
  { id: "warn", label: "警告", hint: "已降级，仍可用", icon: TriangleAlert },
  { id: "info", label: "提示", hint: "仅告知", icon: Info },
];

const RANK: Record<NoticeLevel, number> = { error: 0, warn: 1, info: 2 };

/** 旧版本内核不带等级：按错误记，避免分组和排序落到空值上。 */
const levelOf = (value: unknown): NoticeLevel =>
  value === "warn" || value === "info" ? value : "error";

export default function NoticeCenter() {
  const errors = useAppStore((s) => s.errors);
  const dismissError = useAppStore((s) => s.dismissError);
  const loc = useLocation();
  const nav = useNavigate();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [filter, setFilter] = useState<NoticeLevel | "all">("all");
  const [position, setPosition] = useState({ left: 12, top: 12 });
  /** 关闭失败过的提示：再点一次才弹确认，避免误以为第二次就能直接关掉。 */
  const [stubborn, setStubborn] = useState<ReadonlySet<number>>(new Set());
  /** 已强制隐藏的提示：内核仍在上报，这里按 id 过滤。 */
  const [hidden, setHidden] = useState<ReadonlySet<number>>(new Set());
  const [pending, setPending] = useState<{
    id: number;
    message: string;
  } | null>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const confirm = useRef<HTMLDivElement>(null);
  // 与旧提示条一致：只有首页展示「上次程序未正常结束」的历史记录。
  // 排序按等级（错误在前）再按上报顺序，等级即浏览优先级。
  const { notices, counts } = useMemo(() => {
    const list = errors
      .filter((error) => !hidden.has(error.id))
      .filter(
        (error) =>
          loc.pathname === "/" ||
          !error.message.startsWith("上次程序未正常结束"),
      )
      .map((error) => ({
        id: error.id,
        message: error.message,
        level: levelOf(error.level),
      }))
      .sort(
        (left, right) =>
          RANK[left.level] - RANK[right.level] || left.id - right.id,
      );
    const tally: Record<NoticeLevel, number> = { error: 0, warn: 0, info: 0 };
    for (const notice of list) tally[notice.level] += 1;
    return { notices: list, counts: tally };
  }, [errors, hidden, loc.pathname]);
  const count = notices.length;
  const topLevel: NoticeLevel =
    counts.error > 0 ? "error" : counts.warn > 0 ? "warn" : "info";
  const visible =
    filter === "all"
      ? notices
      : notices.filter((notice) => notice.level === filter);

  useEffect(() => {
    setOpen(false);
    setPending(null);
  }, [loc.pathname, loc.search]);
  // 筛到的那一级被关空后没有内容可看，直接退回全部。
  useEffect(() => {
    if (filter !== "all" && counts[filter] === 0) setFilter("all");
  }, [counts, filter]);
  useLayoutEffect(() => {
    if (!open) return;
    const place = () => {
      if (!trigger.current || !panel.current) return;
      const anchor = trigger.current.getBoundingClientRect();
      const width = panel.current.offsetWidth;
      const height = panel.current.offsetHeight;
      setPosition({
        left: Math.max(
          12,
          Math.min(anchor.right - width, window.innerWidth - width - 12),
        ),
        top: Math.max(
          12,
          Math.min(anchor.bottom + 6, window.innerHeight - height - 12),
        ),
      });
    };
    place();
    const observer = new ResizeObserver(place);
    if (panel.current) observer.observe(panel.current);
    // 焦点进面板但不落在某个按钮上：键盘用户 Tab 即可进入，鼠标用户不会被焦点环分心。
    panel.current?.focus({ preventScroll: true });
    window.addEventListener("resize", place);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", place);
    };
  }, [open]);
  useLayoutEffect(() => {
    if (pending) confirm.current?.querySelector("button")?.focus();
  }, [pending]);
  useEffect(() => {
    if (!open) return;
    const outside = (event: Event) => {
      // 确认框开着时，点击遮罩由确认框自己处理，不能顺手把外层也关掉。
      if (pending) return;
      // 确认框关掉时焦点会落回 body；这不代表用户点了别处。
      if (!(event.target instanceof Node) || !event.target.isConnected) return;
      if (event.target === document.body) return;
      if (confirm.current?.contains(event.target)) return;
      if (
        !panel.current?.contains(event.target) &&
        !trigger.current?.contains(event.target)
      )
        setOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      if (pending) {
        setPending(null);
        return;
      }
      setOpen(false);
      trigger.current?.focus({ preventScroll: true });
    };
    const blur = () => setOpen(false);
    document.addEventListener("pointerdown", outside);
    document.addEventListener("focusin", outside);
    document.addEventListener("keydown", escape);
    window.addEventListener("blur", blur);
    return () => {
      document.removeEventListener("pointerdown", outside);
      document.removeEventListener("focusin", outside);
      document.removeEventListener("keydown", escape);
      window.removeEventListener("blur", blur);
    };
  }, [open, pending]);

  /** 第一次点关闭：真的调内核；内核不认账就记下来，下次再点走确认。 */
  const close = async (notice: { id: number; message: string }) => {
    if (busy) return;
    if (stubborn.has(notice.id)) {
      setPending(notice);
      return;
    }
    setBusy(true);
    let failed = false;
    try {
      await dismissError(notice.id);
    } catch (error) {
      failed = true;
      console.error(error);
    } finally {
      setBusy(false);
    }
    // 内核拒绝或刷新后仍在列表里：这条只能强制隐藏。
    if (failed || useAppStore.getState().errors.some((e) => e.id === notice.id))
      setStubborn((previous) => new Set(previous).add(notice.id));
  };
  /** 关闭当前看得到的这些提示；筛选状态下不会顺手清掉别的等级。 */
  const dismissAll = async (targets: readonly Notice[]) => {
    if (busy) return;
    setBusy(true);
    const refused = new Set<number>();
    try {
      for (const notice of targets) {
        try {
          await dismissError(notice.id);
        } catch (error) {
          console.error(error);
        }
        if (useAppStore.getState().errors.some((e) => e.id === notice.id))
          refused.add(notice.id);
      }
    } finally {
      setBusy(false);
    }
    if (refused.size)
      setStubborn((previous) => new Set([...previous, ...refused]));
  };
  const forceClose = () => {
    if (!pending) return;
    setHidden((previous) => new Set(previous).add(pending.id));
    setPending(null);
  };

  return (
    <>
      <button
        ref={trigger}
        aria-label={
          count > 0 ? `提示消息，${count} 条未处理` : "提示消息，暂无未处理"
        }
        data-tooltip={
          count > 0
            ? LEVELS.filter((level) => counts[level.id] > 0)
                .map((level) => `${level.label} ${counts[level.id]}`)
                .join(" · ")
            : "提示消息"
        }
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls="notice-center-popover"
        onClick={() => setOpen((value) => !value)}
        className={`flex w-11 items-center justify-center text-zinc-500 hover:bg-zinc-800 hover:text-zinc-200 ${open ? "bg-zinc-800 text-zinc-200" : ""}`}
      >
        <span className="notice-trigger">
          <Mail size={15} />
          {count > 0 && (
            <span
              className="notice-badge"
              data-level={topLevel}
              aria-hidden="true"
            >
              {count > 9 ? "9+" : count}
            </span>
          )}
        </span>
      </button>
      {open &&
        createPortal(
          <div
            ref={panel}
            id="notice-center-popover"
            role="dialog"
            aria-label="提示消息"
            tabIndex={-1}
            className="notice-popover ui-popover"
            style={position}
          >
            <header className="notice-head">
              <strong>提示消息</strong>
              <span>{count > 0 ? `${count} 条未处理` : "全部已处理"}</span>
            </header>
            {count > 0 ? (
              <>
                <div
                  className="notice-filters"
                  role="group"
                  aria-label="按等级筛选消息"
                >
                  <button
                    type="button"
                    className="notice-chip"
                    data-active={filter === "all"}
                    aria-pressed={filter === "all"}
                    onClick={() => setFilter("all")}
                  >
                    全部 {count}
                  </button>
                  {LEVELS.filter((level) => counts[level.id] > 0).map(
                    ({ id, label, hint, icon: Icon }) => (
                      <button
                        key={id}
                        type="button"
                        className="notice-chip"
                        data-level={id}
                        data-active={filter === id}
                        data-tooltip={hint}
                        aria-pressed={filter === id}
                        onClick={() => setFilter(filter === id ? "all" : id)}
                      >
                        <Icon size={11} aria-hidden="true" />
                        {label} {counts[id]}
                      </button>
                    ),
                  )}
                </div>
                {visible.length > 0 ? (
                  <ul className="notice-list">
                    {visible.map((notice) => {
                      const meta = LEVELS[RANK[notice.level]];
                      const Icon = meta.icon;
                      return (
                        <li
                          key={notice.id}
                          className="notice-item"
                          data-level={notice.level}
                        >
                          <p className="notice-text">
                            <Icon
                              className="notice-icon"
                              size={12}
                              aria-hidden="true"
                            />
                            <span>{notice.message}</span>
                          </p>
                          <div className="notice-foot">
                            <span className="notice-tag">{meta.label}</span>
                            <button
                              type="button"
                              className="notice-dismiss"
                              aria-label={
                                stubborn.has(notice.id)
                                  ? `再次点击可强制关闭提示：${notice.message.slice(0, 40)}`
                                  : `关闭提示：${notice.message.slice(0, 40)}`
                              }
                              disabled={busy}
                              onClick={() => void close(notice)}
                            >
                              <X size={12} />
                              {stubborn.has(notice.id) ? "仍要关闭？" : "关闭"}
                            </button>
                          </div>
                        </li>
                      );
                    })}
                  </ul>
                ) : (
                  <p className="notice-empty">
                    <Inbox size={16} />
                    该等级暂无未处理的消息。
                  </p>
                )}
                <div className="notice-actions">
                  <button
                    type="button"
                    className="notice-link"
                    onClick={() => {
                      setOpen(false);
                      nav("/settings");
                    }}
                  >
                    <Settings2 size={12} />
                    查看设置
                  </button>
                  {visible.length > 0 && (
                    <button
                      type="button"
                      className="notice-link"
                      disabled={busy}
                      onClick={() => void dismissAll(visible)}
                    >
                      {busy
                        ? "正在关闭…"
                        : filter === "all"
                          ? "全部关闭"
                          : `关闭这 ${visible.length} 条`}
                    </button>
                  )}
                </div>
              </>
            ) : (
              <p className="notice-empty">
                <Inbox size={16} />
                暂无未处理的提示。
              </p>
            )}
          </div>,
          document.body,
        )}
      {pending &&
        createPortal(
          <div
            className="app-dialog-backdrop"
            onPointerDown={(event) => {
              if (event.target === event.currentTarget) setPending(null);
            }}
          >
            <div
              ref={confirm}
              role="alertdialog"
              aria-modal="true"
              aria-labelledby="notice-confirm-title"
              aria-describedby="notice-confirm-body"
              className="app-dialog"
            >
              <h2 id="notice-confirm-title">强制关闭这条提示？</h2>
              <p id="notice-confirm-body">
                这条提示没能从内核移除。确认后只在本会话的消息列表里隐藏，问题本身仍然存在；
                需要处理时请在「设置」或相应页面查看。
              </p>
              <p className="app-dialog-quote">{pending.message}</p>
              <div className="app-dialog-actions">
                <button type="button" onClick={() => setPending(null)}>
                  取消
                </button>
                <button
                  type="button"
                  className="is-danger"
                  onClick={forceClose}
                >
                  确认关闭
                </button>
              </div>
            </div>
          </div>,
          document.body,
        )}
    </>
  );
}
