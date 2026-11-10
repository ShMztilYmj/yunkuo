import { Link } from "react-router-dom";
import { Loader2, RefreshCw, Network } from "lucide-react";
import {
  connectionModel,
  connectionSource,
  type ToolConnection,
  type ToolConnectivity,
  type ToolInspection,
  type ToolSetup,
} from "../services/toolSetup";

export function ToolConnectionSummary({
  connection,
}: {
  connection: ToolConnection;
}) {
  return (
    <section className="tool-connection-summary" aria-label="当前工具配置">
      <h3>用户级配置</h3>
      <dl>
        <div>
          <dt>模型</dt>
          <dd>{connectionModel(connection)}</dd>
        </div>
        <div>
          <dt>连接</dt>
          <dd>
            {connectionSource(connection)}
            {connection.endpoint && <small>{connection.endpoint}</small>}
          </dd>
        </div>
      </dl>
      <p>{connection.note}</p>
    </section>
  );
}
export default function ToolSetupPanel({
  entry,
  inspection,
  connectionTest,
  error,
  busy,
  onInspect,
  onTest,
  onUse,
}: {
  entry?: ToolSetup;
  inspection?: ToolInspection;
  connectionTest?: ToolConnectivity;
  error?: string;
  busy?: string;
  onInspect: () => void;
  onTest: () => void;
  onUse: (path: string) => void;
}) {
  const candidates = inspection?.candidates ?? entry?.candidates ?? [];
  return (
    <section className="tool-setup-panel" aria-label="安装与连接检查">
      {error && (
        <p role="alert" data-failed="true">
          {error}
        </p>
      )}
      {entry && (
        <>
          <div className="tool-setup-facts">
            <span>
              安装：
              {entry.installed == null
                ? "未知"
                : entry.installed
                  ? "已发现"
                  : "未发现"}
            </span>
            <span>
              进程：
              {entry.running == null
                ? "未知"
                : entry.running
                  ? "运行中"
                  : "未运行"}
            </span>
            <span>
              入口：
              {!entry.configured
                ? "未配置"
                : entry.shortcutValid
                  ? "文件有效"
                  : "路径已失效"}
            </span>
          </div>
          <ToolConnectionSummary connection={entry.connection} />
        </>
      )}
      <div className="tool-setup-actions">
        <button disabled={!!busy} onClick={onInspect}>
          {busy === "inspect" ? (
            <Loader2 size={12} className="animate-spin" />
          ) : (
            <RefreshCw size={12} />
          )}
          检测安装与版本
        </button>
        <button
          disabled={
            !!busy || !entry || !["claude", "chatgpt"].includes(entry.key)
          }
          onClick={onTest}
        >
          {busy === "test" ? (
            <Loader2 size={12} className="animate-spin" />
          ) : (
            <Network size={12} />
          )}
          测试连接
        </button>
      </div>
      {inspection && (
        <p role="status">
          {inspection.version && `文件版本 ${inspection.version} · `}
          {inspection.note}
        </p>
      )}
      {candidates.length > 0 && (
        <div className="tool-setup-candidates">
          <p>发现的启动入口（选中后保存）</p>
          {candidates.map((path) => (
            <button key={path} disabled={!!busy} onClick={() => onUse(path)}>
              <span>{path}</span>
              <b>使用</b>
            </button>
          ))}
        </div>
      )}
      {inspection && candidates.length === 0 && (
        <p>未找到可直接使用的入口，可继续手动选择快捷方式。</p>
      )}
      {connectionTest && (
        <p role="status" data-failed={!connectionTest.reachable}>
          上次测试：{connectionTest.reachable ? "地址可达" : "连接未通过"} ·{" "}
          {connectionTest.endpoint}
          {connectionTest.status != null && ` · HTTP ${connectionTest.status}`}
          <br />
          {connectionTest.note}
        </p>
      )}
      <div className="tool-setup-links">
        <Link to="/detect">环境检测与修复</Link>
      </div>
    </section>
  );
}
