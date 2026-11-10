import { ArrowUpRight, Boxes } from "lucide-react";
import { Link } from "react-router-dom";
import type { AppConfig } from "../../../services/api";
import Card from "./Card";
import Note from "./Note";
import Row from "./Row";
import Switch from "./Switch";
import type { SettingsUpdater } from "./types";

/** AI 模块：本地用量扫描，以及指向 AI 管理（启动入口所在）的入口。 */
export default function AiModulesCard({
  config,
  onUpdate,
}: {
  config: AppConfig;
  onUpdate: SettingsUpdater;
}) {
  const scan = config.ai_usage_scan;
  return (
    <Card
      title="AI 工具"
      icon={<Boxes />}
      description="控制本机用量读取；各工具的启动入口与安装检测统一在 AI 管理里配置。"
      badge={scan ? "扫描已开启" : "扫描已关闭"}
      badgeTone={scan ? "ok" : "idle"}
    >
      <Row
        label="本地用量扫描"
        hint="关闭后统计页显示为未开启；工具启动、MCP 与 Skills 同步不受影响"
      >
        <Switch
          v={scan}
          label="切换本地用量扫描"
          on={(b) => onUpdate("ai_usage_scan", b)}
        />
      </Row>
      <Row
        label="启动入口"
        hint="Claude、Codex 等工具的快捷方式、自动定位与启动按钮都在工具卡片上"
      >
        <Link className="settings-action" to="/ai">
          打开 AI 管理
          <ArrowUpRight />
        </Link>
      </Row>
      <Row label="用量与统计" hint="查看本机各工具的用量、额度与统计周期">
        <Link className="settings-action" to="/ai-stats">
          打开统计
          <ArrowUpRight />
        </Link>
      </Row>
      <Note>
        扫描只读取会话文件中的用量字段，不会上传对话内容；关闭后立即停止读取。
      </Note>
    </Card>
  );
}
