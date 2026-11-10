import { Component, type ErrorInfo, type ReactNode } from "react";
import { recordFault } from "../../services/faults";

interface Props {
  children: ReactNode;
  onHome?: () => void;
}

/** 页面错误不会卸载导航与全局状态；重载只能由用户明确触发。 */
export default class ErrorBoundary extends Component<
  Props,
  { failed: boolean }
> {
  state = { failed: false };
  static getDerivedStateFromError() {
    return { failed: true };
  }
  componentDidCatch(_error: Error, _info: ErrorInfo) {
    recordFault("render");
    // 启动画面在 #root 之外，渲染失败时要手动收起，否则错误页会被盖住。
    document.getElementById("boot")?.remove();
  }
  render() {
    if (!this.state.failed) return this.props.children;
    return (
      <div
        role="alert"
        className="m-6 space-y-4 rounded-xl border border-amber-800 bg-zinc-900 p-6 text-sm text-zinc-200"
      >
        <h1 className="font-semibold">页面暂时无法显示</h1>
        <p className="text-xs leading-6 text-zinc-400">
          页面加载或显示发生异常。可以返回首页，或重新加载界面后再试。重新加载只刷新界面；代理和保护状态需要重新读取。
        </p>
        <div className="flex gap-3">
          {this.props.onHome && (
            <button
              onClick={this.props.onHome}
              className="rounded-lg bg-zinc-700 px-4 py-2"
            >
              返回首页
            </button>
          )}
          <button
            onClick={() => window.location.reload()}
            className="rounded-lg btn-primary px-4 py-2"
          >
            重新加载界面
          </button>
        </div>
      </div>
    );
  }
}
