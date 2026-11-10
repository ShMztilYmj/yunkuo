/**
 * 服务层统一入口（barrel）。
 *
 * 原先单文件的 api.ts 已按领域拆分到 ./api/ 下：
 * - invoke.ts       Tauri invoke 薄封装（call）
 * - config.ts       应用配置
 * - core.ts         原生代理 / 流量 / 连接 / 兼容转发
 * - profiles.ts     订阅与配置文件
 * - network.ts      系统代理 / TUN / 隐私保护 / 网络模式
 * - detect.ts       环境检测 / ipmap / DNS 深度探测
 * - timezone.ts     时区同步
 * - ai.ts           AI 用量 / 账号 / 工具配置
 * - maintenance.ts  诊断 / 升级 / 服务安装 / 默认浏览器
 * - app.ts          应用启动 / 数据目录 / 管理员状态 / UI 日志
 * - events.ts       listenCore* 事件监听
 *
 * 对外导出面（api、controller、listenCore* 以及全部类型）与拆分前完全一致，
 * 命令名、参数对象形状与返回值类型均未改动。
 */
import { aiApi } from "./api/ai";
import { appApi } from "./api/app";
import { configApi } from "./api/config";
import { coreApi } from "./api/core";
import { detectApi } from "./api/detect";
import { maintenanceApi } from "./api/maintenance";
import { networkApi } from "./api/network";
import { profilesApi } from "./api/profiles";
import { timezoneApi } from "./api/timezone";

export * from "./api/ai";
export * from "./api/app";
export * from "./api/config";
export * from "./api/core";
export * from "./api/detect";
export * from "./api/events";
export * from "./api/maintenance";
export * from "./api/network";
export * from "./api/profiles";
export * from "./api/timezone";

// 各领域对象合并为同一个 api，键名与拆分前逐一对应；
// 枚举顺序按领域分组（与原文件不同），仓库内没有任何代码依赖该顺序。
export const api = {
  ...aiApi,
  ...configApi,
  ...coreApi,
  ...profilesApi,
  ...networkApi,
  ...timezoneApi,
  ...detectApi,
  ...appApi,
  ...maintenanceApi,
};
