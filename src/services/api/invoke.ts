import { invoke } from "@tauri-apps/api/core";

/**
 * 后端命令错误。
 *
 * Rust 侧的命令边界把错误序列化成 `{ code, message }`；这里转成 Error 子类，
 * 既保留原有文案展示（`String(error)` 仍然只得到 message），
 * 又能按 `code` 做重试、降级或分支处理。
 */
export class ApiError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = "ApiError";
    this.code = code;
    // 大量旧界面用 `${String(error)}` 直接展示，保持输出仍是纯文案。
    Object.defineProperty(this, "toString", { value: () => message });
  }
}

/** 把 `{ code, message }` 形态的错误转成 ApiError；字符串错误保持原样。 */
export function normalizeError(error: unknown): unknown {
  if (typeof error === "object" && error !== null) {
    const { code, message } = error as { code?: unknown; message?: unknown };
    if (typeof message === "string") {
      return new ApiError(typeof code === "string" ? code : "unknown", message);
    }
  }
  return error;
}

/** 统一取展示文案：旧命令返回字符串，新命令返回 ApiError，结果都是纯文本。 */
export function errorMessage(error: unknown): string {
  const normalized = normalizeError(error);
  if (normalized instanceof Error) return normalized.message;
  return String(normalized);
}

/** 只有迁移到 AppError 的命令才有错误码，其余返回 null。 */
export function errorCode(error: unknown): string | null {
  return error instanceof ApiError ? error.code : null;
}

/**
 * 统一的 Tauri 命令调用入口。
 * 命令名、参数对象与错误语义一律原样透传，不做日志、重试或二次包装；
 * 唯一职责是把 `{ code, message }` 归一化成 ApiError。
 */
export function call<T = unknown>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  return invoke<T>(command, args).catch((error: unknown) => {
    throw normalizeError(error);
  });
}
