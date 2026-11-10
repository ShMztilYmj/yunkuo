import { useState } from "react";
import { api, type ServiceStatusInfo } from "../../../services/api";
import { usePolling } from "../../../shared/hooks/usePolling";

/** TUN 系统服务的安装 / 运行状态徽标。 */
export default function ServiceStateBadge() {
  const [status, setStatus] = useState<ServiceStatusInfo | null>(null);
  const [failed, setFailed] = useState(false);
  usePolling(async (isCurrent) => {
    try {
      const next = await api.serviceStatus();
      if (isCurrent()) {
        setStatus(next);
        setFailed(false);
      }
    } catch {
      if (isCurrent()) {
        setStatus(null);
        setFailed(true);
      }
    }
  }, 5000);

  if (!status) {
    return (
      <span className="settings-service" data-tone={failed ? "warn" : "idle"}>
        <i aria-hidden="true" />
        <span>
          {failed ? "服务状态读取失败，稍后自动重试" : "正在读取服务状态…"}
        </span>
      </span>
    );
  }
  if (status.running && status.compatible) {
    return (
      <span className="settings-service" data-tone="ok">
        <i aria-hidden="true" />
        <span>
          服务运行中 · 托管内核：
          {status.coreRunning ? `PID ${status.pid}` : "未启动"}
        </span>
      </span>
    );
  }
  if (status.installed || status.running) {
    return (
      <span className="settings-service" data-tone="warn">
        <i aria-hidden="true" />
        <span>{status.error || "已安装，但服务未在线"}</span>
      </span>
    );
  }
  return (
    <span className="settings-service">
      <i aria-hidden="true" />
      <span>未安装 · 首次开启 TUN 时会自动安装</span>
    </span>
  );
}
