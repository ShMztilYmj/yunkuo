import { useCallback, useState, type ReactNode } from "react";
import { ArrowRight } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useNavigate } from "react-router-dom";
import {
  browserEnvironment,
  type BrowserEnvironment,
} from "../../../features/fingerprint-browser/services/browser";
import { usePolling } from "../../../shared/hooks/usePolling";

/* ---------- 新手引导的公共零件 ----------
   每一步只回答两件事：怎么做、做到没有。版式固定为三块——
   「怎么做」的编号动作、「做到就算过」的判定灯、「别踩」的一行警告。
   正文里不写原理讲解，也不放示意图：新手按着点就行，要细节的去对应功能页。
   类名统一 g- 前缀，样式在 styles/maintenance-guide.css。 */

/** 读当前指纹资料（和指纹浏览器页同一份数据）；读不到时返回 null。 */
export function useBrowserEnvironment(interval = 15000) {
  const [env, setEnv] = useState<BrowserEnvironment | null>(null);

  const load = useCallback(async (isCurrent: () => boolean = () => true) => {
    try {
      const next = await browserEnvironment.browser();
      if (isCurrent()) setEnv(next);
    } catch {
      // 读不到就保持上一份状态：判定灯显示中性，不影响页面说明
    }
  }, []);
  usePolling(load, interval);

  return env;
}

/** 判定灯：ok = 已经到位，todo = 还差这一步，idle = 状态未知（没读到数据）。 */
export type GuideTone = "ok" | "todo" | "idle";

export interface GuideStepItem {
  title: string;
  detail?: string;
  /** 这一步对应的入口：`to` 走应用内路由，`href` 用系统浏览器打开 */
  action?: { label: string; to: string } | { label: string; href: string };
}

export interface GuideCheckItem {
  label: string;
  detail?: string;
  tone?: GuideTone;
}

/** 一块小节：标题 + 一句提示 + 内容。 */
export function GuideSection({
  title,
  hint,
  tone,
  children,
}: {
  title: string;
  hint?: string;
  tone?: "warn";
  children: ReactNode;
}) {
  return (
    <section className="g-block" data-tone={tone}>
      <header className="g-block-head">
        <h3>{title}</h3>
        {hint ? <p>{hint}</p> : null}
      </header>
      {children}
    </section>
  );
}

/** 怎么做：编号动作，一步一行，右侧直接跳过去。 */
export function GuideSteps({ items }: { items: readonly GuideStepItem[] }) {
  const nav = useNavigate();
  return (
    <ol className="g-steps">
      {items.map((item, index) => (
        <li className="g-step" key={item.title}>
          <span className="g-step-num" aria-hidden="true">
            {index + 1}
          </span>
          <span className="g-step-copy">
            <strong>{item.title}</strong>
            {item.detail ? <small>{item.detail}</small> : null}
          </span>
          {item.action ? (
            <button
              type="button"
              className="g-step-go"
              onClick={() => {
                const action = item.action!;
                if ("to" in action) {
                  nav(action.to);
                } else {
                  void openUrl(action.href).catch(() => {
                    // 没有系统浏览器权限时静默失败：地址本身可复制
                  });
                }
              }}
            >
              {item.action.label}
              <ArrowRight aria-hidden="true" />
            </button>
          ) : null}
        </li>
      ))}
    </ol>
  );
}

/** 做到就算过：灯 + 一句判定标准。 */
export function GuideChecks({ items }: { items: readonly GuideCheckItem[] }) {
  return (
    <ul className="g-checks">
      {items.map((item) => (
        <li
          className="g-check"
          key={item.label}
          data-tone={item.tone ?? "idle"}
        >
          <span className="g-dot" aria-hidden="true" />
          <span className="g-check-copy">
            <strong>{item.label}</strong>
            {item.detail ? <small>{item.detail}</small> : null}
          </span>
        </li>
      ))}
    </ul>
  );
}

/** 别踩：一行一条，宁少勿多。 */
export function GuideAvoid({ items }: { items: readonly string[] }) {
  return (
    <ul className="g-avoid">
      {items.map((item) => (
        <li key={item}>{item}</li>
      ))}
    </ul>
  );
}
