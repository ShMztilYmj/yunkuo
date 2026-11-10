import { Plug } from "lucide-react";
import CapabilityPanel from "../components/CapabilityPanel";
import PageHeader from "../../../shared/ui/PageHeader";
import AiTabs from "../components/AiTabs";

/** MCP 与 Skills 能力库：独立页面，页头与另外三个模块共用同一套版式。 */
export default function AiCapabilities() {
  return (
    <div className="capability-page">
      <PageHeader
        icon={Plug}
        title="MCP 与 Skills"
        meta={<span>按工具启用，写入各工具用户级配置</span>}
        tabs={<AiTabs />}
      />
      <CapabilityPanel />
    </div>
  );
}
