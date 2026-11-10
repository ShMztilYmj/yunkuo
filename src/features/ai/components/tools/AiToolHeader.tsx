import { Plus, Rocket } from "lucide-react";
import PageHeader from "../../../../shared/ui/PageHeader";
import AiTabs from "../AiTabs";

/**
 * AI 启动工具页头：标题 / 卡组摘要 / 添加工具 + 模块页签，
 * 版式与尺寸交给共用 <PageHeader />，与代理、检测、指纹浏览器完全对齐。
 */
export default function AiToolHeader({
  adding,
  canAdd,
  count,
  scanned,
  pass,
  onToggleAdd,
}: {
  adding: boolean;
  canAdd: boolean;
  count: number;
  scanned: boolean;
  pass: boolean;
  onToggleAdd: () => void;
}) {
  const gate = scanned
    ? pass
      ? { text: "环境已通过", tone: "is-ready" }
      : { text: "环境未达标", tone: "is-warning" }
    : { text: "环境尚未检测", tone: "" };
  return (
    <PageHeader
      icon={Rocket}
      title="AI 启动工具"
      meta={
        <>
          <span>{count} 张工具卡</span>
          <i className="proxy-meta-dot" aria-hidden="true" />
          <span>Claude 与 Codex 默认保留</span>
          <i className="proxy-meta-dot" aria-hidden="true" />
          <span>与悬浮窗同步</span>
        </>
      }
      actions={
        <button
          type="button"
          className="proxy-action"
          aria-expanded={adding}
          aria-controls="ai-tool-picker"
          disabled={!canAdd}
          onClick={onToggleAdd}
        >
          <Plus size={15} />
          添加工具
        </button>
      }
      tabs={<AiTabs />}
      note={
        <>
          <span>选一张卡片，开始下一次创作。启动前会自动复检环境。</span>
          <span className={`ai-tools-gate ${gate.tone}`}>
            <i />
            {gate.text}
          </span>
        </>
      }
    />
  );
}
