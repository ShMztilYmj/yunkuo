/** Turnstile 官方恒成功测试密钥：仅检查网络和组件加载，不代表真实站点风控结果。 */

export type TurnState = "idle" | "running" | "ok" | "fail";

export interface TurnItem {
  mode: "managed" | "nonInteractive";
  label: string;
  sitekey: string;
  state: TurnState;
  /** token 获取耗时（ms），失败为 null */
  ms: number | null;
  detail: string;
}

export const TURNSTILE_KEYS = {
  managed: "1x00000000000000000000AA",
  nonInteractive: "1x00000000000000000000BB",
};

interface TurnstileWidgetParams {
  sitekey: string;
  callback?: (token: string) => void;
  "error-callback"?: (code: string) => void;
  "expired-callback"?: () => void;
  "timeout-callback"?: () => void;
  appearance?: "always" | "execute" | "interaction-only";
  size?: "normal" | "flexible" | "compact";
  theme?: "light" | "dark" | "auto";
}

interface TurnstileApi {
  render: (el: HTMLElement, params: TurnstileWidgetParams) => string;
  reset: (id?: string) => void;
  remove: (id: string) => void;
}

declare global {
  interface Window {
    turnstile?: TurnstileApi;
    __cocTurnstileReady?: () => void;
  }
}

const SCRIPT_SRC =
  "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit&onload=__cocTurnstileReady";

let scriptLoading: Promise<TurnstileApi> | null = null;

/** 自检通过后保留在页面上的 widget：重跑前用它精确回收，避免 iframe 叠加 */
const liveWidgetIds: Partial<Record<TurnItem["mode"], string>> = {};

/** widget 主题跟随应用主题（html[data-theme]），否则亮色页面里会出现深色组件 */
function preferredTheme(): "light" | "dark" {
  const attr = document.documentElement.dataset.theme;
  if (attr === "light" || attr === "dark") return attr;
  let mql: MediaQueryList | null = null;
  try {
    mql =
      typeof window.matchMedia === "function"
        ? window.matchMedia("(prefers-color-scheme: dark)")
        : null;
  } catch {
    mql = null;
  }
  return mql?.matches ? "dark" : "light";
}

/** 注入 Turnstile 脚本（explicit 渲染模式，只注入一次） */
export function loadTurnstile(): Promise<TurnstileApi> {
  if (window.turnstile) return Promise.resolve(window.turnstile);
  if (scriptLoading) return scriptLoading;

  scriptLoading = new Promise<TurnstileApi>((resolve, reject) => {
    const timeout = setTimeout(() => {
      reject(
        new Error(
          "Turnstile 脚本加载超时 —— 无法连上 challenges.cloudflare.com",
        ),
      );
    }, 12000);

    window.__cocTurnstileReady = () => {
      clearTimeout(timeout);
      if (window.turnstile) resolve(window.turnstile);
      else reject(new Error("Turnstile API 未就绪"));
    };

    const s = document.createElement("script");
    s.src = SCRIPT_SRC;
    s.async = true;
    s.defer = true;
    s.onerror = () => {
      clearTimeout(timeout);
      reject(
        new Error(
          "Turnstile 脚本加载失败 —— 无法连上 challenges.cloudflare.com",
        ),
      );
    };
    document.head.appendChild(s);
  });
  scriptLoading = scriptLoading.catch((error) => {
    scriptLoading = null;
    document.querySelector(`script[src="${SCRIPT_SRC}"]`)?.remove();
    throw error;
  });
  return scriptLoading;
}

export function initialItems(): TurnItem[] {
  return [
    {
      mode: "managed",
      label: "托管模式（Managed）",
      sitekey: TURNSTILE_KEYS.managed,
      state: "idle",
      ms: null,
      detail: "恒成功测试密钥，仅检查可见组件加载",
    },
    {
      mode: "nonInteractive",
      label: "非交互模式（Non-interactive）",
      sitekey: TURNSTILE_KEYS.nonInteractive,
      state: "idle",
      ms: null,
      detail: "恒成功测试密钥，仅检查隐形组件加载",
    },
  ];
}

const SOLVE_TIMEOUT = 15000;

/** 运行一轮完整的两种模式测试；widgets 会渲染在传入的容器里 */
export async function runTurnstileTest(
  mount: { managed: HTMLElement | null; nonInteractive: HTMLElement | null },
  onUpdate: (items: TurnItem[]) => void,
): Promise<TurnItem[]> {
  let items = initialItems();
  const emit = () => onUpdate([...items]);

  let api: TurnstileApi;
  try {
    // 脚本要走网络（首次约数秒），先把两种模式标成进行中，
    // 否则卡片会一直停在「待检测」，看起来像没反应。
    items = items.map((it) => ({
      ...it,
      state: "running",
      ms: null,
      detail: "正在加载 Turnstile 脚本…",
    }));
    emit();
    api = await loadTurnstile();
  } catch (e) {
    items = items.map((it) => ({
      ...it,
      state: "fail",
      ms: null,
      detail: String((e as Error).message ?? e),
    }));
    emit();
    return items;
  }

  const tests: Array<Promise<void>> = [];
  for (const it of items) {
    const container =
      it.mode === "managed" ? mount.managed : mount.nonInteractive;
    tests.push(
      (async () => {
        if (!container) {
          items = items.map((x) =>
            x.mode === it.mode
              ? { ...x, state: "fail", detail: "容器未挂载" }
              : x,
          );
          emit();
          return;
        }
        // 先把上一轮留下的 widget / 半截 iframe 清干净，避免重跑后叠框
        const previous = liveWidgetIds[it.mode];
        if (previous) {
          try {
            api.remove(previous);
          } catch {
            /* 容器已重建或已卸载，忽略 */
          }
          delete liveWidgetIds[it.mode];
        }
        container.replaceChildren();

        items = items.map((x) =>
          x.mode === it.mode
            ? { ...x, state: "running", ms: null, detail: "组件自检中…" }
            : x,
        );
        emit();

        const start = performance.now();
        let widgetId: string | null = null;
        let finalState: TurnState = "fail";
        await new Promise<void>((resolve) => {
          let done = false;
          const finish = (state: TurnState, detail: string) => {
            if (done) return;
            done = true;
            finalState = state;
            clearTimeout(timer);
            const ms =
              state === "ok" ? Math.round(performance.now() - start) : null;
            items = items.map((x) =>
              x.mode === it.mode ? { ...x, state, ms, detail } : x,
            );
            emit();
            resolve();
          };
          const timer = setTimeout(() => {
            finish("fail", "组件自检超时（15s）");
          }, SOLVE_TIMEOUT);
          try {
            widgetId = api.render(container, {
              sitekey: it.sitekey,
              callback: () => finish("ok", "组件自检完成，不代表真实风控通过"),
              "error-callback": (code) =>
                finish("fail", `组件加载出错（代码 ${code}）`),
              "timeout-callback": () => finish("fail", "组件自检超时"),
              theme: preferredTheme(),
              size: it.mode === "managed" ? "flexible" : "normal",
            });
            // 组件已经注入容器：让卡片把「加载中」占位换成组件本身
            if (!done) {
              items = items.map((x) =>
                x.mode === it.mode ? { ...x, detail: "等待组件返回令牌…" } : x,
              );
              emit();
            }
          } catch (e) {
            finish("fail", `渲染失败：${String(e)}`);
          }
        });

        if (widgetId) {
          // finalState 在 Promise 回调里赋值，TS 会把这里的类型收窄成初始值 "fail"。
          if ((finalState as TurnState) === "ok") {
            // 通过后保留 widget：把组件留在卡片里，用户能看到勾选/令牌状态
            liveWidgetIds[it.mode] = widgetId;
          } else {
            // 失败/超时：回收，别留一个不可用的残框
            try {
              api.remove(widgetId);
            } catch {
              /* 忽略 */
            }
            container.replaceChildren();
          }
        }
      })(),
    );
  }

  await Promise.all(tests);
  return items;
}
