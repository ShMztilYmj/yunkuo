import { Boxes, ChartNoAxesCombined, Plug, Rocket } from "lucide-react";
import PageTabs from "../../../shared/ui/PageTabs";

export const AI_SUB = [
  { to: "/ai", label: "启动工具", icon: Rocket },
  { to: "/ai-providers", label: "服务商与 Agent", icon: Boxes },
  { to: "/ai-stats", label: "用量与统计", icon: ChartNoAxesCombined },
  { to: "/ai-capabilities", label: "MCP 与 Skills", icon: Plug },
];

/** AI 子页导航：与代理页签同一套版式，并进各子页页头右侧。 */
export default function AiTabs() {
  return <PageTabs label="AI 管理导航" tabs={AI_SUB} />;
}
