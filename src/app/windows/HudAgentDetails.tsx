import { ArrowUpRight } from "lucide-react";
import AgentBot from "../../shared/ui/AgentBot";
import {
  botStateForSession,
  agentBotId,
} from "../../features/ai/services/agentBots";
import {
  formatTokens,
  localUsageSupported,
  recentSessions,
  sessionState,
} from "../../features/ai/services/aiStats";
import ProviderQuotaPanel from "../../features/ai/components/ProviderQuotaPanel";
import { useRemovedProviders } from "../../features/ai/hooks/useRemovedProviders";
import { visibleProviders } from "../../features/ai/services/providerVisibility";
import {
  accountQuotaView,
  activeProviderOf,
  providerQuotaView,
} from "../../features/ai/services/providerAccounts";
import type { AiUsageItem } from "../../services/api";

export type HudDetailView = "usage" | "sessions";
export default function HudAgentDetails({
  item,
  tool,
  label,
  now,
  view,
  onOpen,
}: {
  item?: AiUsageItem;
  tool: string;
  label: string;
  now: number;
  view: HudDetailView;
  onOpen: () => void;
}) {
  const removed = useRemovedProviders();
  const sessions = recentSessions(item ? [item] : [], now),
    // 在「服务商与 Agent」里移除过的模型源，悬窗里同样不再列出。
    providers = visibleProviders(
      item?.key ?? tool,
      item?.providers ?? [],
      removed,
    ),
    // 账号额度（官方订阅窗口）优先；模型源额度只取 Agent 当前在用的那一个。
    accountQuota = accountQuotaView(item),
    active = activeProviderOf(providers),
    providerQuota = active
      ? providerQuotaView(active, item?.providerSync)
      : null,
    quota = accountQuota ?? providerQuota,
    accountName = item?.account?.email ?? item?.account?.name ?? null,
    quotaTitle = accountQuota
      ? (accountName ?? label)
      : (active?.label ?? undefined);
  const usage = item?.usageAvailable;
  return (
    <section
      key={`${tool}:${view}`}
      className="hud-tool-detail"
      data-view={view}
      aria-label={`${label}统计`}
    >
      {view === "usage" ? (
        <>
          <div className="hud-quota-list" aria-label={`${label}额度窗口`}>
            {quota ? (
              <ProviderQuotaPanel
                quota={quota}
                title={quotaTitle}
                badge={!accountQuota && active?.active ? "使用中" : undefined}
              />
            ) : (
              <div className="hud-quota-empty">
                <span>额度</span>
                <strong>—</strong>
                <p>暂无额度快照</p>
              </div>
            )}
          </div>
          <div className="hud-periods" aria-label={`${label}用量汇总`}>
            {(
              [
                ["today", "今日"],
                ["last7", "近 7 天"],
              ] as const
            ).map(([period, title]) => (
              <button
                key={period}
                data-period={period}
                onClick={onOpen}
                aria-label={`查看${label}${title}统计`}
              >
                <span>{title}</span>
                <strong>
                  <b>
                    {usage && item?.[period]
                      ? formatTokens(item[period]!.total)
                      : "—"}
                  </b>
                  <small> Token</small>
                </strong>
                <span className="hud-cost" title="暂无费用数据">
                  费用 —
                </span>
                <ArrowUpRight size={12} />
              </button>
            ))}
          </div>
          <p className="hud-source-note">
            {item
              ? `${item.dataSource || "用量记录"}${!item.complete && usage ? " · 部分统计" : ""}`
              : localUsageSupported(tool)
                ? "暂无本地数据"
                : "用量未接入"}
            {accountQuota && item?.rate.accountVerified !== true
              ? " · 额度未核验账号"
              : ""}
          </p>
        </>
      ) : (
        <section className="hud-sessions" aria-label={`${label}最近会话`}>
          <div className="hud-section-heading">
            <h2>会话详情</h2>
            <span>{sessions.length} 个近期会话</span>
          </div>
          {sessions.length ? (
            sessions.slice(0, 8).map((session) => (
              <button
                key={session.id}
                className="hud-session"
                onClick={onOpen}
                data-tooltip="查看会话"
                data-tooltip-detail="点击在 AI 统计中查看会话详情。"
              >
                <AgentBot
                  bot={agentBotId(tool)}
                  state={botStateForSession(session, now)}
                  size={32}
                />
                <span>
                  <strong title={session.project}>{session.project}</strong>
                  <small>
                    {session.model || label} ·{" "}
                    {session.usageAvailable === false
                      ? "用量 —"
                      : `${formatTokens(session.total)} Token`}
                  </small>
                  <small>
                    {new Date(session.lastActivityAt * 1000).toLocaleString(
                      "zh-CN",
                      {
                        month: "numeric",
                        day: "numeric",
                        hour: "2-digit",
                        minute: "2-digit",
                        hour12: false,
                      },
                    )}
                  </small>
                </span>
                <em data-state={sessionState(session, now)}>
                  {
                    {
                      running: "工作中",
                      asking: "等待确认",
                      completed: "完成",
                      ended: "结束",
                      unknown: "未知",
                    }[sessionState(session, now)]
                  }
                </em>
              </button>
            ))
          ) : (
            <div className="hud-session-empty">
              <strong>还没有可显示的会话</strong>
              <p>
                {item?.note || "使用这个 AI 工具后，可在这里查看最近的会话。"}
              </p>
            </div>
          )}
        </section>
      )}
    </section>
  );
}
