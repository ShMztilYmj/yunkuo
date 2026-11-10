import {
  GuideAvoid,
  GuideChecks,
  GuideSection,
  GuideSteps,
  useBrowserEnvironment,
  type GuideCheckItem,
  type GuideStepItem,
} from "./guideKit";

/* 「指纹浏览器」这一步：一份资料一个身份。
   判定灯读的是 browser_environment_status，和指纹浏览器页同一份数据。 */

const STEPS: readonly GuideStepItem[] = [
  {
    title: "新建资料，地区选对",
    detail: "做美国站就选美国 · 洛杉矶，语言和时区会自动带出来。",
    action: { label: "去新建", to: "/fingerprint-browser" },
  },
  {
    title: "改完先点「保存配置」",
    detail: "不保存，启动按钮一直是灰的 —— 新手最常卡在这。",
  },
  {
    title: "启动，然后核对四项",
    detail: "ippure 里 IP、地区、时区、语言四项要互相对得上。",
    action: { label: "去启动", to: "/fingerprint-browser" },
  },
  {
    title: "一个账号只待一份资料",
    detail: "注册和登录都用这一份，别多开，也别中途换指纹。",
  },
];

const AVOID: readonly string[] = [
  "一份资料登两个号 —— 等于自己把两个号绑在一起",
  "中途换指纹 —— 像换了台设备，立刻触发验证",
  "在指纹页换国家 —— 出口只跟代理节点走",
];

export default function FingerprintOnboarding() {
  const env = useBrowserEnvironment();

  const checks: readonly GuideCheckItem[] = [
    {
      label: "引擎已准备",
      detail: "页头徽标显示「引擎已准备」",
      tone: env ? (env.installed ? "ok" : "todo") : "idle",
    },
    {
      label: "资料已保存",
      detail: "有语言和时区，启动按钮不是灰的",
      tone: env?.config ? "ok" : "idle",
    },
    {
      label: "启动没有报错",
      detail: "系统代理、虚拟网卡、出站模式三项都过",
      tone: env?.running ? "ok" : "idle",
    },
    {
      label: "四项一致",
      detail: "出口 IP、地区、浏览器时区、浏览器语言",
      tone: "idle",
    },
  ];

  return (
    <div className="g-guide">
      <GuideSection title="怎么做" hint="一份资料对应一个账号，建好就不再动它">
        <GuideSteps items={STEPS} />
      </GuideSection>
      <GuideSection title="做到就算过" hint="前两盏灯跟着当前资料自己变">
        <GuideChecks items={checks} />
      </GuideSection>
      <GuideSection title="别踩这三条" tone="warn">
        <GuideAvoid items={AVOID} />
      </GuideSection>
    </div>
  );
}
