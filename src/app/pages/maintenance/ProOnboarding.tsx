import {
  GuideAvoid,
  GuideChecks,
  GuideSection,
  GuideSteps,
  type GuideCheckItem,
  type GuideStepItem,
} from "./guideKit";

/* 「订阅」这一步：三天稳了再付钱，付完记住续费日和退款入口。 */

const STEPS: readonly GuideStepItem[] = [
  {
    title: "先把卡绑进钱包",
    detail:
      "招行 Visa 在掌上生活打开「境外线上交易」，再绑进 Apple Pay / Google Pay。",
  },
  {
    title: "在指纹浏览器里订 Pro",
    detail: "同一份资料进 claude.ai/upgrade，选 Pro，用钱包付款。",
    action: { label: "打开 Claude", to: "/fingerprint-browser" },
  },
  {
    title: "确认扣款、记下续费日",
    detail: "账单商户是 ANTHROPIC；设置里能看到下次扣款时间。",
  },
  {
    title: "把退款入口记下来",
    detail: "网页订的找 Claude 支持；应用内购的找 Apple 或 Google。",
  },
];

const AVOID: readonly string[] = [
  "一上来就订 Max —— 新号风控更高，Pro 够用",
  "找代订 / 共享号 —— 随时被收回，退款也找不到人",
  "把退款当取消 —— 两步都要做，否则下月照扣",
];

export default function ProOnboarding() {
  const checks: readonly GuideCheckItem[] = [
    { label: "设置里显示 Pro，不是 Free", tone: "idle" },
    { label: "能看到下次扣费日期", tone: "idle" },
    {
      label: "卡扣得动",
      detail: "境外线上交易已开，或商店余额够（含税多留几美元）",
      tone: "idle",
    },
    {
      label: "知道怎么取消、怎么退",
      detail: "网页找官方，App 内购找 Apple / Google 订单",
      tone: "idle",
    },
  ];

  return (
    <div className="g-guide">
      <GuideSection title="怎么做" hint="先绑支付再下单，付完把退路记清楚">
        <GuideSteps items={STEPS} />
      </GuideSection>
      <GuideSection title="做到就算过" hint="四条都清楚了，账号才算能用起来">
        <GuideChecks items={checks} />
      </GuideSection>
      <GuideSection title="别踩这三条" tone="warn">
        <GuideAvoid items={AVOID} />
      </GuideSection>
    </div>
  );
}
