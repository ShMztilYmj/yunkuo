import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import {
  AlertTriangle,
  ArrowRight,
  Boxes,
  Loader2,
  RefreshCw,
  Undo2,
} from "lucide-react";
import { useAppStore } from "../../../store";
import AgentBot from "../../../shared/ui/AgentBot";
import { aiToolCards } from "../services/aiToolCards";
import { agentBotId } from "../services/agentBots";
import {
  balanceLabel,
  buildAgentColumns,
  columnRows,
  formatAmount,
  quotaSummary,
  quotaTone,
  registeredCount,
  summarizeColumns,
  withoutRemovedProviders,
  type AgentProviderColumn,
  type ProviderBinding,
  type ProviderQuotaView,
} from "../services/providerAccounts";
import {
  removeProvider,
  restoreAllProviders,
  restoreProvider,
} from "../services/providerVisibility";
import { useProviderAccounts } from "../hooks/useProviderAccounts";
import { useRemovedProviders } from "../hooks/useRemovedProviders";
import { useToolSetup } from "../hooks/useToolSetup";
import type { ToolSetup } from "../services/toolSetup";
import PageHeader from "../../../shared/ui/PageHeader";
import AiTabs from "../components/AiTabs";
import ProviderDetailCard, {
  type ProviderDetailSelection,
  type RemovedProviderNotice,
} from "../components/ProviderDetailCard";

const LEGEND = [
  ["active", "已核验"],
  ["configured", "已保存"],
  ["unsynced", "待同步"],
  ["empty", "未配置"],
  ["error", "需处理"],
  ["managed", "工具自管"],
] as const;

type RunState = "on" | "off" | "unknown";

/** Agent 进程状态：读不到进程清单时保持未知，不把未知当成已关闭。 */
function runStateOf(tools: ToolSetup[] | undefined, key: string): RunState {
  const tool = tools?.find((item) => item.key === key);
  if (!tool || tool.running == null) return "unknown";
  return tool.running ? "on" : "off";
}

function AgentRunBadge({ state }: { state: RunState }) {
  return (
    <span
      className="ai-providers-run"
      data-state={state}
      data-tooltip={
        state === "on"
          ? "Agent 正在运行：额度按固定间隔实时同步"
          : state === "off"
            ? "Agent 已关闭：额度停止同步，保留最后一次快照"
            : "未读取到 Agent 进程状态"
      }
    >
      <i aria-hidden="true" />
      {state === "on" ? "运行中" : state === "off" ? "已关闭" : "状态未知"}
    </span>
  );
}

/** 单元格里的额度摘要：订阅制给已用百分比，按量付费给余额。 */
function BindingQuota({ quota }: { quota: ProviderQuotaView }) {
  if (!quota.windows.length && !quota.balances.length) return null;
  const summary = quotaSummary(quota) ?? undefined;
  if (quota.billing === "balance")
    return (
      <span className="provider-account-quota" data-tooltip={summary}>
        {quota.balances.map((balance) => (
          <span className="provider-account-quota-item" key={balance.label}>
            {balanceLabel(balance.label)}{" "}
            <b>{formatAmount(balance.amount, balance.currency)}</b>
          </span>
        ))}
      </span>
    );
  return (
    <span className="provider-account-quota" data-tooltip={summary}>
      {quota.windows.map((window, index) => (
        <span
          className="provider-account-quota-item"
          key={`${window.label}:${index}`}
        >
          {window.label}{" "}
          <b data-tone={quotaTone(window.usedPercent)}>
            {Math.round(window.usedPercent)}%
          </b>
        </span>
      ))}
      {quota.balances.map((balance) => (
        <span className="provider-account-quota-item" key={balance.label}>
          {balanceLabel(balance.label)}{" "}
          <b>{formatAmount(balance.amount, balance.currency)}</b>
        </span>
      ))}
    </span>
  );
}

/**
 * 单元格只负责选择：详情统一在页面下侧的详情卡片里展开，行内不再就地撑高，
 * 对照表也就不会因为展开一条把其余行挤走。
 */
function BindingCell({
  binding,
  agent,
  selected,
  onSelect,
}: {
  binding: ProviderBinding;
  agent: string;
  selected: boolean;
  onSelect: () => void;
}) {
  const lines = (
    <>
      <span
        className="provider-account-provider"
        data-tooltip={binding.provider}
      >
        <i aria-hidden="true" />
        <span>{binding.provider}</span>
        {binding.active && <em className="provider-account-active">使用中</em>}
      </span>
      <strong className="provider-account-name" data-tooltip={binding.account}>
        {binding.account}
      </strong>
      {binding.detail && (
        <span className="provider-account-detail" data-tooltip={binding.detail}>
          {binding.detail}
        </span>
      )}
      {binding.quota ? (
        <BindingQuota quota={binding.quota} />
      ) : binding.state === "active" && binding.source === "login" ? (
        <span
          className="provider-account-quota provider-account-quota-empty"
          data-tooltip="订阅登录且 Agent 运行时读取官方额度快照"
        >
          暂无额度快照
        </span>
      ) : null}
    </>
  );
  return (
    <div
      className="provider-account-wrap"
      data-selected={selected || undefined}
    >
      <button
        type="button"
        className="provider-account"
        data-state={binding.state}
        aria-pressed={selected}
        aria-controls="provider-detail"
        aria-label={`${agent}：${binding.provider}，${binding.account}，查看详情`}
        onClick={onSelect}
      >
        {lines}
      </button>
    </div>
  );
}

function registeredLabel(column: AgentProviderColumn): string {
  const registered = registeredCount(column);
  return registered ? `${registered} 项已登记` : "未登记服务商";
}

/** 窄容器下把「列」拆成卡片：每个 Agent 一张卡，卡内按登记顺序排列。 */
function AgentCards({
  columns,
  tools,
  selection,
  onSelect,
}: {
  columns: AgentProviderColumn[];
  tools: ToolSetup[] | undefined;
  selection: string | null;
  onSelect: (column: AgentProviderColumn, binding: ProviderBinding) => void;
}) {
  return (
    <div className="ai-providers-cards">
      {columns.map((column) => {
        const bot = agentBotId(column.key);
        return (
          <article className="ai-provider-card" key={column.key}>
            <header className="ai-provider-card-head">
              {bot && (
                <AgentBot
                  bot={bot}
                  size={26}
                  className="ai-providers-agent-bot"
                />
              )}
              <div className="ai-providers-agent-copy">
                <span className="ai-providers-agent-line">
                  <strong data-tooltip={column.label}>{column.label}</strong>
                  <AgentRunBadge state={runStateOf(tools, column.key)} />
                </span>
                <span data-tooltip={column.hint}>
                  {registeredLabel(column)}
                </span>
              </div>
            </header>
            <ol className="ai-provider-card-list">
              {column.bindings.map((binding, index) => (
                <li key={binding.id} data-state={binding.state}>
                  <span className="ai-provider-card-index" aria-hidden="true">
                    {index + 1}
                  </span>
                  <BindingCell
                    binding={binding}
                    agent={column.label}
                    selected={selection === binding.id}
                    onSelect={() => onSelect(column, binding)}
                  />
                </li>
              ))}
            </ol>
          </article>
        );
      })}
    </div>
  );
}

/**
 * 服务商与 Agent 对照表：列是已添加的 Agent，行是各 Agent 的登记顺序。
 * 只读展示本机配置与登录快照，不读取密钥，也不把查询失败当成未配置。
 *
 * 版式：整页固定为一屏，表体在内部滚动（表头吸顶），下侧预留详情卡片，
 * 窄容器自动改为卡片列表。用户添加的模型源可以在详情里移除。
 */
export default function AiProviders() {
  const config = useAppStore((s) => s.config);
  const cards = aiToolCards(config);
  const setup = useToolSetup();
  const accounts = useProviderAccounts(cards.map((card) => card.key));
  const removed = useRemovedProviders();
  const columns = withoutRemovedProviders(
    buildAgentColumns(
      cards.map((card) => ({
        meta: card,
        item: accounts.items?.find((item) => item.key === card.key),
        entry: setup.snapshot?.tools.find((tool) => tool.key === card.key),
        codexAccounts: accounts.codexAccounts,
        claudeIdentities: accounts.claudeIdentities,
      })),
    ),
    removed,
  );
  const summary = summarizeColumns(columns);
  const rows = columnRows(columns);
  const readAt = accounts.readAt
    ? new Date(accounts.readAt).toLocaleTimeString("zh-CN", { hour12: false })
    : null;
  const warnings = [
    ...(setup.error ? [setup.error] : []),
    ...accounts.warnings,
  ];
  const loading = !accounts.loaded || (!setup.snapshot && !setup.error);

  // 选中项只记身份，详情从当前列重算：刷新后数据变了仍指向同一条登记。
  const [selection, setSelection] = useState<{
    agentKey: string;
    bindingId: string;
  } | null>(null);
  const [removedNotice, setRemovedNotice] =
    useState<RemovedProviderNotice | null>(null);
  const picked = selection;
  const selectedColumn = picked
    ? columns.find((column) => column.key === picked.agentKey)
    : undefined;
  const selectedBinding =
    picked && selectedColumn
      ? selectedColumn.bindings.find(
          (binding) => binding.id === picked.bindingId,
        )
      : undefined;
  const detail: ProviderDetailSelection | null =
    selectedColumn && selectedBinding
      ? {
          agentKey: selectedColumn.key,
          agentLabel: selectedColumn.label,
          binding: selectedBinding,
        }
      : null;

  const select = (column: AgentProviderColumn, binding: ProviderBinding) => {
    setRemovedNotice(null);
    setSelection((current) =>
      current && current.bindingId === binding.id
        ? null
        : { agentKey: column.key, bindingId: binding.id },
    );
  };
  const removeSelected = () => {
    if (!selectedColumn || !selectedBinding) return;
    removeProvider(selectedBinding.id);
    setRemovedNotice({
      key: selectedBinding.id,
      agentKey: selectedColumn.key,
      agentLabel: selectedColumn.label,
      provider: selectedBinding.provider,
      account: selectedBinding.account,
    });
    setSelection(null);
  };
  const closeDetail = () => {
    setRemovedNotice(null);
    setSelection(null);
  };
  const undoRemove = () => {
    const notice = removedNotice;
    if (!notice) return;
    restoreProvider(notice.key);
    setRemovedNotice(null);
    setSelection({ agentKey: notice.agentKey, bindingId: notice.key });
  };

  // Esc 收起详情：焦点若还在单元格上，键盘用户也能一键回到表格。
  useEffect(() => {
    if (!selection && !removedNotice) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      setRemovedNotice(null);
      setSelection(null);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [selection, removedNotice]);

  return (
    <div className="ai-providers-page" data-agents={columns.length}>
      <PageHeader
        icon={Boxes}
        title="服务商与 Agent"
        meta={
          <>
            <span>{summary.agents} 个 Agent</span>
            <i className="proxy-meta-dot" aria-hidden="true" />
            <span>{summary.providers} 个服务商</span>
            <i className="proxy-meta-dot" aria-hidden="true" />
            <span>{summary.accounts} 个账号</span>
          </>
        }
        actions={
          <button
            type="button"
            className="proxy-action ai-providers-refresh"
            disabled={accounts.refreshing || loading}
            onClick={() => void accounts.refresh()}
          >
            {accounts.refreshing ? (
              <Loader2 size={14} className="animate-spin" />
            ) : (
              <RefreshCw size={14} />
            )}
            刷新对照表
          </button>
        }
        tabs={<AiTabs />}
        note={
          <>
            <span>
              按 Agent
              分列核对各自登记的服务商与账号；点击任一服务商在下方查看详情，非官方添加的可以在详情里移除。
            </span>
            <span className="ai-providers-timestamp">
              {readAt ? `读取于 ${readAt} · 每分钟刷新` : "尚未读取"}
            </span>
          </>
        }
      />

      {warnings.length > 0 && (
        <div
          role="alert"
          className="ai-providers-warning"
          aria-label="读取警告"
        >
          <AlertTriangle size={15} />
          <div>
            {warnings.map((warning) => (
              <p key={warning}>{warning}</p>
            ))}
          </div>
        </div>
      )}

      {loading ? (
        <div className="ai-providers-loading" role="status">
          <Loader2 size={16} className="animate-spin" />
          正在读取各 Agent 的服务商与账号…
        </div>
      ) : (
        <section
          className="ai-providers-board"
          aria-label="服务商与 Agent 对照表"
        >
          <div className="ai-providers-scroll">
            <table className="ai-providers-table">
              <caption className="sr-only">
                每个已添加的 Agent 各自登记的服务商与账号
              </caption>
              <thead>
                <tr>
                  <th
                    scope="col"
                    className="ai-providers-corner"
                    data-tooltip="列是 Agent，行是各 Agent 的登记顺序"
                  >
                    序号
                  </th>
                  {columns.map((column) => {
                    const bot = agentBotId(column.key);
                    return (
                      <th scope="col" key={column.key}>
                        <div className="ai-providers-agent">
                          {bot && (
                            <AgentBot
                              bot={bot}
                              size={32}
                              className="ai-providers-agent-bot"
                            />
                          )}
                          <div className="ai-providers-agent-copy">
                            <span className="ai-providers-agent-line">
                              <strong data-tooltip={column.label}>
                                {column.label}
                              </strong>
                              <AgentRunBadge
                                state={runStateOf(
                                  setup.snapshot?.tools,
                                  column.key,
                                )}
                              />
                            </span>
                            <span data-tooltip={column.hint}>
                              {registeredLabel(column)}
                            </span>
                          </div>
                        </div>
                      </th>
                    );
                  })}
                </tr>
              </thead>
              <tbody>
                {Array.from({ length: rows }, (_, index) => (
                  <tr key={index}>
                    <th scope="row">{index + 1}</th>
                    {columns.map((column) => {
                      const binding = column.bindings[index];
                      return (
                        <td key={column.key}>
                          {binding ? (
                            <BindingCell
                              binding={binding}
                              agent={column.label}
                              selected={selection?.bindingId === binding.id}
                              onSelect={() => select(column, binding)}
                            />
                          ) : (
                            <span
                              className="ai-providers-blank"
                              aria-hidden="true"
                            >
                              —
                            </span>
                          )}
                        </td>
                      );
                    })}
                  </tr>
                ))}
              </tbody>
            </table>
            <AgentCards
              columns={columns}
              tools={setup.snapshot?.tools}
              selection={selection?.bindingId ?? null}
              onSelect={select}
            />
          </div>
          <footer className="ai-providers-foot">
            <p>
              服务商来自各工具的用户级配置与模型源，账号来自本机登录与加密快照；不读取密钥，
              查询失败也不会显示成零余额。额度只在对应 Agent
              运行时按固定间隔同步，关闭后保留最后一次快照。移除只影响本页与各
              Agent 详情，工具内的配置保持不变。
            </p>
            <ul className="ai-providers-legend" aria-label="状态说明">
              {LEGEND.map(([state, label]) => (
                <li key={state} data-state={state}>
                  <i aria-hidden="true" />
                  {label}
                </li>
              ))}
            </ul>
            <div className="ai-providers-links">
              {removed.size > 0 && (
                <button
                  type="button"
                  className="ai-providers-restore"
                  onClick={() => {
                    restoreAllProviders();
                    closeDetail();
                  }}
                >
                  <Undo2 size={11} />
                  已隐藏 {removed.size} 项 · 全部恢复
                </button>
              )}
              <Link to="/ai">
                启动工具
                <ArrowRight size={12} />
              </Link>
              <Link to="/ai-stats">
                用量与统计
                <ArrowRight size={12} />
              </Link>
            </div>
          </footer>
        </section>
      )}

      {/* 详情是独立卡片：和上面的对照表分开，始终占住页面下沿。 */}
      <ProviderDetailCard
        selection={detail}
        removed={removedNotice}
        readAt={readAt}
        onRemove={removeSelected}
        onUndo={undoRemove}
        onClose={closeDetail}
      />
    </div>
  );
}
