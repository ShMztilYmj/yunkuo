import type { ReactNode } from "react";

/** 卡片底部说明条：给操作结果、限制条件或错误留一个固定位置。 */
export default function Note({
  children,
  tone = "idle",
  role,
}: {
  children: ReactNode;
  tone?: "idle" | "ok" | "warn" | "error";
  role?: "status" | "alert";
}) {
  return (
    <p
      className="settings-note"
      data-tone={tone === "idle" ? undefined : tone}
      role={role}
    >
      {children}
    </p>
  );
}
