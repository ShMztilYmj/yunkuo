/** 前端跨模块只发布失效通知，检测状态由应用装配的接收方更新。 */
export const ENVIRONMENT_INVALIDATED = "coc:environment-invalidated";

export function notifyEnvironmentInvalidated(): void {
  window.dispatchEvent(new Event(ENVIRONMENT_INVALIDATED));
}
