/**
 * 路由分包加载占位。
 *
 * 用与页面同构的骨架代替“加载中”文字：分包命中缓存时（绝大多数切换）骨架根本不会出现，
 * 慢一点的首次进入也只是看到版式而不是一行字。样式里做了 150ms 延迟，避免闪一下。
 */
export default function RouteFallback() {
  return (
    <div
      className="route-skeleton"
      role="status"
      aria-busy="true"
      aria-live="polite"
    >
      <span className="skeleton-label">正在打开页面</span>
      <span className="skeleton-bar skeleton-heading" />
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
  );
}
