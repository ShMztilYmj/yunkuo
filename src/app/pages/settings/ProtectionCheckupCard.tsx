import { CheckCircle2, ShieldAlert, ShieldCheck, XCircle } from "lucide-react";
import { Link } from "react-router-dom";
import { useProxyStore } from "../../../features/proxy/state";

const clock = (at: number) => {
  const value = new Date(at);
  const now = new Date();
  return value.toDateString() === now.toDateString()
    ? value.toLocaleTimeString("zh-CN", { hour12: false })
    : value.toLocaleString("zh-CN", { hour12: false });
};

/**
 * 严格保护体检卡：逐项展示服务端 live_ok 的复核结果。
 * 数据随 core_status 每 3 秒到达；保护运行期间服务端每秒复核，这里只负责呈现。
 */
export default function ProtectionCheckupCard() {
  const privacy = useProxyStore((state) => state.privacyStatus);
  const core = useProxyStore((state) => state.coreStatus);
  const config = useProxyStore((state) => state.config);
  const armed = privacy?.armed === true;
  const healthy =
    armed && privacy?.healthy === true && privacy?.tunnelOn === true;
  const checks = privacy?.checks ?? [];
  const failed = checks.filter((check) => !check.ok).length;
  const allPassed = checks.length > 0 && failed === 0;
  // 旧版安全服务只回报健康布尔值；此时保护仍在运行，只是没有分项结果。
  const legacy = healthy && checks.length === 0;
  // 未启用严格方案时服务不会上报 privacy，这是正常的“未启用”，不是状态丢失。
  const pending = privacy == null && (core == null || config == null);
  const unavailable =
    privacy == null && !pending && config?.network_mode === "strict";
  const idle = !armed && !pending && !unavailable;
  const tone =
    pending || unavailable || idle
      ? "idle"
      : healthy && (allPassed || legacy)
        ? "ok"
        : "alert";
  const verdict = pending
    ? "读取中"
    : unavailable
      ? "状态未知"
      : idle
        ? "未启用"
        : healthy && allPassed
          ? "全部通过"
          : healthy
            ? checks.length === 0
              ? "运行中"
              : `${failed} 项异常`
            : "已阻断";

  return (
    <section
      id="protection-checkup"
      className="protection-checkup ui-panel"
      aria-label="保护体检"
    >
      <header className="protection-checkup-head">
        <span className="protection-checkup-badge" data-tone={tone}>
          {tone === "alert" ? (
            <ShieldAlert size={16} aria-hidden="true" />
          ) : (
            <ShieldCheck size={16} aria-hidden="true" />
          )}
        </span>
        <div className="protection-checkup-heading">
          <h3>保护体检</h3>
          <p>严格保护的实时校验：内核身份、TUN 与路由、出口节点、系统代理。</p>
        </div>
        <span className="protection-checkup-verdict" data-tone={tone}>
          <i aria-hidden="true" />
          {verdict}
        </span>
      </header>

      {pending ? (
        <p className="protection-checkup-empty">正在读取严格保护状态…</p>
      ) : unavailable ? (
        <p className="protection-checkup-empty">
          保护状态未知：安全服务未就绪或读取失败，请先启动 TUN 服务并刷新状态。
        </p>
      ) : idle ? (
        <div className="protection-checkup-empty">
          <span>
            严格保护未启用。启用后会自动逐项核对内核身份、TUN
            与路由、出口节点和系统代理。
          </span>
          <Link className="protection-checkup-action" to="/overview">
            去启用严格保护
          </Link>
        </div>
      ) : checks.length === 0 ? (
        <p className="protection-checkup-empty">
          {legacy
            ? "当前安全服务未提供分项体检结果；重新安装或修复 TUN 服务后即可逐项显示。"
            : "严格保护正在阻断，但尚未生成复核结果；重新启用保护后会立即体检。"}
        </p>
      ) : (
        <ul className="protection-checkup-list">
          {checks.map((check) => (
            <li
              key={check.key}
              className="protection-checkup-item"
              data-ok={check.ok}
              aria-label={`${check.title}：${check.ok ? "通过" : "异常"}，${check.detail}`}
            >
              <span className="protection-checkup-mark" aria-hidden="true">
                {check.ok ? <CheckCircle2 size={15} /> : <XCircle size={15} />}
              </span>
              <span className="protection-checkup-copy">
                <strong>{check.title}</strong>
                <small>{check.detail}</small>
              </span>
              <span className="protection-checkup-state">
                {check.ok ? "通过" : "异常"}
              </span>
            </li>
          ))}
        </ul>
      )}

      {privacy?.error && armed && (checks.length === 0 || allPassed) && (
        <p className="protection-checkup-error" role="alert">
          <ShieldAlert size={14} aria-hidden="true" />
          <span>{privacy.error}</span>
        </p>
      )}

      <footer className="protection-checkup-foot">
        <span>
          {privacy?.checkedAt
            ? `最近复核 ${clock(privacy.checkedAt)}`
            : "尚未复核"}
        </span>
        <span>
          {healthy
            ? checks.length > 0
              ? "保护运行期间每秒复核"
              : "修复服务后自动复核"
            : armed
              ? "重新启用后自动复核"
              : "启用后自动复核"}
        </span>
      </footer>
    </section>
  );
}
