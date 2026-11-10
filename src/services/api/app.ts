import { call } from "./invoke";

// ---------- 应用级：启动 AI 工具 / 数据目录 / 管理员状态 / UI 日志 ----------

export interface LaunchResult {
  ok: boolean;
  failures: string[];
}

export const appApi = {
  launchClaude: (confirmed = false) =>
    call<LaunchResult>("launch_claude", { confirmed }),
  openDataDir: () => call<void>("open_data_dir"),
  isElevated: () => call<boolean>("is_elevated_cmd"),
  launchAi: (tool: string, confirmed = false) =>
    call<LaunchResult>("launch_ai", { tool, confirmed }),
  uiLog: (line: string) => call<void>("ui_log", { line }),
  /** 用户在「仍有会话在运行」的确认框里选择强制退出。 */
  forceExit: () => call<void>("app_force_exit"),
};
