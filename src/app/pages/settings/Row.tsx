import type { ReactNode } from "react";

/**
 * 设置行：左侧标签（可带一行解释），右侧控件。
 * 需要整行宽度的输入（路径、绕过列表、密钥）用 wide 改为上下排列。
 */
export default function Row({
  label,
  hint,
  children,
  wide,
}: {
  label: string;
  /** 标签下的补充说明：写清影响范围或约束，而不是重复标签 */
  hint?: string;
  children: ReactNode;
  wide?: boolean;
}) {
  return (
    <div className={`settings-row${wide ? " is-wide" : ""}`}>
      <div className="settings-row-label">
        <span>{label}</span>
        {hint && <small className="settings-row-hint">{hint}</small>}
      </div>
      <div className="settings-row-control">{children}</div>
    </div>
  );
}
