import { Suspense, useLayoutEffect, useRef } from "react";
import { Outlet, useLocation, useNavigate } from "react-router-dom";
import ErrorBoundary from "../../shared/ui/ErrorBoundary";
import RouteFallback from "../../shared/ui/RouteFallback";

/** 右侧工作区：路由内容（带错误边界与加载占位）；提示统一收进标题栏的消息中心 */
export default function WorkspaceContent() {
  const loc = useLocation();
  const nav = useNavigate();
  const content = useRef<HTMLElement>(null);
  useLayoutEffect(() => {
    if (content.current) content.current.scrollTop = 0;
  }, [loc.pathname]);

  return (
    <section className="workspace" aria-label="工作区">
      <main ref={content} className="workspace-content">
        <ErrorBoundary key={loc.pathname + loc.search} onHome={() => nav("/")}>
          <Suspense fallback={<RouteFallback />}>
            <div className="page-transition" key={loc.pathname + loc.search}>
              <Outlet />
            </div>
          </Suspense>
        </ErrorBoundary>
      </main>
    </section>
  );
}
