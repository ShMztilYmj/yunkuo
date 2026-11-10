import { useAppStore } from "../../../store";
import {
  GuideAvoid,
  GuideChecks,
  GuideSection,
  GuideSteps,
  type GuideCheckItem,
  type GuideStepItem,
} from "./guideKit";

/* 「代理」这一步：四步把出口定下来。
   文案只留动作和判定，判定灯里能读实时状态的尽量读真实配置。 */

const STEPS: readonly GuideStepItem[] = [
  {
    title: "买一条美国住宅 IP",
    detail: "住宅、独享、纯净度 95 分以上；机房 IP 不要。",
    action: {
      label: "看服务商",
      href: "https://miyaip.com/?invitecode=3646291",
    },
  },
  {
    title: "把订阅导入云阔",
    detail: "服务商后台复制 Clash / Mihomo 链接，粘进「订阅管理」。",
    action: { label: "去导入", to: "/profiles" },
  },
  {
    title: "手动选一条美国节点",
    detail: "别用自动选择或负载均衡：出口会跳，账号就串了。",
    action: { label: "选节点", to: "/proxies" },
  },
  {
    title: "打开接管并应用",
    detail:
      "系统代理开、TUN 开、IPv6 关，方案选「自定义 · 全局」，点一次应用。",
    action: { label: "去代理中心", to: "/overview" },
  },
];

const AVOID: readonly string[] = [
  "用自动选择 / 负载均衡 —— 出口说变就变，账号跟着串",
  "买机房 IP（Hosting / Datacenter）—— 一眼就露",
  "纯净度 90 分以下硬上 —— 省下的钱不够赔号",
];

export default function ProxyOnboarding() {
  const config = useAppStore((s) => s.config);
  const tone = (on: boolean | undefined) =>
    on === undefined ? "idle" : on ? "ok" : "todo";

  const checks: readonly GuideCheckItem[] = [
    { label: "系统代理已开", tone: tone(config?.enable_system_proxy) },
    { label: "TUN 已开", tone: tone(config?.enable_tun) },
    {
      label: "IPv6 已关",
      tone: config ? tone(!config.ipv6) : "idle",
    },
    {
      label: "节点固定，出口在美国",
      detail: "检测页的出口 IP 显示美国，就不是机房段",
      tone: "idle",
    },
  ];

  return (
    <div className="g-guide">
      <GuideSection title="怎么做" hint="按顺序四步，做完就能接管整台机器">
        <GuideSteps items={STEPS} />
      </GuideSection>
      <GuideSection
        title="做到就算过"
        hint="前三盏灯是自己绿的，最后一盏去检测页确认"
      >
        <GuideChecks items={checks} />
      </GuideSection>
      <GuideSection title="别踩这三条" tone="warn">
        <GuideAvoid items={AVOID} />
      </GuideSection>
    </div>
  );
}
