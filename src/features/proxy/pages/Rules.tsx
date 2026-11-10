import { useCallback, useMemo, useRef, useState } from "react";
import {
  Compass,
  Info,
  ListFilter,
  Loader2,
  RefreshCw,
  Search,
  X,
} from "lucide-react";
import { api, type IpInfo } from "../../../services/api";
import {
  proxyApi,
  type ProxyExplanation,
  type ProxyRule,
} from "../services/api";
import ProxyTabs from "../components/ProxyTabs";
import PageHeader from "../../../shared/ui/PageHeader";
import { missingRulesNotice, summarizeRules } from "../rule-summary";
import { usePolling } from "../../../shared/hooks/usePolling";

/** 规则类型只做视觉分组，不改变表格内容。 */
function kindTone(kind: string) {
  const value = kind.toLowerCase();
  if (value.includes("domain")) return "domain";
  if (value.includes("geo")) return "geo";
  if (value.includes("process")) return "process";
  if (value.includes("rule") || value.includes("set")) return "rule";
  return undefined;
}

function targetTone(target: string) {
  if (target === "DIRECT") return "direct";
  if (target === "REJECT" || target === "REJECT-DROP") return "reject";
  return "proxy";
}

export default function Rules() {
  const [rules, setRules] = useState<ProxyRule[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const [readAt, setReadAt] = useState<number | null>(null);
  const [host, setHost] = useState("");
  const [explaining, setExplaining] = useState(false);
  const [explainError, setExplainError] = useState<string | null>(null);
  const [explanation, setExplanation] = useState<ProxyExplanation | null>(null);
  const [exit, setExit] = useState<IpInfo | null>(null);

  const busyRef = useRef(false);
  const load = useCallback(async (isCurrent: () => boolean = () => true) => {
    if (busyRef.current) return;
    busyRef.current = true;
    try {
      const d = await proxyApi.rules();
      if (!isCurrent()) return;
      setRules(d);
      setError(null);
      setReadAt(Date.now());
    } catch (e) {
      if (isCurrent()) setError(String(e));
    } finally {
      busyRef.current = false;
    }
  }, []);

  usePolling(load, 10000);

  /** 自动刷新保持静默，只有手动刷新才让按钮转圈。 */
  const refresh = async () => {
    setBusy(true);
    try {
      await load();
    } finally {
      setBusy(false);
    }
  };

  /** 解释与出口 IP 并行取：两者都走内核，一起等能省一次往返。 */
  const explain = async () => {
    const value = host.trim();
    if (!value || explaining) return;
    setExplaining(true);
    setExplainError(null);
    try {
      const [result, info] = await Promise.all([
        proxyApi.explain(value),
        api.getIpInfo().catch(() => null),
      ]);
      setExplanation(result);
      setExit(info);
    } catch (e) {
      setExplanation(null);
      setExplainError(String(e));
    } finally {
      setExplaining(false);
    }
  };

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return rules;
    return rules.filter(
      (r) =>
        r.pattern.toLowerCase().includes(q) ||
        r.target.toLowerCase().includes(q) ||
        r.kind.toLowerCase().includes(q),
    );
  }, [rules, query]);

  const overview = useMemo(() => summarizeRules(rules), [rules]);
  const notice = useMemo(() => missingRulesNotice(overview), [overview]);
  /** 首次成功读取后才能断言“没有规则”，避免加载瞬间闪一条提示。 */
  const noRoutingRules = overview.missing && readAt !== null && !error;

  const metaHint = noRoutingRules
    ? overview.onlyFallback
      ? "仅兜底 Match · 规则模式 ≈ 全局模式"
      : "订阅没有 rules 段"
    : "订阅规则按顺序匹配";

  const hasQuery = query.trim().length > 0;
  return (
    <div className="proxy-data-page rules-page">
      <PageHeader
        icon={ListFilter}
        title="规则"
        badge={
          overview.onlyFallback
            ? `${overview.total} 条 · 仅兜底`
            : `${overview.total} 条`
        }
        meta={
          <>
            <span>
              {busy
                ? "读取中…"
                : readAt
                  ? `最后读取 ${new Date(readAt).toLocaleTimeString()}`
                  : "尚未读取"}
            </span>
            <i className="proxy-meta-dot" />
            <span>{metaHint}</span>
            {hasQuery && (
              <>
                <i className="proxy-meta-dot" />
                <span>
                  匹配 <strong>{filtered.length}</strong> 条
                </span>
              </>
            )}
          </>
        }
        actions={
          <>
            <div className="proxy-search">
              <Search aria-hidden="true" />
              <input
                aria-label="搜索规则"
                placeholder="搜索类型、域名或策略…"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
              />
              {hasQuery && (
                <button
                  type="button"
                  className="proxy-search-clear"
                  aria-label="清空搜索"
                  onClick={() => setQuery("")}
                >
                  <X size={13} />
                </button>
              )}
            </div>
            <button
              type="button"
              onClick={() => void refresh()}
              disabled={busy}
              className="proxy-action"
            >
              <RefreshCw className={busy ? "proxy-spin" : ""} />
              刷新
            </button>
          </>
        }
        tabs={<ProxyTabs />}
      />

      <div className="proxy-data-content">
        {error && (
          <div className="proxy-error" role="alert">
            {error}
          </div>
        )}

        {noRoutingRules && overview.onlyFallback && notice && (
          <div className="proxy-footnote" role="note">
            <Info size={13} />
            <span>
              <strong>{notice.title}</strong>：{notice.detail}
            </span>
          </div>
        )}

        <section className="proxy-panel rule-explain" aria-label="规则命中解释">
          <div className="rule-explain-head">
            <span className="rule-explain-icon" aria-hidden="true">
              <Compass size={15} />
            </span>
            <div className="rule-explain-copy">
              <h2>为什么没命中</h2>
              <p>
                输入域名，按当前规则表与代理组选择推导命中路径；需要内核解析的条件会单独标出。
              </p>
            </div>
            <form
              className="rule-explain-form"
              onSubmit={(event) => {
                event.preventDefault();
                void explain();
              }}
            >
              <input
                aria-label="要解释的域名或 IP"
                placeholder="claude.ai 或 203.0.113.7"
                value={host}
                onChange={(event) => setHost(event.target.value)}
                spellCheck={false}
                autoComplete="off"
              />
              <button
                type="submit"
                className="proxy-action proxy-primary-small"
                disabled={explaining || host.trim().length === 0}
              >
                {explaining ? (
                  <Loader2 className="proxy-spin" size={13} />
                ) : (
                  <Compass size={13} />
                )}
                解析
              </button>
            </form>
          </div>

          {explainError && (
            <p className="rule-explain-error" role="alert">
              {explainError}
            </p>
          )}

          {explanation && <ExplanationView result={explanation} exit={exit} />}
        </section>

        <section className="proxy-panel proxy-table-card">
          {filtered.length === 0 ? (
            <div className="proxy-empty">
              <span className="proxy-empty-icon">
                {hasQuery ? <Search size={20} /> : <ListFilter size={20} />}
              </span>
              <strong>
                {rules.length === 0
                  ? error
                    ? "无法读取规则"
                    : (notice?.title ?? "当前订阅没有规则")
                  : "没有匹配的规则"}
              </strong>
              <p>
                {rules.length === 0
                  ? error
                    ? "内核未运行或控制器不可达；启动内核后会从这里读取。"
                    : (notice?.detail ?? "配置里没有 rules 段。")
                  : "换个关键词，或清空搜索查看全部规则。"}
              </p>
              {hasQuery && (
                <button
                  type="button"
                  className="proxy-action"
                  onClick={() => setQuery("")}
                >
                  清空搜索
                </button>
              )}
            </div>
          ) : (
            <div className="proxy-table-scroll">
              <table className="proxy-table">
                <thead>
                  <tr>
                    <th className="proxy-col-kind">类型</th>
                    <th>内容</th>
                    <th className="proxy-col-target">策略</th>
                    <th className="proxy-col-rule-size proxy-numeric">
                      命中规模
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {filtered.map((r, i) => (
                    <tr key={`${r.kind}-${r.pattern}-${i}`}>
                      <td className="proxy-col-kind">
                        <span
                          className="proxy-kind"
                          data-kind={kindTone(r.kind)}
                        >
                          {r.kind}
                        </span>
                      </td>
                      <td>
                        <span
                          className="proxy-cell-strong proxy-mono"
                          title={r.pattern || undefined}
                        >
                          {r.pattern || "—"}
                        </span>
                      </td>
                      <td className="proxy-col-target">
                        <span
                          className="proxy-policy"
                          data-tone={targetTone(r.target)}
                          title={r.target}
                        >
                          {r.target}
                        </span>
                      </td>
                      <td
                        className={`proxy-col-rule-size proxy-numeric${r.size > 0 ? "" : " is-zero"}`}
                      >
                        {r.size}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </section>
      </div>
    </div>
  );
}

function ExplanationView({
  result,
  exit,
}: {
  result: ProxyExplanation;
  exit: IpInfo | null;
}) {
  const builtin = result.finalType
    ? ["direct", "reject", "reject-drop"].includes(
        result.finalType.toLowerCase(),
      )
    : false;
  const exitText = exit?.exitIp
    ? `${exit.exitIp}${exit.exitCountry ? ` · ${exit.exitCountry}` : ""}`
    : "未取得";
  return (
    <div className="rule-explain-result" role="status">
      <div className="rule-explain-grid">
        <div>
          <span>命中规则</span>
          <strong>
            {result.matched
              ? `#${result.matched.index} ${result.matched.kind}`
              : "没有可命中的规则"}
          </strong>
        </div>
        <div>
          <span>匹配内容</span>
          <code>{result.matched?.payload || "—"}</code>
        </div>
        <div>
          <span>去向策略</span>
          <strong>{result.matched?.target || "—"}</strong>
        </div>
        <div>
          <span>选择链</span>
          <span className="rule-explain-chain">
            {result.chain.length > 0 ? result.chain.join(" → ") : "—"}
          </span>
        </div>
        <div>
          <span>最终出口</span>
          <strong>
            {result.finalNode ??
              (builtin ? "内置策略，无节点" : "分组尚未选择节点")}
            {result.finalType ? ` · ${result.finalType}` : ""}
          </strong>
        </div>
        <div>
          <span>当前出口 IP（实时）</span>
          <strong>{exitText}</strong>
        </div>
      </div>

      {result.unrouted && (
        <p className="rule-explain-warn">
          规则表里没有兜底 Match，未命中的流量由内核按 DIRECT
          处理；这本身就是一条「为什么直连」的原因。
        </p>
      )}
      {result.fallback && (
        <p className="rule-explain-note">
          命中的是兜底
          Match：更具体的域名或网段条件都没有命中，结果等同于全局走这个策略。
        </p>
      )}
      {!result.finalNode && !builtin && result.chain.length > 0 && (
        <p className="rule-explain-note">
          链路停在分组上：该组当前没有选中节点，内核会在真正连接时再判定。
        </p>
      )}
      {result.skippedCount > 0 && (
        <details className="rule-explain-skipped">
          <summary>
            有 {result.skippedCount} 条规则需要内核解析，未参与本次判定
          </summary>
          <ul>
            {result.skipped.map((item) => (
              <li key={`${item.index}-${item.kind}`}>
                <span>#{item.index}</span>
                <strong>{item.kind}</strong>
                <em>{item.reason}</em>
              </li>
            ))}
          </ul>
          <p>
            跳过不等于没命中：GEOSITE、GEOIP、RULE-SET
            与进程规则需要内核现场数据，这里只回答能确定的部分。
          </p>
        </details>
      )}
    </div>
  );
}
