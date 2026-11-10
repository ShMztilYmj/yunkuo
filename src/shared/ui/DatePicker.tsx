import {
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import {
  CalendarDays,
  ChevronLeft,
  ChevronRight,
  ChevronsLeft,
  ChevronsRight,
} from "lucide-react";

/* ---------- 日期工具（全部走本地时区，不碰 UTC 以免跨时区差一天） ---------- */
const WEEK_LABELS = ["一", "二", "三", "四", "五", "六", "日"];
const WEEK_NAMES = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];
const MONTH_LABELS = Array.from(
  { length: 12 },
  (_, index) => `${index + 1} 月`,
);
const YEAR_SPAN = 10;
const MONTHS_PER_YEAR = 12;

type PickerMode = "day" | "month" | "year";

const pad2 = (value: number) => String(value).padStart(2, "0");

/** 当天零点：所有比较都按日期，不按时刻。 */
function today(): Date {
  const now = new Date();
  return new Date(now.getFullYear(), now.getMonth(), now.getDate());
}

/** `Date` → 表单值与接口共用的 `YYYY-MM-DD`。 */
export function toDateValue(date: Date) {
  return `${date.getFullYear()}-${pad2(date.getMonth() + 1)}-${pad2(date.getDate())}`;
}

/** `YYYY-MM-DD` → `Date`；格式或日期非法（如 2026-02-30）返回 null。 */
export function parseDateValue(value: string): Date | null {
  const match = /^(\d{4})-(\d{1,2})-(\d{1,2})$/.exec(value.trim());
  if (!match) return null;
  const year = Number(match[1]);
  const month = Number(match[2]) - 1;
  const day = Number(match[3]);
  const date = new Date(year, month, day);
  if (
    date.getFullYear() !== year ||
    date.getMonth() !== month ||
    date.getDate() !== day
  ) {
    return null;
  }
  return date;
}

function weekdayName(date: Date) {
  return WEEK_NAMES[(date.getDay() + 6) % 7];
}

/** 按月平移并夹住月末：1 月 31 日 + 1 个月 = 2 月 28/29 日。 */
function addMonths(date: Date, count: number) {
  const first = new Date(date.getFullYear(), date.getMonth() + count, 1);
  const last = new Date(first.getFullYear(), first.getMonth() + 1, 0).getDate();
  return new Date(
    first.getFullYear(),
    first.getMonth(),
    Math.min(date.getDate(), last),
  );
}

function shiftDays(date: Date, days: number) {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate() + days);
}

function daysInMonth(year: number, month: number) {
  return new Date(year, month + 1, 0).getDate();
}

export interface DatePreset {
  label: string;
  /** 相对今天往后加几个月；与 `days` 二选一。 */
  months?: number;
  /** 相对今天往后加几天。 */
  days?: number;
}

export interface DatePickerProps {
  /** `YYYY-MM-DD`，空串表示未选择。 */
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
  /** 触发按钮的无障碍名称；外层已有可见标签时也建议带上。 */
  ariaLabel?: string;
  placeholder?: string;
  /** 面板底部的快捷档位，例如「3 个月」。 */
  presets?: DatePreset[];
  /** 是否显示「清除」；外层已有「不到期」开关时可以关掉。 */
  clearable?: boolean;
  /** 把今天之前的日期压暗，用于到期日这类「选过去没意义」的场景。 */
  dimPast?: boolean;
}

/**
 * 应用内日期选择器：替代原生 `<input type="date">`。
 *
 * 原生控件在 WebView 里弹的是一块英文白底日历（mm/dd/yyyy、Su Mo Tu…），
 * 和深色面板完全脱节，也没法和「不到期」这类并列选项对齐。这里改成中文日历：
 * 触发器与相邻输入框同高同色，面板用 popover 进入 top layer，因此不会被
 * `<dialog showModal>` 的遮罩压住、也不会被弹窗卡片的 overflow 裁掉。
 */
export default function DatePicker({
  value,
  onChange,
  disabled = false,
  ariaLabel = "选择日期",
  placeholder = "选择日期",
  presets,
  clearable = true,
  dimPast = false,
}: DatePickerProps) {
  const trigger = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const dayNodes = useRef(new Map<string, HTMLButtonElement>());
  const monthNodes = useRef(new Map<number, HTMLButtonElement>());
  const yearNodes = useRef(new Map<number, HTMLButtonElement>());
  const panelId = useId();

  const [open, setOpen] = useState(false);
  const [placed, setPlaced] = useState(false);
  const [mode, setMode] = useState<PickerMode>("day");
  // 面板当前翻到的月份（始终存 1 号）与键盘游标所在日期。
  const [view, setView] = useState(() => {
    const base = parseDateValue(value) ?? today();
    return new Date(base.getFullYear(), base.getMonth(), 1);
  });
  const [cursor, setCursor] = useState(() =>
    toDateValue(parseDateValue(value) ?? today()),
  );
  // 只有「该把焦点交给日历」的操作才递增：打开面板、方向键移动、切换视图。
  // 用 ‹ › 翻月时焦点留在按钮上，连续点击不会被拽走。
  const [focusTick, setFocusTick] = useState(0);

  const todayValue = toDateValue(today());
  const selected = parseDateValue(value);

  const cells = useMemo(() => {
    const first = new Date(view.getFullYear(), view.getMonth(), 1);
    // 以周一为一周起点：offset = 1 号前面要补几天。
    const offset = (first.getDay() + 6) % 7;
    const start = new Date(first.getFullYear(), first.getMonth(), 1 - offset);
    return Array.from(
      { length: 42 },
      (_, index) =>
        new Date(
          start.getFullYear(),
          start.getMonth(),
          start.getDate() + index,
        ),
    );
  }, [view]);

  const decadeStart = Math.floor(view.getFullYear() / YEAR_SPAN) * YEAR_SPAN;

  const openPanel = () => {
    const base = parseDateValue(value) ?? today();
    setView(new Date(base.getFullYear(), base.getMonth(), 1));
    setCursor(toDateValue(base));
    setMode("day");
    setPlaced(false);
    setOpen(true);
    setFocusTick((tick) => tick + 1);
  };

  const closePanel = (restoreFocus = true) => {
    setOpen(false);
    setPlaced(false);
    if (restoreFocus) {
      requestAnimationFrame(() => {
        trigger.current?.focus({ preventScroll: true });
      });
    }
  };

  const commit = (date: Date) => {
    setCursor(toDateValue(date));
    onChange(toDateValue(date));
    closePanel();
  };

  /* 弹出面板：先入 top layer，再按触发器位置摆放（下方放不下就翻到上方）。 */
  useLayoutEffect(() => {
    if (!open) return;
    const node = panel.current;
    const anchor = trigger.current;
    if (!node || !anchor) return;

    if (!node.hasAttribute("popover")) node.setAttribute("popover", "manual");
    if (!node.matches(":popover-open")) {
      try {
        node.showPopover();
      } catch {
        // 老内核没有 popover：退回普通定位，仍在 <dialog> 之上。
        node.removeAttribute("popover");
      }
    }

    const place = () => {
      const rect = anchor.getBoundingClientRect();
      const width = node.offsetWidth;
      const height = node.offsetHeight;
      if (!width || !height) return;
      const gap = 6;
      const margin = 10;
      const viewportWidth = window.innerWidth;
      const viewportHeight = window.innerHeight;
      let top = rect.bottom + gap;
      if (top + height > viewportHeight - margin) {
        const above = rect.top - gap - height;
        top =
          above >= margin
            ? above
            : Math.max(margin, viewportHeight - height - margin);
      }
      // 触发器在右半边就让面板右对齐：浮层比输入框宽，居左会戳出弹窗卡片，
      // 右对齐后整块仍落在卡片右缘之内。
      const preferRight = rect.left + rect.width / 2 > viewportWidth / 2;
      const left = Math.min(
        Math.max(margin, preferRight ? rect.right - width : rect.left),
        Math.max(margin, viewportWidth - width - margin),
      );
      node.style.left = `${Math.round(left)}px`;
      node.style.top = `${Math.round(top)}px`;
    };

    place();
    setPlaced(true);
    const observer = new ResizeObserver(place);
    observer.observe(node);
    return () => observer.disconnect();
  }, [open]);

  /* 关闭途径：点空白处、Esc、窗口尺寸变化 / 祖先滚动（面板是 fixed，必须跟着收起）。 */
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (panel.current?.contains(target)) return;
      // 触发器自己负责开合，这里放行，否则会「先关再开」。
      if (trigger.current?.contains(target)) return;
      closePanel(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      // 拦在对话框之前：Esc 只收起日历，不关闭整个编辑弹窗。
      event.preventDefault();
      event.stopPropagation();
      closePanel();
    };
    const onViewportChange = (event: Event) => {
      if (
        event.type === "scroll" &&
        event.target instanceof Node &&
        panel.current?.contains(event.target)
      ) {
        return;
      }
      closePanel(false);
    };
    window.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("resize", onViewportChange);
    window.addEventListener("scroll", onViewportChange, true);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("resize", onViewportChange);
      window.removeEventListener("scroll", onViewportChange, true);
    };
  }, [open]);

  /* 焦点跟随游标：打开时落在已选日期，方向键移动后跟到新格子。 */
  useEffect(() => {
    if (!open || !placed) return;
    const node =
      mode === "day"
        ? dayNodes.current.get(cursor)
        : mode === "month"
          ? monthNodes.current.get(view.getMonth())
          : yearNodes.current.get(view.getFullYear() - decadeStart);
    node?.focus({ preventScroll: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, placed, mode, focusTick]);

  const shiftView = (months: number) => {
    const target = new Date(view.getFullYear(), view.getMonth() + months, 1);
    const base = parseDateValue(cursor) ?? target;
    const day = Math.min(
      base.getDate(),
      daysInMonth(target.getFullYear(), target.getMonth()),
    );
    setView(target);
    setCursor(
      toDateValue(new Date(target.getFullYear(), target.getMonth(), day)),
    );
  };

  const pickMonth = (month: number) => {
    const base = parseDateValue(cursor) ?? today();
    const day = Math.min(
      base.getDate(),
      daysInMonth(view.getFullYear(), month),
    );
    setView(new Date(view.getFullYear(), month, 1));
    setCursor(toDateValue(new Date(view.getFullYear(), month, day)));
    setMode("day");
    setFocusTick((tick) => tick + 1);
  };

  const pickYear = (year: number) => {
    setView(new Date(year, view.getMonth(), 1));
    setMode("month");
  };

  /* 网格内的键盘导航：方向键 ±1/±7 天，Home/End 到本周首尾，PageUp/Down 翻月。 */
  const onGridKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (mode !== "day") return;
    const base = parseDateValue(cursor) ?? today();
    const weekday = (base.getDay() + 6) % 7;
    let next: Date;
    switch (event.key) {
      case "ArrowLeft":
        next = shiftDays(base, -1);
        break;
      case "ArrowRight":
        next = shiftDays(base, 1);
        break;
      case "ArrowUp":
        next = shiftDays(base, -7);
        break;
      case "ArrowDown":
        next = shiftDays(base, 7);
        break;
      case "Home":
        next = shiftDays(base, -weekday);
        break;
      case "End":
        next = shiftDays(base, 6 - weekday);
        break;
      case "PageUp":
        next = addMonths(base, event.shiftKey ? -12 : -1);
        break;
      case "PageDown":
        next = addMonths(base, event.shiftKey ? 12 : 1);
        break;
      default:
        return;
    }
    event.preventDefault();
    setCursor(toDateValue(next));
    setView(new Date(next.getFullYear(), next.getMonth(), 1));
    setFocusTick((tick) => tick + 1);
  };

  const applyPreset = (preset: DatePreset) => {
    const base = today();
    commit(
      preset.months
        ? addMonths(base, preset.months)
        : shiftDays(base, preset.days ?? 0),
    );
  };

  const title =
    mode === "day"
      ? `${view.getFullYear()} 年 ${view.getMonth() + 1} 月`
      : mode === "month"
        ? `${view.getFullYear()} 年`
        : `${decadeStart} – ${decadeStart + YEAR_SPAN - 1}`;
  const step =
    mode === "day"
      ? 1
      : mode === "month"
        ? MONTHS_PER_YEAR
        : MONTHS_PER_YEAR * YEAR_SPAN;

  return (
    <>
      <button
        ref={trigger}
        type="button"
        className="dp-trigger"
        aria-label={`${ariaLabel}：${value || "未选择"}`}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={open ? panelId : undefined}
        data-open={open ? "true" : undefined}
        disabled={disabled}
        onClick={() => (open ? closePanel() : openPanel())}
        onKeyDown={(event) => {
          if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
          event.preventDefault();
          if (!open) openPanel();
        }}
      >
        <span className="dp-value" data-empty={value ? undefined : "true"}>
          {value || placeholder}
        </span>
        {selected && (
          <span className="dp-weekday">{weekdayName(selected)}</span>
        )}
        <CalendarDays className="dp-icon" size={13} aria-hidden="true" />
      </button>

      {open && (
        <div
          ref={panel}
          id={panelId}
          className="dp-panel"
          role="dialog"
          aria-label="选择日期"
          data-placed={placed ? "true" : "false"}
          onKeyDown={onGridKeyDown}
          onBlur={(event) => {
            const next = event.relatedTarget;
            if (!(next instanceof Node)) return;
            if (panel.current?.contains(next)) return;
            if (trigger.current?.contains(next)) return;
            closePanel(false);
          }}
        >
          <div className="dp-head">
            {mode === "day" && (
              <button
                type="button"
                className="dp-nav"
                aria-label="上一年"
                onClick={() => shiftView(-MONTHS_PER_YEAR)}
              >
                <ChevronsLeft size={13} />
              </button>
            )}
            <button
              type="button"
              className="dp-nav"
              aria-label={
                mode === "day"
                  ? "上一月"
                  : mode === "month"
                    ? "上一年份"
                    : "上一组年份"
              }
              onClick={() => shiftView(-step)}
            >
              <ChevronLeft size={13} />
            </button>
            <button
              type="button"
              className="dp-title"
              disabled={mode === "year"}
              aria-label={mode === "day" ? "选择月份" : "选择年份"}
              onClick={() => setMode(mode === "day" ? "month" : "year")}
            >
              {title}
            </button>
            <button
              type="button"
              className="dp-nav"
              aria-label={
                mode === "day"
                  ? "下一月"
                  : mode === "month"
                    ? "下一年份"
                    : "下一组年份"
              }
              onClick={() => shiftView(step)}
            >
              <ChevronRight size={13} />
            </button>
            {mode === "day" && (
              <button
                type="button"
                className="dp-nav"
                aria-label="下一年"
                onClick={() => shiftView(MONTHS_PER_YEAR)}
              >
                <ChevronsRight size={13} />
              </button>
            )}
          </div>

          {mode === "day" ? (
            <>
              <div className="dp-week" aria-hidden="true">
                {WEEK_LABELS.map((label, index) => (
                  <span
                    key={label}
                    data-weekend={index >= 5 ? "true" : undefined}
                  >
                    {label}
                  </span>
                ))}
              </div>
              <div
                className="dp-grid"
                role="group"
                aria-label={`${view.getFullYear()} 年 ${view.getMonth() + 1} 月`}
              >
                {cells.map((date) => {
                  const key = toDateValue(date);
                  const outside = date.getMonth() !== view.getMonth();
                  const isSelected = key === value;
                  return (
                    <button
                      key={key}
                      type="button"
                      className="dp-day"
                      ref={(node) => {
                        if (node) dayNodes.current.set(key, node);
                        else dayNodes.current.delete(key);
                      }}
                      tabIndex={key === cursor ? 0 : -1}
                      aria-label={`${date.getFullYear()} 年 ${
                        date.getMonth() + 1
                      } 月 ${date.getDate()} 日 ${weekdayName(date)}`}
                      aria-pressed={isSelected}
                      data-outside={outside ? "true" : undefined}
                      data-selected={isSelected ? "true" : undefined}
                      data-today={key === todayValue ? "true" : undefined}
                      data-past={
                        dimPast && key < todayValue ? "true" : undefined
                      }
                      onClick={() => commit(date)}
                    >
                      {date.getDate()}
                    </button>
                  );
                })}
              </div>
            </>
          ) : mode === "month" ? (
            <div
              className="dp-units"
              role="group"
              aria-label={`${view.getFullYear()} 年`}
            >
              {MONTH_LABELS.map((label, index) => (
                <button
                  key={label}
                  type="button"
                  className="dp-unit"
                  ref={(node) => {
                    if (node) monthNodes.current.set(index, node);
                    else monthNodes.current.delete(index);
                  }}
                  tabIndex={index === view.getMonth() ? 0 : -1}
                  data-active={
                    selected &&
                    selected.getFullYear() === view.getFullYear() &&
                    selected.getMonth() === index
                      ? "true"
                      : undefined
                  }
                  onClick={() => pickMonth(index)}
                >
                  {label}
                </button>
              ))}
            </div>
          ) : (
            <div
              className="dp-units is-years"
              role="group"
              aria-label="选择年份"
            >
              {Array.from({ length: YEAR_SPAN }, (_, index) => {
                const year = decadeStart + index;
                return (
                  <button
                    key={year}
                    type="button"
                    className="dp-unit"
                    ref={(node) => {
                      if (node) yearNodes.current.set(index, node);
                      else yearNodes.current.delete(index);
                    }}
                    tabIndex={year === view.getFullYear() ? 0 : -1}
                    data-active={
                      selected?.getFullYear() === year ? "true" : undefined
                    }
                    onClick={() => pickYear(year)}
                  >
                    {year}
                  </button>
                );
              })}
            </div>
          )}

          {presets && presets.length > 0 && (
            <div className="dp-presets">
              {presets.map((preset) => (
                <button
                  key={preset.label}
                  type="button"
                  className="dp-chip"
                  onClick={() => applyPreset(preset)}
                >
                  {preset.label}
                </button>
              ))}
            </div>
          )}

          <div className="dp-foot">
            <button
              type="button"
              className="dp-link"
              onClick={() => commit(today())}
            >
              今天
            </button>
            {clearable && value && (
              <button
                type="button"
                className="dp-link"
                data-tone="danger"
                onClick={() => {
                  onChange("");
                  closePanel();
                }}
              >
                清除
              </button>
            )}
          </div>
        </div>
      )}
    </>
  );
}
