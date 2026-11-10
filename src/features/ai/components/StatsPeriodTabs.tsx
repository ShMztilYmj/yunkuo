/**
 * 统计时段切换的唯一实现：用量主视图与额度视图共用同一套外观、aria 与交互。
 *
 * 只负责呈现——时段枚举和各时段的取数仍由各自的数据源定义，
 * 因为两侧能提供的档位本来就不同（本地扫描有「本周」，额度视图没有）。
 */
export default function StatsPeriodTabs<T extends string>({
  items,
  value,
  onChange,
  label,
}: {
  items: readonly { key: T; label: string }[];
  value: T;
  onChange: (key: T) => void;
  label: string;
}) {
  return (
    <div
      role="group"
      aria-label={label}
      className="flex flex-wrap gap-1 rounded-xl bg-zinc-800/60 p-1"
    >
      {items.map((item) => (
        <button
          key={item.key}
          aria-pressed={value === item.key}
          onClick={() => onChange(item.key)}
          className={`rounded-lg px-3 py-1.5 text-xs ${
            value === item.key
              ? "bg-zinc-900 font-medium text-zinc-100 shadow-sm"
              : "text-zinc-500 hover:text-zinc-200"
          }`}
        >
          {item.label}
        </button>
      ))}
    </div>
  );
}
