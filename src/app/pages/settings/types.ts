export type { SettingsUpdater } from "../../configuration";

/** 设置页分类；与 ?tab= 查询参数保持一致。 */
export type SettingsTab =
  "proxy" | "timezone" | "detect" | "ai" | "general" | "about";
