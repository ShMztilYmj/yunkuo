import {
  Boxes,
  Clock3,
  Info,
  ScanLine,
  Server,
  SlidersHorizontal,
  type LucideIcon,
} from "lucide-react";
import type { SettingsTab } from "./types";

/** 设置页的分类定义：分类栏、页内小标题都从这里读，避免两处文案漂移。 */
export interface SettingsSection {
  key: SettingsTab;
  /** 分类栏上的名称 */
  label: string;
  /** 分类栏上的第二行提示 */
  hint: string;
  /** 内容区标题 */
  title: string;
  /** 内容区说明：这一组设置解决什么问题 */
  description: string;
  group: string;
  icon: LucideIcon;
}

export const SETTINGS_SECTIONS: readonly SettingsSection[] = [
  {
    key: "proxy",
    label: "代理与内核",
    hint: "端口 · 接管 · 守护",
    title: "代理与内核",
    description:
      "mihomo 的监听端口、系统接管方式与保护状态，决定流量如何离开这台电脑。",
    group: "连接与网络",
    icon: Server,
  },
  {
    key: "timezone",
    label: "时间与时区",
    hint: "跟随出口 IP",
    title: "时间与时区",
    description: "让系统时间跟随出口地区，减少时区与 IP 归属不一致带来的风险。",
    group: "连接与网络",
    icon: Clock3,
  },
  {
    key: "detect",
    label: "检测与门控",
    hint: "评分 · 地区 · 情报源",
    title: "检测与门控",
    description:
      "决定启动前的放行条件，并为出口检测补充独立情报源；各工具的启动入口在 AI 管理中配置。",
    group: "环境与 AI",
    icon: ScanLine,
  },
  {
    key: "ai",
    label: "AI 模块",
    hint: "本地用量扫描",
    title: "AI 模块",
    description: "控制本机 AI 工具用量数据的读取范围，不影响工具启动与运行。",
    group: "环境与 AI",
    icon: Boxes,
  },
  {
    key: "general",
    label: "通用",
    hint: "外观 · 启动行为",
    title: "通用",
    description: "界面外观、登录后的启动方式与本地数据位置。",
    group: "应用",
    icon: SlidersHorizontal,
  },
  {
    key: "about",
    label: "关于",
    hint: "版本与许可",
    title: "关于",
    description: "当前版本、维护入口与随安装包提供的开源许可。",
    group: "应用",
    icon: Info,
  },
] as const;

export function settingsSection(tab: SettingsTab): SettingsSection {
  return (
    SETTINGS_SECTIONS.find((item) => item.key === tab) ?? SETTINGS_SECTIONS[0]
  );
}

/** 分类栏按组展示；顺序即 SETTINGS_SECTIONS 的顺序。 */
export const SETTINGS_GROUPS: readonly {
  label: string;
  items: readonly SettingsSection[];
}[] = Array.from(new Set(SETTINGS_SECTIONS.map((item) => item.group))).map(
  (label) => ({
    label,
    items: SETTINGS_SECTIONS.filter((item) => item.group === label),
  }),
);
