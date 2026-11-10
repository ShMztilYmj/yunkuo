import { RefreshCw, TriangleAlert } from "lucide-react";

/** 配置尚未就绪时的占位：读取中显示设置页骨架，失败可重试。 */
export default function ConfigLoadingState({
  configLoading,
  configError,
  onReload,
}: {
  configLoading: boolean;
  configError: string | null;
  onReload: () => void;
}) {
  if (configError) {
    return (
      <div className="settings-error" aria-busy={configLoading}>
        <span className="settings-error-icon" aria-hidden="true">
          <TriangleAlert />
        </span>
        <p role="alert">读取设置失败：{configError}</p>
        <button
          type="button"
          className="settings-action"
          disabled={configLoading}
          onClick={onReload}
        >
          <RefreshCw className={configLoading ? "animate-spin" : ""} />
          重新加载设置
        </button>
      </div>
    );
  }
  return (
    <div
      className="settings-skeleton"
      role="status"
      aria-busy="true"
      aria-live="polite"
    >
      <span className="skeleton-label">正在读取设置</span>
      <span className="skeleton-bar skeleton-heading" />
      <div className="settings-skeleton-body" aria-hidden="true">
        <div className="settings-skeleton-rail">
          {Array.from({ length: 6 }, (_, index) => (
            <span key={index} />
          ))}
        </div>
        <div className="settings-skeleton-cards">
          <div className="skeleton-card">
            <span />
            <span />
            <span />
          </div>
          <div className="skeleton-card">
            <span />
            <span />
          </div>
        </div>
      </div>
    </div>
  );
}
