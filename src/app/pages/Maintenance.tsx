import { useCallback, useEffect, useState, type ComponentType } from "react";
import {
  ArrowLeft,
  ArrowRight,
  ArrowUpRight,
  Fingerprint,
  Globe,
  RefreshCw,
  ScanLine,
  ShieldCheck,
  UserPlus,
  Zap,
  type LucideIcon,
} from "lucide-react";
import { Link, useLocation, useNavigate } from "react-router-dom";
import { api, type MaintenanceStatus } from "../../services/api";
import { usePolling } from "../../shared/hooks/usePolling";
import PageHeader from "../../shared/ui/PageHeader";
import AccountOnboarding from "./maintenance/AccountOnboarding";
import DetectOnboarding from "./maintenance/DetectOnboarding";
import FingerprintOnboarding from "./maintenance/FingerprintOnboarding";
import ProxyOnboarding from "./maintenance/ProxyOnboarding";
import ProOnboarding from "./maintenance/ProOnboarding";

/** 首次使用的五步：顺序即上手顺序，左侧导航与页面内容都从这里生成，避免两处文案漂移。 */
interface Step {
  id: string;
  label: string;
  /** 导航第二行：这一步做到什么就算过 */
  hint: string;
  icon: LucideIcon;
  /** 对应的功能页 */
  to: string;
  cta: string;
  /** 一句话目标：新手只读这一句也知道要干什么 */
  intro: string;
}

/** 每一步的正文组件：只有「怎么做 / 做到就算过 / 别踩」三块，按 id 一一对应。 */
const GUIDES: Record<string, ComponentType> = {
  proxy: ProxyOnboarding,
  detect: DetectOnboarding,
  browser: FingerprintOnboarding,
  account: AccountOnboarding,
  subscription: ProOnboarding,
};

const STEPS: readonly Step[] = [
  {
    id: "proxy",
    label: "代理",
    hint: "导入线路并接管流量",
    icon: Globe,
    to: "/overview",
    cta: "打开代理中心",
    intro: "买一条干净的美国住宅 IP，把整台机器的出口接过去。",
  },
  {
    id: "detect",
    label: "检测",
    hint: "确认出口与指纹一致",
    icon: ScanLine,
    to: "/detect",
    cta: "开始检测",
    intro: "跑一轮检测，把红的修掉，页头变成「就绪」再往下走。",
  },
  {
    id: "browser",
    label: "指纹浏览器",
    hint: "一账号一环境",
    icon: Fingerprint,
    to: "/fingerprint-browser",
    cta: "打开指纹浏览器",
    intro: "给这个账号建一份独立环境：一个账号只待在一份资料里。",
  },
  {
    id: "account",
    label: "注册及使用",
    hint: "Gmail 注册 · 三天观察",
    icon: UserPlus,
    to: "/fingerprint-browser",
    cta: "去注册",
    intro: "在指纹浏览器里用自己注册的 Gmail 注册，前三天正常聊。",
  },
  {
    id: "subscription",
    label: "订阅",
    hint: "Claude Pro · 支付与续费",
    icon: Zap,
    to: "/fingerprint-browser",
    cta: "去订阅",
    intro: "三天没问题再订 Pro：绑好支付、确认续费日、记下退款入口。",
  },
];

/** 地址栏里的步骤：`?step=browser`；非法值一律回落到第一步。 */
const stepFromSearch = (search: string): string | null => {
  const id = new URLSearchParams(search).get("step");
  return STEPS.some((step) => step.id === id) ? id : null;
};

/**
 * 首次使用与维护。
 *
 * 版式与设置页一致：左侧常驻的五步分类栏（沿用 `.settings-rail` 那套），
 * 右侧一次只呈现一个步骤——点分类栏换页，一步一页，不再把五步堆成一条长列表。
 * 当前步骤记在地址栏（`?step=`），刷新界面或前进后退都能回到同一页。
 * 页头继续显示应用 / 内核 / 服务版本，数据取自 `maintenance_status`。
 */
export default function Maintenance() {
  const nav = useNavigate();
  const location = useLocation();
  const [status, setStatus] = useState<MaintenanceStatus | null>(null);
  const [error, setError] = useState("");
  const [refreshing, setRefreshing] = useState(false);
  const [stepId, setStepId] = useState<string>(
    () => stepFromSearch(location.search) ?? STEPS[0].id,
  );

  /** 从别处带着 `?step=` 进来（或点了后退）时跟着切换。 */
  useEffect(() => {
    const next = stepFromSearch(location.search);
    if (next) setStepId(next);
  }, [location.search]);

  const index = Math.max(
    0,
    STEPS.findIndex((item) => item.id === stepId),
  );
  const step = STEPS[index];
  const previous = index > 0 ? STEPS[index - 1] : null;
  const following = index < STEPS.length - 1 ? STEPS[index + 1] : null;

  const goStep = useCallback(
    (id: string) => {
      setStepId(id);
      nav({ search: `?step=${id}` }, { replace: true });
    },
    [nav],
  );

  const load = useCallback(async (isCurrent: () => boolean = () => true) => {
    try {
      const next = await api.maintenanceStatus();
      if (isCurrent()) {
        setStatus(next);
        setError("");
      }
    } catch (e) {
      if (isCurrent()) setError(String(e));
    }
  }, []);
  usePolling(load, 15000);

  /** 页头的「刷新状态」只重读维护状态。 */
  const refresh = useCallback(async () => {
    setRefreshing(true);
    try {
      await load();
    } finally {
      setRefreshing(false);
    }
  }, [load]);

  const StepIcon = step.icon;
  /** 当前步骤的图文引导；没有对应组件时只渲染页头与相关入口。 */
  const StepGuide = GUIDES[step.id];

  return (
    <div className="maintenance-page">
      <PageHeader
        icon={ShieldCheck}
        title="首次使用与维护"
        meta={
          <>
            <span>
              应用 <strong>v{status?.appVersion ?? "读取中"}</strong>
            </span>
            <i className="proxy-meta-dot" aria-hidden="true" />
            <span>
              内核 <strong>{status?.kernelVersion ?? "未在线"}</strong>
            </span>
            <i className="proxy-meta-dot" aria-hidden="true" />
            <span>
              服务 <strong>{status?.serviceVersion ?? "未就绪"}</strong>
            </span>
          </>
        }
        actions={
          <button
            type="button"
            className="proxy-action"
            disabled={refreshing}
            aria-busy={refreshing}
            aria-label="刷新维护状态"
            onClick={() => void refresh()}
          >
            <RefreshCw
              className={refreshing ? "animate-spin" : ""}
              aria-hidden="true"
            />
            {refreshing ? "刷新中…" : "刷新状态"}
          </button>
        }
        note={error ? <span role="alert">{error}</span> : null}
      />

      <div className="maintenance-body">
        <nav className="settings-rail" aria-label="首次使用步骤">
          <div className="settings-rail-group">
            <span className="settings-rail-label">首次使用</span>
            {STEPS.map((item, itemIndex) => {
              const Icon = item.icon;
              return (
                <button
                  key={item.id}
                  type="button"
                  className="settings-rail-item"
                  aria-current={stepId === item.id ? "page" : undefined}
                  onClick={() => goStep(item.id)}
                >
                  <span className="settings-rail-icon" aria-hidden="true">
                    <Icon />
                  </span>
                  <span className="settings-rail-copy">
                    <span className="settings-rail-title">{item.label}</span>
                    <span className="settings-rail-hint">{item.hint}</span>
                  </span>
                  <span className="maintenance-rail-order" aria-hidden="true">
                    {itemIndex + 1}
                  </span>
                </button>
              );
            })}
          </div>
          <div className="settings-rail-foot">
            <span>云阔 v{status?.appVersion ?? "读取中"}</span>
            <Link className="settings-rail-link" to="/settings">
              偏好设置
              <ArrowUpRight aria-hidden="true" />
            </Link>
          </div>
        </nav>

        <div className="settings-content" key={step.id}>
          <section
            className={
              StepGuide ? "maintenance-step is-guide" : "maintenance-step"
            }
            aria-labelledby={`step-${step.id}-title`}
          >
            <header className="maintenance-step-head">
              <span className="maintenance-step-index" aria-hidden="true">
                {index + 1}
              </span>
              <div className="maintenance-step-copy">
                <h2 id={`step-${step.id}-title`}>
                  <StepIcon aria-hidden="true" />
                  {step.label}
                </h2>
                <p>{step.intro}</p>
              </div>
              <button
                type="button"
                className="maintenance-step-go"
                onClick={() => nav(step.to)}
              >
                {step.cta}
                <ArrowRight aria-hidden="true" />
              </button>
            </header>
            <StepGuide />
          </section>

          <div className="maintenance-pager">
            {previous ? (
              <button
                type="button"
                className="maintenance-pager-button"
                onClick={() => goStep(previous.id)}
              >
                <ArrowLeft aria-hidden="true" />
                上一步 · {previous.label}
              </button>
            ) : (
              <span />
            )}
            {following ? (
              <button
                type="button"
                className="maintenance-pager-button"
                data-variant="next"
                onClick={() => goStep(following.id)}
              >
                下一步 · {following.label}
                <ArrowRight aria-hidden="true" />
              </button>
            ) : (
              <button
                type="button"
                className="maintenance-pager-button"
                data-variant="next"
                onClick={() => nav("/overview")}
              >
                回到代理中心
                <ArrowRight aria-hidden="true" />
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
