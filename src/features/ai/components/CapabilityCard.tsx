import { BookOpen, Plug } from "lucide-react";
import type { Capability } from "../services/capabilities";

interface Props {
  /** 当前选中的能力；为空时牌面一直是背面。 */
  entry: Capability | null;
  /** 当前条目启用的 Agent 名称。 */
  sharedAgents: string[];
  flipped: boolean;
  /** 来源的中文标签，例如「本地文件夹」「Claude Code」。 */
  sourceLabel: string;
  onToggle: () => void;
}

/**
 * 能力库右侧的扑克牌：选中 MCP 或 Skill 后翻到正面，显示能力库维护的完整信息。
 * 具体配置和文件操作仍走表格与编辑面板，这里只做一眼能读完的摘要。
 */
export default function CapabilityCard({
  entry,
  sharedAgents,
  flipped,
  sourceLabel,
  onToggle,
}: Props) {
  const title = entry?.title?.trim() || entry?.name || "—";
  const isMcp = entry?.kind === "mcp";
  const typeLabel = isMcp ? "MCP" : "SKILL";
  const detail = isMcp
    ? [entry?.transport?.toUpperCase(), sourceLabel].filter(Boolean).join(" · ")
    : [`${entry?.fileCount ?? 0} 个文件`, sourceLabel]
        .filter(Boolean)
        .join(" · ");

  return (
    <div
      className={`capability-card${flipped ? " is-flipped" : ""}`}
      data-empty={entry ? undefined : "true"}
    >
      <div
        className="capability-card-body"
        role="button"
        tabIndex={0}
        aria-label={flipped ? "把能力牌面翻回背面" : "翻开能力牌面"}
        onClick={onToggle}
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            onToggle();
          }
        }}
      >
        <div
          className="capability-card-face capability-card-back"
          aria-hidden={flipped}
        >
          <span className="capability-card-corner is-tl">♠</span>
          <span className="capability-card-corner is-tr">♥</span>
          <span className="capability-card-corner is-bl">♦</span>
          <span className="capability-card-corner is-br">♣</span>
          <div className="capability-card-seal">
            <span>能力库</span>
            <small>{entry ? "点开查看牌面" : "点选条目翻开"}</small>
          </div>
        </div>
        <div
          className="capability-card-face capability-card-front"
          data-kind={entry?.kind}
          aria-hidden={!flipped}
        >
          <div className="capability-card-hero">
            <div className="capability-card-head">
              <span className="capability-card-type">
                <span className="capability-card-emblem" aria-hidden="true">
                  {isMcp ? <Plug size={11} /> : <BookOpen size={11} />}
                </span>
                {typeLabel}
                <em>能力卡</em>
              </span>
              <span>点击翻回</span>
            </div>

            <strong
              className="capability-card-title"
              title={entry?.name ?? undefined}
            >
              <span>{title}</span>
              {entry?.name && entry.name !== title && (
                <em className="capability-card-realname">{entry.name}</em>
              )}
            </strong>
            <p className="capability-card-meta">{detail}</p>

            <div className="capability-card-id" title={entry?.id}>
              <span>能力 ID</span>
              <code>{entry?.id ?? "—"}</code>
            </div>
          </div>

          <div className="capability-card-crest" aria-hidden="true">
            <span className="capability-card-crest-ring">
              <span className="capability-card-crest-icon">
                {isMcp ? <Plug size={25} /> : <BookOpen size={25} />}
              </span>
            </span>
            <small>{typeLabel} · 能力档案</small>
          </div>

          <div className="capability-card-story">
            <div className="capability-card-field capability-card-ability">
              <span className="capability-card-label">能力</span>
              <p
                className="capability-card-desc"
                title={entry?.summary || undefined}
              >
                {entry?.summary?.trim() || "未填写能力说明"}
              </p>
            </div>

            <div className="capability-card-field">
              <span className="capability-card-label">详情</span>
              <p
                className="capability-card-detail"
                title={entry?.detail || undefined}
              >
                {entry?.detail?.trim() || "未填写详情"}
              </p>
            </div>
          </div>

          <div className="capability-card-footer">
            {(!!entry?.tags?.length || entry?.repository) && (
              <div className="capability-card-tags">
                {entry?.tags?.map((tag) => (
                  <span key={tag}>{tag}</span>
                ))}
                {entry?.repository && (
                  <a
                    href={entry.repository}
                    target="_blank"
                    rel="noreferrer"
                    onClick={(event) => event.stopPropagation()}
                  >
                    开源项目 ↗
                  </a>
                )}
              </div>
            )}

            <div className="capability-card-shared">
              <span className="capability-card-rule" aria-hidden="true">
                <i />
              </span>
              <span className="capability-card-label">共用 Agent</span>
              <div>
                {sharedAgents.length ? (
                  sharedAgents.map((name) => (
                    <span className="capability-card-agent" key={name}>
                      {name}
                    </span>
                  ))
                ) : (
                  <em>未在 Agent 中启用</em>
                )}
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
