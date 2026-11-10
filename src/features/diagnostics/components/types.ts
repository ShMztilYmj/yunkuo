import type { NativeDetect } from "../../../services/api";

/** 逐项修复状态：修复中 / 已修复 / 失败 */
export type FixState = "fixing" | "done" | "error";

/** 单项修复条目：带唯一 key、修复动作（可选自动执行）、修复后本项校验 */
export interface FixItem {
  key: string;
  text: string;
  /** 修复按钮文案；空表示需人工处理 */
  actionLabel?: string;
  run?: () => Promise<void>;
  /** 修复动作后用于判定该项是否已解决的本地快检；未设置则依赖全量重测 */
  quickCheck?: (n: NativeDetect) => boolean;
  navigateTo?: string;
  targetId?: string;
  manualHint?: string;
}

/** Turnstile 两种模式自检组件的挂载容器 */
export interface TurnstileMounts {
  managed: HTMLElement | null;
  nonInteractive: HTMLElement | null;
}
