import { useEnvironmentVerdict } from "../../../features/diagnostics/hooks/useEnvironmentVerdict";
import {
  GuideAvoid,
  GuideChecks,
  GuideSection,
  GuideSteps,
  type GuideCheckItem,
  type GuideStepItem,
} from "./guideKit";

/* 「注册及使用」这一步：在指纹浏览器里注册，前三天别折腾。
   判定灯里的门禁结论和检测页、侧栏是同一份。 */

const STEPS: readonly GuideStepItem[] = [
  {
    title: "从指纹浏览器进 Claude",
    detail: "同一份资料、同一个窗口，别开无痕。",
    action: { label: "打开 Claude", to: "/fingerprint-browser" },
  },
  {
    title: "用自己注册的 Gmail 登",
    detail: "选 Continue with Google；要手机号就给能收码的，跳出 KYC 先停手。",
  },
  {
    title: "前三天正常聊",
    detail: "早中晚各几轮，不换节点、不换指纹、不加设备。",
  },
  {
    title: "三天没异常再订阅",
    detail: "到下一步订 Pro，别急着当天就付钱。",
  },
];

const AVOID: readonly string[] = [
  "买成品号 / 用一次性邮箱 —— 邮箱本身就带风控记录",
  "注册当天就订 Pro —— 号还没稳，钱容易打水漂",
  "电脑手机同时在线 —— 两条出口，最容易被抓",
];

export default function AccountOnboarding() {
  const { pass, scanned } = useEnvironmentVerdict();

  const checks: readonly GuideCheckItem[] = [
    {
      label: "环境已经就绪",
      detail: "上一步的检测是「就绪」，不是「需处理」",
      tone: scanned ? (pass ? "ok" : "todo") : "idle",
    },
    {
      label: "全程同一份资料、同一条出口",
      detail: "注册、三天观察、订阅都不换",
      tone: "idle",
    },
    {
      label: "邮箱是自己注册的 Gmail",
      tone: "idle",
    },
    {
      label: "三天没有异常",
      detail: "没有 403、没有连续验证码、没弹 KYC",
      tone: "idle",
    },
  ];

  return (
    <div className="g-guide">
      <GuideSection title="怎么做" hint="从注册到第三天，一路都别换环境">
        <GuideSteps items={STEPS} />
      </GuideSection>
      <GuideSection title="做到就算过" hint="第一条灯跟着门禁结论自己变">
        <GuideChecks items={checks} />
      </GuideSection>
      <GuideSection title="别踩这三条" tone="warn">
        <GuideAvoid items={AVOID} />
      </GuideSection>
    </div>
  );
}
