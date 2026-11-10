import { ShieldCheck } from "lucide-react";
import type { CSSProperties } from "react";
import { Link } from "react-router-dom";
import type { AppConfig } from "../../../services/api";
import Card from "./Card";
import Note from "./Note";
import Row from "./Row";
import Switch from "./Switch";
import type { SettingsUpdater } from "./types";

/** 环境门控规则：风险评分阈值、服务可达要求、禁止启动的出口地区。 */
export default function GateRulesCard({
  config,
  onUpdate,
}: {
  config: AppConfig;
  onUpdate: SettingsUpdater;
}) {
  const fill = ((config.gate_threshold - 10) / 60) * 100;
  return (
    <Card
      title="环境门控规则"
      icon={<ShieldCheck />}
      description="检测评分、服务连通性与出口地区共同决定是否放行启动。"
      badge={`阈值 ${config.gate_threshold} 分`}
      badgeTone="idle"
    >
      <Row label="风险评分阈值" hint="分数越低要求越严格；改动立即生效" wide>
        <div className="settings-slider">
          <input
            type="range"
            min={10}
            max={70}
            step={1}
            value={config.gate_threshold}
            aria-label="风险评分阈值"
            className="settings-range"
            style={{ "--range-fill": `${fill}%` } as CSSProperties}
            onChange={(e) => onUpdate("gate_threshold", Number(e.target.value))}
          />
          <span className="settings-slider-value">
            {config.gate_threshold}
            <small>分</small>
          </span>
        </div>
      </Row>
      <Row
        label="要求工具服务可达"
        hint="Claude / Codex 启动时按连接配置检查；检测页保留 Claude 参考结果"
      >
        <Switch
          v={config.require_claude_reachable}
          label="切换工具服务可达要求"
          on={(b) => onUpdate("require_claude_reachable", b)}
        />
      </Row>
      <Row
        label="禁止启动的出口地区"
        hint="两位国家代码，逗号分隔；命中任一地区则阻止启动"
        wide
      >
        <input
          className="settings-input is-mono"
          aria-label="禁止启动的出口地区"
          placeholder="例如：IR, KP"
          key={`blocked-${config.blocked_countries.join(",")}`}
          defaultValue={config.blocked_countries.join(",")}
          spellCheck={false}
          autoComplete="off"
          onBlur={(e) =>
            onUpdate(
              "blocked_countries",
              e.target.value
                .split(",")
                .map((s) => s.trim().toUpperCase())
                .filter(Boolean),
            )
          }
        />
      </Row>
      <Note>
        阈值同时作用于 AI 管理中的工具启动与首页的一键启动；改动后立即生效。
        <Link to="/detect">打开检测与修复</Link>
      </Note>
    </Card>
  );
}
