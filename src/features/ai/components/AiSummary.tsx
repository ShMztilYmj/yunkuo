import { useState } from "react";
import { Link } from "react-router-dom";
import {
  Activity,
  ArrowRight,
  ArrowUpRight,
  ChartNoAxesCombined,
  Gauge,
} from "lucide-react";
import type { AiSession } from "../../../services/api";
import type { AiUsageItem } from "../../../services/api";
import {
  formatTokens,
  quotaResetText,
  quotaView,
  recentSessions,
  sessionState,
} from "../services/aiStats";
import { readAiUsage } from "../services/aiUsage";
import { usePolling } from "../../../shared/hooks/usePolling";
import { useToolSetup } from "../hooks/useToolSetup";
import { AI_TOOL_METADATA, aiToolCards } from "../services/aiToolCards";

import { useAppStore } from "../../../store";

const QUOTA_FRESH_SECONDS = 1800;

interface AiSummaryProps {
  /** 首页页头的「刷新状态」会递增这个值，强制重新读取一次本机用量。 */
  refreshToken?: number;
}

function sessionAge(seconds: number, now: number) {
  const diff = Math.max(0, now - seconds);
  if (diff < 60) return "刚刚";
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`;
  if (diff < 86_400) return `${Math.floor(diff / 3600)} 小时前`;
  return `${Math.floor(diff / 86_400)} 天前`;
}

function sessionStateText(session: AiSession, now: number) {
  const state = sessionState(session, now);
  if (state === "running") return "运行中";
  if (state === "asking") return "等待确认";
  if (state === "completed") return "已完成";
  if (state === "ended") return "已结束";
  return "活动未知";
}

/**
 * 首页的 AI 概览：把「今天用了多少、有没有在跑、额度还剩多少」放在一张卡里。
 * 卡片本身不负责启动工具，底部统一跳到 AI 工具页，由工具页处理启动与门控。
 */
export default function AiSummary({ refreshToken = 0 }: AiSummaryProps) {
  const config = useAppStore((state) => state.config);
  const keys = aiToolCards(config).map((tool) => tool.key as string);
  const tools = aiToolCards(config);
  const [items, setItems] = useState<AiUsageItem[] | null>(null);
  const [error, setError] = useState(false);
  usePolling(
    async (isCurrent) => {
      try {
        const snapshot = await readAiUsage();
        if (isCurrent()) {
          setItems(snapshot.items);
          setError(false);
        }
      } catch {
        if (isCurrent()) setError(true);
      }
    },
    60_000,
    true,
    !!config,
    `${keys.join(",")}:${refreshToken}`,
  );
  const now = Date.now() / 1000;
  const selectedItems = items?.filter((item) => keys.includes(item.key)) ?? [];
  const available = selectedItems.filter((item) => item.usageAvailable);
  const partial = available.some((item) => !item.complete);
  // 「运行中」以 Agent 进程为准（与 AI 工具页一致）：某个工具进程在线，即使它
  // 最近一条会话已经结束，也应该算在运行中。
  const { snapshot: toolSnapshot } = useToolSetup();
  const onlineKeys = new Set(
    (toolSnapshot?.tools ?? [])
      .filter((tool) => tool.running === true && keys.includes(tool.key))
      .map((tool) => tool.key),
  );
  // 额度取「最新采集到的那份」：某个工具的快照过期时，别让它盖住其它工具的实时额度。
  const quotaPool = available.filter((item) => item.rate.primary);
  const quota =
    quotaPool.find(
      (item) => now - (item.rate.capturedAt ?? 0) <= QUOTA_FRESH_SECONDS,
    ) ??
    quotaPool
      .slice()
      .sort((a, b) => (b.rate.capturedAt ?? 0) - (a.rate.capturedAt ?? 0))[0] ??
    null;
  const view = quota?.rate.primary
    ? quotaView(quota.rate.primary, quota.rate, quota.scannedAt, now)
    : null;
  const sessions = recentSessions(selectedItems, now);
  const runningSessions = sessions.filter(
    (session) => sessionState(session, now) === "running",
  );
  const quotaFresh =
    view && !view.stale && !view.resetPassed && view.remaining != null;
  const quotaRunning = quota ? onlineKeys.has(quota.key) : false;
  const quotaTool =
    AI_TOOL_METADATA.find((tool) => tool.key === quota?.key)?.label ??
    quota?.key;
  const totalToday = available.reduce((sum, item) => sum + item.today.total, 0);
  const visibleSessions = sessions.slice(0, 3);
  const toolLabels = new Map<string, string>(
    AI_TOOL_METADATA.map((tool) => [tool.key, tool.short] as const),
  );

  return (
    <section
      className="home-ai-panel home-panel"
      aria-labelledby="home-ai-heading"
    >
      <header className="home-panel-head">
        <div className="home-panel-title">
          <span className="home-panel-icon" aria-hidden="true">
            <ChartNoAxesCombined size={15} />
          </span>
          <div>
            <h2 id="home-ai-heading">AI 工作概览</h2>
            <p>本机用量、会话与额度</p>
          </div>
        </div>
        <Link
          to="/ai-stats"
          className="home-panel-link"
          aria-label="查看 AI 用量统计"
        >
          统计
          <ArrowUpRight size={13} aria-hidden="true" />
        </Link>
      </header>

      <div className="home-ai-total">
        <p className="home-ai-total-label">
          今日 Token
          {partial && <span className="home-ai-partial">部分统计</span>}
        </p>
        <strong>{available.length ? formatTokens(totalToday) : "—"}</strong>
        <span>
          {error
            ? "读取失败，打开统计页重试"
            : items
              ? available.length
                ? "来自已添加的工具"
                : "使用工具后，在这里查看用量"
              : "正在读取本地统计…"}
        </span>
      </div>

      <div className="home-ai-metrics">
        <div className="home-ai-metric">
          <span className="home-ai-metric-label">
            <Activity size={12} aria-hidden="true" />
            运行中工具
          </span>
          <strong>
            {toolSnapshot ? onlineKeys.size : "—"}
            <small>个</small>
          </strong>
          <span className="home-ai-metric-note">
            {onlineKeys.size
              ? "Agent 进程在线"
              : runningSessions.length
                ? "进程未检测到，仅有会话活动"
                : "暂无正在运行的 Agent"}
          </span>
        </div>
        <div className="home-ai-metric" data-tone={view?.tone ?? "muted"}>
          <span className="home-ai-metric-label">
            <Gauge size={12} aria-hidden="true" />
            {quotaTool ? `${quotaTool} 额度` : "额度快照"}
          </span>
          <strong data-quota="true">
            {quotaFresh ? (
              <>
                <small>剩余</small>
                {Math.round(view.remaining!)}
                <em>%</em>
              </>
            ) : view ? (
              "待更新"
            ) : (
              "暂无快照"
            )}
          </strong>
          {quotaFresh ? (
            <>
              <span
                className="home-ai-quota-track"
                role="meter"
                aria-label={`${quotaTool ?? "本机"}额度剩余`}
                aria-valuenow={Math.round(view.remaining!)}
                aria-valuemin={0}
                aria-valuemax={100}
              >
                <i style={{ width: `${Math.min(100, view.remaining!)}%` }} />
              </span>
              <span className="home-ai-metric-note">
                {quotaResetText(view.seconds, view.resetPassed, "short")}
              </span>
            </>
          ) : (
            <span className="home-ai-metric-note">
              {view
                ? quotaRunning
                  ? "Agent 运行中，额度同步中"
                  : "快照已过期，运行工具后更新"
                : "运行工具后生成额度快照"}
            </span>
          )}
        </div>
      </div>

      <div className="home-ai-tools" aria-label="已接入的 AI 工具">
        <span className="home-ai-tools-label">工具</span>
        <div className="home-ai-tool-list">
          {tools.map((tool) => (
            <span
              key={tool.key}
              className="home-ai-tool"
              data-active={onlineKeys.has(tool.key)}
              data-tooltip={tool.label}
            >
              <i aria-hidden="true" />
              {tool.short}
            </span>
          ))}
        </div>
      </div>

      <div className="home-ai-sessions">
        <div className="home-ai-sessions-head">
          <span>最近会话</span>
          <span>{sessions.length ? `${sessions.length} 条` : ""}</span>
        </div>
        {items ? (
          visibleSessions.length ? (
            <ul className="home-ai-session-list">
              {visibleSessions.map((session) => (
                <li className="home-ai-session" key={session.id}>
                  <i
                    className="home-ai-session-dot"
                    data-state={sessionState(session, now)}
                    aria-hidden="true"
                  />
                  <span className="home-ai-session-copy">
                    <strong
                      className="home-ai-session-project"
                      data-tooltip={session.project || undefined}
                    >
                      {session.project || "未命名项目"}
                    </strong>
                    <span className="home-ai-session-desc">
                      {toolLabels.get(session.tool) ?? session.tool}
                      <i aria-hidden="true">·</i>
                      {sessionStateText(session, now)}
                      <i aria-hidden="true">·</i>
                      {sessionAge(session.lastActivityAt, now)}
                    </span>
                  </span>
                </li>
              ))}
            </ul>
          ) : (
            <p className="home-ai-session-empty">还没有可识别的会话</p>
          )
        ) : (
          <p className="home-ai-session-empty">正在读取本地会话…</p>
        )}
      </div>

      <div className="home-ai-actions">
        <Link
          to="/ai"
          className="home-button home-button-primary home-ai-launch"
        >
          打开 AI 工具
          <ArrowRight size={14} aria-hidden="true" />
        </Link>
      </div>
    </section>
  );
}
