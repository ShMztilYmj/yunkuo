/** 设置开关：受控布尔值；异步设置进行中时锁定并降低不透明度。 */
export default function Switch({
  v,
  on,
  disabled = false,
  label,
}: {
  v: boolean;
  on: (b: boolean) => void;
  disabled?: boolean;
  label?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={v}
      aria-label={label}
      disabled={disabled}
      onClick={() => on(!v)}
      className="settings-switch"
      data-on={v}
    >
      <span aria-hidden="true" />
    </button>
  );
}
