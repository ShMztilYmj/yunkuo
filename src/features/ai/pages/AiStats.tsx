import { useCallback, useEffect, useRef, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import {
  AlertTriangle,
  ArrowRight,
  Boxes,
  ChartNoAxesCombined,
  Loader2,
  PanelTop,
  RefreshCw,
} from "lucide-react";
import { type AiUsageItem } from "../../../services/api";
import { readAiUsage } from "../services/aiUsage";
import AiMonitorControls from "../components/AiMonitorControls";
import AgentHudSettings from "../components/AgentHudSettings";
import AgentBot from "../../../shared/ui/AgentBot";
import {
  agentBotId,
  botStateForSession,
  botStateForUsage,
} from "../services/agentBots";
import { usePolling } from "../../../shared/hooks/usePolling";
import {
  AI_PERIODS,
  cacheShare,
  formatTokens as tokens,
  localUsageSupported,
  recentSessions,
  sessionState,
  trendDays,
  usageFor,
  type AiPeriod,
} from "../services/aiStats";
import { aiToolCards } from "../services/aiToolCards";
import { useAppStore } from "../../../store";
import AgentIntegrations from "../components/AgentIntegrations";
import StatsPeriodTabs from "../components/StatsPeriodTabs";
import PageHeader from "../../../shared/ui/PageHeader";
import AiTabs from "../components/AiTabs";

const integer = new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 0 });

export default function AiStats() {
  const [searchParams, setSearchParams] = useSearchParams();
  // 旧链接里的 ?tab=quotas 已随「账号额度」页合并到服务商页，这里只保留用量与悬浮条。
  const tab = searchParams.get("tab") === "hud" ? "hud" : "tools";

  return (
    <div className="ai-stats-page">
      <PageHeader
        icon={ChartNoAxesCombined}
        title="AI 统计"
        meta={<span>用量、趋势与工作状态，一眼掌握</span>}
        tabs={<AiTabs />}
      />
      <div
        role="group"
        aria-label="AI 统计视图"
        className="settings-tabs flex flex-wrap gap-1"
      >
        {(
          [
            ["tools", <ChartNoAxesCombined size={16} />, "用量统计"],
            ["hud", <PanelTop size={16} />, "悬浮条设置"],
          ] as const
        ).map(([id, icon, label]) => (
          <button
            key={label}
            aria-pressed={id === tab}
            aria-controls="ai-stats-content"
            onClick={() => setSearchParams(id === "tools" ? {} : { tab: id })}
            className={`flex items-center gap-1.5 rounded-lg px-3 py-2 text-sm ${id === tab ? "bg-zinc-900 font-medium text-zinc-100 shadow-sm" : "text-zinc-400 hover:bg-zinc-800/50 hover:text-zinc-200"}`}
          >
            {icon}
            {label}
          </button>
        ))}
      </div>
      <div id="ai-stats-content">
        {tab === "hud" ? (
          <div className="max-w-4xl">
            <AgentHudSettings />
          </div>
        ) : (
          <AiUsageStats />
        )}
      </div>
    </div>
  );
}

function AiUsageStats() {
  const config = useAppStore((state) => state.config);
  const TOOLS = aiToolCards(config).map((tool) => ({
    ...tool,
    color:
      tool.key === "claude"
        ? "bg-orange-400"
        : tool.key === "chatgpt"
          ? "bg-emerald-400"
          : tool.key === "deepseek"
            ? "bg-blue-400"
            : "bg-zinc-500",
  }));
  const scopeKey = TOOLS.map((tool) => tool.key).join(",");
  const [items, setItems] = useState<AiUsageItem[] | null>(null);
  const [period, setPeriod] = useState<AiPeriod>("today");
  const [trendPeriod, setTrendPeriod] = useState<7 | 30>(7);
  const [allSessions, setAllSessions] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [readAt, setReadAt] = useState<number | null>(null);
  const [now, setNow] = useState(() => Date.now() / 1000);
  const pending = useRef(0);
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const refresh = useCallback(
    async (isCurrent: () => boolean = () => true, force = true) => {
      pending.current++;
      setRefreshing(true);
      try {
        const snapshot = await readAiUsage(force);
        if (!mounted.current || !isCurrent()) return;
        setItems(snapshot.items);
        setReadAt(Date.now());
        setNow(Date.now() / 1000);
        setError(null);
      } catch (failure) {
        if (mounted.current && isCurrent())
          setError(`读取统计失败：${String(failure)}`);
      } finally {
        pending.current--;
        if (mounted.current && pending.current === 0) setRefreshing(false);
      }
    },
    [],
  );
  usePolling(
    (isCurrent) => refresh(isCurrent, false),
    60_000,
    true,
    !!config,
    scopeKey,
  );
  usePolling(
    async () => {
      setNow(Date.now() / 1000);
    },
    30_000,
    false,
  );

  const rows = TOOLS.map((tool) => ({
    ...tool,
    item: items?.find((item) => item.key === tool.key),
  }));
  const available = rows.flatMap((row) =>
    row.item?.usageAvailable ? [row.item] : [],
  );
  const partial = available.some((item) => !item.complete);
  const periodItems = available.flatMap((item) => {
    const usage = usageFor(item, period);
    return usage ? [usage] : [];
  });
  const sum = (field: "total" | "input" | "output" | "sessions") =>
    periodItems.reduce((value, usage) => value + usage[field], 0);
  const total = sum("total");
  const hasData = periodItems.length > 0;
  const label = AI_PERIODS.find((value) => value.key === period)!.label;
  const sessions = recentSessions(
    (items ?? []).filter((item) => TOOLS.some((tool) => tool.key === item.key)),
    now,
  );
  const running = sessions.filter(
    (session) => sessionState(session, now) === "running",
  ).length;
  const days = trendDays(available, trendPeriod, new Date(now * 1000));
  const maxDay = Math.max(1, ...days.map((day) => day.total));
  const hasTrend = available.some((item) => item.daily?.length);

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <StatsPeriodTabs<AiPeriod>
          items={AI_PERIODS}
          value={period}
          onChange={setPeriod}
          label="统计时段"
        />
        <div className="flex flex-wrap items-center gap-3">
          <span className="text-[11px] text-zinc-500">
            {readAt
              ? `读取于 ${new Date(readAt).toLocaleTimeString("zh-CN", { hour12: false })} · 每分钟刷新`
              : "按工具来源读取数据"}
          </span>
          <button
            className="btn-secondary flex items-center gap-2 rounded-xl px-3 py-2 text-xs disabled:opacity-50"
            disabled={refreshing}
            onClick={() => void refresh()}
          >
            {refreshing ? (
              <Loader2 size={14} className="animate-spin" />
            ) : (
              <RefreshCw size={14} />
            )}
            刷新统计
          </button>
        </div>
      </div>
      {error && (
        <div
          role="alert"
          className="rounded-xl border border-amber-700/40 bg-amber-500/10 p-3 text-xs leading-relaxed text-amber-300"
        >
          {error}。
          {items
            ? "下方保留上次成功读取的数据，请刷新重试。"
            : "请点击刷新统计重试。"}
        </div>
      )}
      {items === null ? (
        !error && (
          <div
            role="status"
            className="flex items-center gap-2 py-10 text-sm text-zinc-500"
          >
            <Loader2 size={16} className="animate-spin" />
            正在读取本地统计…
          </div>
        )
      ) : (
        <>
          {partial && (
            <div
              role="status"
              className="flex items-start gap-2 rounded-xl border border-amber-700/40 bg-amber-500/10 p-3 text-xs text-amber-300"
            >
              <AlertTriangle size={15} className="shrink-0" />
              部分本地记录无法完整统计，汇总仅包含可核验用量。
            </div>
          )}
          {!available.length && (
            <div
              role="status"
              className="rounded-xl border border-zinc-800 p-4 text-sm text-zinc-500"
            >
              暂无可核验的本地 Token
              记录。使用已连接工具后刷新；仅有工作状态时会话仍会显示。
            </div>
          )}
          <div
            className="grid grid-cols-2 gap-3 md:grid-cols-4"
            aria-label={`${label}用量汇总`}
          >
            {(
              [
                ["总 Token", total, false],
                ["输入 Token", sum("input"), false],
                ["输出 Token", sum("output"), false],
                ["有用量会话", sum("sessions"), true],
              ] as const
            ).map(([name, value, exact], index) => (
              <section
                key={name}
                aria-label={name}
                className={`ui-panel ai-metric min-w-0 border p-4 ${index === 0 ? "ai-metric-primary" : ""}`}
              >
                <p className="text-xs text-zinc-500">
                  {label} · {name}
                  {partial ? "（部分）" : ""}
                </p>
                <p
                  className="mt-2 truncate text-2xl font-semibold tabular-nums"
                  data-tooltip={hasData ? integer.format(value) : "暂无数据"}
                >
                  {hasData
                    ? exact
                      ? integer.format(value)
                      : tokens(value)
                    : "—"}
                </p>
              </section>
            ))}
          </div>
          <div className="grid grid-cols-1 gap-4">
            <section
              aria-label="用量趋势"
              className="ui-panel min-w-0 border p-4"
            >
              <div className="flex items-center justify-between gap-2">
                <h2 className="text-sm font-medium">用量趋势</h2>
                <div
                  role="group"
                  aria-label="趋势时段"
                  className="flex gap-1 text-[11px]"
                >
                  {([7, 30] as const).map((count) => (
                    <button
                      key={count}
                      aria-pressed={trendPeriod === count}
                      onClick={() => setTrendPeriod(count)}
                      className={`rounded-lg px-2 py-1 ${trendPeriod === count ? "bg-zinc-800 text-zinc-100" : "text-zinc-500"}`}
                    >
                      {count} 天
                    </button>
                  ))}
                </div>
              </div>
              {hasTrend ? (
                <>
                  <div
                    className="ai-trend mt-4"
                    role="list"
                    aria-label={`近 ${trendPeriod} 天每日 Token`}
                  >
                    {days.map((day, index) => (
                      <div
                        key={day.date}
                        className="ai-trend-column"
                        role="listitem"
                        aria-label={`${day.date}：${integer.format(day.total)} Token`}
                      >
                        <div
                          className="ai-trend-track"
                          tabIndex={0}
                          aria-label={`${day.date} 用量 ${integer.format(day.total)} Token`}
                        >
                          <div
                            className="ai-trend-stack"
                            style={{ height: `${(day.total / maxDay) * 100}%` }}
                          >
                            {day.values
                              .filter((value) => value.total > 0)
                              .map((value) => (
                                <span
                                  key={value.key}
                                  className={
                                    TOOLS.find((tool) => tool.key === value.key)
                                      ?.color
                                  }
                                  style={{ flex: value.total }}
                                />
                              ))}
                          </div>
                          <div className="ai-trend-tooltip">
                            <strong>
                              {day.label} · {tokens(day.total)}
                            </strong>
                            {day.values
                              .filter((value) => value.total > 0)
                              .map((value) => (
                                <span key={value.key}>
                                  {
                                    TOOLS.find((tool) => tool.key === value.key)
                                      ?.label
                                  }{" "}
                                  {tokens(value.total)}
                                </span>
                              ))}
                          </div>
                        </div>
                        <span className="ai-trend-date">
                          {trendPeriod === 7 ||
                          (index % 7 === 0 && index < days.length - 2) ||
                          index === days.length - 1
                            ? day.label
                            : ""}
                        </span>
                      </div>
                    ))}
                  </div>
                  <div className="mt-3 flex flex-wrap justify-between gap-2 text-[11px] text-zinc-500">
                    <span>
                      近 {trendPeriod} 天 ·{" "}
                      {tokens(days.reduce((sum, day) => sum + day.total, 0))}{" "}
                      Token{partial ? "（部分）" : ""}
                    </span>
                    <div className="flex gap-3">
                      {rows
                        .filter((row) => row.item?.usageAvailable)
                        .map((row) => (
                          <span
                            key={row.key}
                            className="flex items-center gap-1.5"
                          >
                            <i
                              className={`h-1.5 w-1.5 rounded-full ${row.color}`}
                            />
                            {row.label}
                          </span>
                        ))}
                    </div>
                  </div>
                </>
              ) : (
                <p className="flex h-36 items-center justify-center text-xs text-zinc-500">
                  暂无每日用量记录
                </p>
              )}
            </section>
          </div>
          <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
            <section aria-label="用量占比" className="ui-panel border p-4">
              <h2 className="text-sm font-medium">
                {label}用量占比{partial ? "（部分）" : ""}
              </h2>
              {total > 0 ? (
                <div className="mt-4 space-y-4">
                  {rows
                    .filter((row) => usageFor(row.item!, period))
                    .map((row) => {
                      const value = usageFor(row.item!, period)!.total,
                        share = (value / total) * 100;
                      return (
                        <div key={row.key}>
                          <div className="mb-1.5 flex justify-between text-xs">
                            <span>{row.label}</span>
                            <span className="text-zinc-500">
                              {tokens(value)} · {share.toFixed(1)}%
                            </span>
                          </div>
                          <div
                            role="meter"
                            aria-label={`${row.label}用量占比`}
                            aria-valuemin={0}
                            aria-valuemax={100}
                            aria-valuenow={Number(share.toFixed(1))}
                            className="h-2 overflow-hidden rounded-full bg-zinc-800"
                          >
                            <div
                              className={`h-full rounded-full ${row.color}`}
                              style={{ width: `${share}%` }}
                            />
                          </div>
                        </div>
                      );
                    })}
                </div>
              ) : (
                <p className="mt-4 text-xs text-zinc-500">
                  {hasData ? "该时段暂无 Token 用量" : "暂无可用数据"}
                </p>
              )}
            </section>
            <section aria-label="缓存利用率" className="ui-panel border p-4">
              <h2 className="text-sm font-medium">缓存利用率</h2>
              <p className="mt-1 text-[11px] text-zinc-500">
                缓存读取占全部输入 Token 的比例
              </p>
              <div className="mt-4 grid grid-cols-2 gap-3">
                {rows
                  .filter((row) => row.item?.usageAvailable)
                  .map((row) => {
                    const cache = cacheShare(row.item!, period);
                    return (
                      <div
                        key={row.key}
                        className="rounded-xl bg-zinc-800/40 p-3"
                      >
                        <div className="text-xs text-zinc-400">{row.label}</div>
                        <p
                          className="mt-1.5 text-xl font-semibold tabular-nums"
                          aria-label={`${row.label}缓存利用率`}
                        >
                          {cache ? `${cache.percent.toFixed(1)}%` : "—"}
                        </p>
                        <p className="mt-1 text-[10px] text-zinc-500">
                          {cache
                            ? `${tokens(cache.cached)} / ${tokens(cache.input)} 输入`
                            : "该时段无输入记录"}
                        </p>
                      </div>
                    );
                  })}
              </div>
            </section>
          </div>
          <section aria-label="最近会话" className="ui-panel border p-4">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <h2 className="text-sm font-medium">最近会话</h2>
              <span
                className={`rounded-full px-2 py-1 text-[11px] ${running ? "bg-emerald-500/10 text-emerald-400" : "bg-zinc-800/50 text-zinc-500"}`}
              >
                已识别运行中 {running}
              </span>
            </div>
            <p className="mt-1 text-[11px] text-zinc-500">
              按会话事件识别状态；30 分钟没有新证据时显示未知，默认显示最近 6
              个主会话。
            </p>
            {sessions.length ? (
              <div className="mt-3 divide-y divide-zinc-800/70">
                {(allSessions ? sessions : sessions.slice(0, 6)).map(
                  (session) => {
                    const state = sessionState(session, now);
                    return (
                      <div
                        key={session.id}
                        data-session-bot={agentBotId(session.tool)}
                        className="flex items-center justify-between gap-3 py-2.5 text-xs"
                      >
                        <div className="flex min-w-0 items-center gap-1.5">
                          <AgentBot
                            bot={agentBotId(session.tool)}
                            size={38}
                            state={botStateForSession(session, now)}
                          />
                          <div className="min-w-0">
                            <div className="truncate font-medium">
                              {session.project}
                            </div>
                            <p className="mt-1 truncate text-[10px] text-zinc-500">
                              {TOOLS.find((tool) => tool.key === session.tool)
                                ?.label ?? session.tool}
                              {session.model ? ` · ${session.model}` : ""} ·{" "}
                              {new Date(
                                session.lastActivityAt * 1000,
                              ).toLocaleString("zh-CN", {
                                month: "numeric",
                                day: "numeric",
                                hour: "2-digit",
                                minute: "2-digit",
                              })}
                            </p>
                          </div>
                        </div>
                        <div className="flex shrink-0 items-center gap-3">
                          <span className="text-zinc-500">
                            {session.usageAvailable === false
                              ? "用量 —"
                              : `${tokens(session.total)} Token`}
                          </span>
                          <span className={`ai-session-state ${state}`}>
                            <i />
                            {
                              {
                                running: "运行中",
                                asking: "等待确认",
                                completed: "已完成",
                                ended: "已结束",
                                unknown: "状态未知",
                              }[state]
                            }
                          </span>
                        </div>
                      </div>
                    );
                  },
                )}
              </div>
            ) : (
              <p className="mt-4 text-xs text-zinc-500">暂无可识别的会话事件</p>
            )}
            {sessions.length > 6 && (
              <button
                className="btn-secondary mt-3 rounded-lg px-3 py-1.5 text-xs"
                aria-expanded={allSessions}
                onClick={() => setAllSessions(!allSessions)}
              >
                {allSessions
                  ? "收起会话"
                  : `查看全部 ${sessions.length} 个会话`}
              </button>
            )}
            <AiMonitorControls />
          </section>
          <section
            aria-label="工具用量明细"
            className="ui-panel overflow-hidden border"
          >
            <div className="flex flex-wrap items-center justify-between gap-2 border-b border-zinc-800 px-4 py-3">
              <h2 className="text-sm font-medium">工具用量</h2>
              <span className="text-[11px] text-zinc-500">
                {label} · {available.length} 个数据源已接入
              </span>
            </div>
            <div className="overflow-x-auto">
              <table className="w-full min-w-[660px] text-left text-xs">
                <caption className="sr-only">
                  {label}各工具的本地 Token 和会话统计
                </caption>
                <thead className="bg-zinc-950/50 text-[11px] text-zinc-500">
                  <tr>
                    {[
                      "工具 / 来源",
                      "总 Token",
                      "输入",
                      "输出",
                      "缓存读取",
                      "缓存写入",
                      "会话",
                    ].map((name) => (
                      <th
                        key={name}
                        scope="col"
                        className="whitespace-nowrap px-4 py-3 font-normal"
                      >
                        {name}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {rows.map((row) => {
                    const usage = row.item ? usageFor(row.item, period) : null;
                    return (
                      <tr key={row.key} className="border-t border-zinc-800/60">
                        <th
                          scope="row"
                          className="min-w-44 px-4 py-3 font-normal"
                        >
                          <div className="flex items-center gap-1 font-medium">
                            {agentBotId(row.key) ? (
                              <AgentBot
                                bot={agentBotId(row.key)}
                                size={32}
                                state={botStateForUsage(row.item, now)}
                              />
                            ) : (
                              <span
                                className={`h-2 w-2 rounded-full ${row.color}`}
                              />
                            )}
                            {row.label}
                          </div>
                          <div className="mt-1 text-[10px] text-zinc-500">
                            {usage
                              ? row.item?.dataSource
                              : localUsageSupported(row.key)
                                ? "暂无本地数据"
                                : "用量未接入"}
                            {usage && !row.item?.complete
                              ? " · 统计不完整"
                              : ""}
                          </div>
                        </th>
                        {(
                          [
                            "total",
                            "input",
                            "output",
                            "cached",
                            "cacheWrite",
                            "sessions",
                          ] as const
                        ).map((field) => (
                          <td
                            key={field}
                            className="whitespace-nowrap px-4 py-3 tabular-nums"
                            data-tooltip={
                              usage ? integer.format(usage[field]) : "未接入"
                            }
                          >
                            {usage
                              ? field === "sessions"
                                ? integer.format(usage[field])
                                : tokens(usage[field])
                              : "—"}
                          </td>
                        ))}
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
            <div className="space-y-1 border-t border-zinc-800 px-4 py-3 text-[11px] leading-relaxed text-zinc-500">
              <p>
                按本机时区统计，本周从周一开始，近 7 / 30
                天包含今天；累计覆盖当前来源仍保留的记录，云端来源可能包含其他设备，不按当前登录账号分账。有用量会话不代表正在运行。
              </p>
              <p>
                Codex 数据来自本地会话，不包含 ChatGPT
                网页或桌面聊天；缓存字段可能已计入输入或总量，请勿再次相加。
              </p>
              {available
                .filter((item) => !item.complete)
                .map((item) => (
                  <p key={item.key} className="text-amber-300">
                    {TOOLS.find((tool) => tool.key === item.key)?.label}：
                    {item.note}
                  </p>
                ))}
            </div>
          </section>
        </>
      )}
      <AgentIntegrations onChanged={() => void refresh()} />
      <div className="flex flex-wrap gap-x-5 gap-y-2 text-xs text-zinc-500">
        <Link to="/ai" className="flex items-center gap-1 hover:text-zinc-200">
          AI 启动工具
          <ArrowRight size={13} />
        </Link>
        <Link
          to="/ai-providers"
          className="flex items-center gap-1 hover:text-zinc-200"
        >
          <Boxes size={13} />
          服务商与 Agent
          <ArrowRight size={13} />
        </Link>
        <Link
          to="/ai-capabilities"
          className="flex items-center gap-1 hover:text-zinc-200"
        >
          MCP 与 Skills
          <ArrowRight size={13} />
        </Link>
      </div>
    </div>
  );
}
