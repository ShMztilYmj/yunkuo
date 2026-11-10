import { call } from "../../services/api/invoke";

/**
 * 托盘快捷面板。面板只负责渲染，所有动作统一交给主进程的 tray_action：
 * 返回的字符串是要显示在面板里的提示，null 表示该动作不需要提示。
 */
export const tray = {
  ready: () => call<void>("tray_ready"),
  action: (action: string) => call<string | null>("tray_action", { action }),
};
