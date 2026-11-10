import { useState } from "react";
import { Activity, Trash2, Loader2 } from "lucide-react";
import { proxyApi, type ProxyConnections } from "../services/api";
import { usePolling } from "../../../shared/hooks/usePolling";
import { formatTrafficBytes } from "../services/subscriptionTraffic";
import ProxyTabs from "../components/ProxyTabs";
import PageHeader from "../../../shared/ui/PageHeader";

/** 连接表沿用两位小数（GB）；其它档位与订阅卡共用同一实现。 */
const fmtBytes = (n: number) => formatTrafficBytes(n, 2);

export default function Connections() {
  const [data, setData] = useState<ProxyConnections | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [updating, setUpdating] = useState(false);
  const [page, setPage] = useState(0);
  const pageSize = 100;

  usePolling(async (isCurrent) => {
    setUpdating(true);
    try {
      const d = await proxyApi.connections();
      if (!isCurrent()) return;
      setData(d);
      setError(null);
    } catch (e) {
      if (isCurrent()) setError(String(e));
    } finally {
      if (isCurrent()) setUpdating(false);
    }
  }, 1500);

  const closeConnections = async (id: string | null = null) => {
    try {
      await proxyApi.closeConnections(id);
      setData(await proxyApi.connections());
      setError(null);
    } catch (reason) {
      setError(String(reason));
    }
  };

  const conns = data?.items ?? [];
  const pages = Math.max(1, Math.ceil(conns.length / pageSize));
  const currentPage = Math.min(page, pages - 1);
  const visible = conns.slice(
    currentPage * pageSize,
    (currentPage + 1) * pageSize,
  );

  return (
    <div className="proxy-data-page connections-page">
      <PageHeader
        icon={Activity}
        title="连接"
        badge={`${conns.length} 条活跃`}
        meta={
          <>
            <span className="proxy-live-dot" aria-hidden="true" />
            <span>实时刷新</span>
            <i className="proxy-meta-dot" />
            <span>
              总上传 <strong>{fmtBytes(data?.uploaded ?? 0)}</strong>
            </span>
            <i className="proxy-meta-dot" />
            <span>
              总下载 <strong>{fmtBytes(data?.downloaded ?? 0)}</strong>
            </span>
            <i className="proxy-meta-dot" />
            <span>每 1.5 秒自动刷新</span>
          </>
        }
        actions={
          <>
            {updating && (
              <Loader2 className="proxy-spin h-4 w-4 text-zinc-500" />
            )}
            <button
              type="button"
              onClick={() => void closeConnections()}
              disabled={!conns.length || updating}
              className="proxy-action is-danger"
            >
              <Trash2 /> 关闭全部连接
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

        <section className="proxy-panel proxy-table-card">
          {conns.length === 0 ? (
            <div className="proxy-empty">
              <span className="proxy-empty-icon">
                <Activity size={20} />
              </span>
              <strong>暂无活跃连接</strong>
              <p>
                代理内核运行后，经过内核的连接会实时显示在这里；可以直接关闭单条或全部连接。
              </p>
            </div>
          ) : (
            <div className="proxy-table-scroll">
              <table className="proxy-table">
                <thead>
                  <tr>
                    <th className="proxy-col-host">目标</th>
                    <th className="proxy-col-rule">命中规则</th>
                    <th className="proxy-col-chain">规则链</th>
                    <th className="proxy-col-proc">进程</th>
                    <th className="proxy-col-net">网络</th>
                    <th className="proxy-col-up proxy-numeric">上传</th>
                    <th className="proxy-col-down proxy-numeric">下载</th>
                    <th aria-label="操作"></th>
                  </tr>
                </thead>
                <tbody>
                  {visible.map((c) => {
                    const host =
                      c.host ||
                      (c.destinationIp && c.destinationPort
                        ? `${c.destinationIp}:${c.destinationPort}`
                        : "—");
                    const proc = c.processPath
                      ? c.processPath.split(/[/\\]/).pop() || "—"
                      : (c.process ?? "—");
                    const chain = c.chain?.length ? c.chain.join(" ← ") : "—";
                    const rule = c.rule
                      ? c.rulePattern
                        ? `${c.rule} · ${c.rulePattern}`
                        : c.rule
                      : "—";
                    return (
                      <tr key={c.id}>
                        <td className="proxy-col-host">
                          <span className="proxy-cell-strong" title={host}>
                            {host}
                          </span>
                        </td>
                        <td className="proxy-col-rule">
                          <span className="proxy-rule" title={rule}>
                            {rule}
                          </span>
                        </td>
                        <td className="proxy-col-chain">
                          <span className="proxy-chain" title={chain}>
                            {chain}
                          </span>
                        </td>
                        <td className="proxy-col-proc">
                          <span className="proxy-mono proxy-muted">{proc}</span>
                        </td>
                        <td className="proxy-col-net">
                          <span className="proxy-badge">
                            {c.network}/{c.inbound}
                          </span>
                        </td>
                        <td className="proxy-col-up proxy-numeric is-up">
                          {fmtBytes(c.uploaded)}
                        </td>
                        <td className="proxy-col-down proxy-numeric is-down">
                          {fmtBytes(c.downloaded)}
                        </td>
                        <td>
                          <button
                            type="button"
                            onClick={() => void closeConnections(c.id)}
                            className="proxy-row-action"
                            data-tooltip="关闭连接"
                            aria-label="关闭连接"
                          >
                            <Trash2 size={14} />
                          </button>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          )}
          {conns.length > 0 && (
            <div className="proxy-table-foot">
              <span>数据来自内核控制器，关闭连接后会立即从列表移除</span>
              <span>{conns.length} 条</span>
            </div>
          )}
        </section>

        {pages > 1 && (
          <div className="proxy-pagination">
            <span>
              每页 {pageSize} 条 · 第 {currentPage + 1} / {pages} 页
            </span>
            <div className="proxy-pagination-actions">
              <button
                type="button"
                disabled={currentPage === 0}
                onClick={() => setPage(currentPage - 1)}
              >
                上一页
              </button>
              <button
                type="button"
                disabled={currentPage >= pages - 1}
                onClick={() => setPage(currentPage + 1)}
              >
                下一页
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
