import { useProxyStore } from "../state";
import type { ProxyLifecycle } from "../services/api";
const stateLabel: Record<ProxyLifecycle, string> = {
  stopped: "已停止",
  starting: "启动中",
  ready: "就绪",
  reconfiguring: "应用配置中",
  stopping: "停止中",
  failed: "运行失败",
};
/**
 * 所有界面读取同一份内核、监听端口和系统代理观测。
 * 页头副标题与订阅页同一套写法：`·` 分隔，末段状态词按生命周期着色。
 */
export default function ProxyRuntimeStatus() {
  const core = useProxyStore((s) => s.coreStatus);
  const error = useProxyStore((s) => s.coreError);
  const lifecycle: ProxyLifecycle =
    core?.state ?? (core?.running ? "ready" : "stopped");
  const failed = Boolean(error || core?.last_error) || lifecycle === "failed";
  return (
    <span
      aria-label="代理运行状态"
      className="overview-runtime-meta"
      data-state={failed ? "error" : core ? lifecycle : "unknown"}
      data-tooltip={error || core?.last_error || undefined}
    >
      {core ? (
        <>
          <span>云阔 mihomo</span>
          <i className="proxy-meta-dot" aria-hidden="true" />
          <span>{core.service_managed ? "TUN 服务" : "应用托管"}</span>
          <i className="proxy-meta-dot" aria-hidden="true" />
          <strong>{stateLabel[lifecycle]}</strong>
        </>
      ) : error ? (
        "运行状态未知"
      ) : (
        "读取运行状态…"
      )}
    </span>
  );
}
