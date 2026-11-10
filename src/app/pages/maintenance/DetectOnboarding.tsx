import { useEnvironmentVerdict } from "../../../features/diagnostics/hooks/useEnvironmentVerdict";
import {
  GuideAvoid,
  GuideChecks,
  GuideSection,
  GuideSteps,
  type GuideCheckItem,
  type GuideStepItem,
} from "./guideKit";

/* 「检测」这一步：跑一轮、把红的修掉、再跑一轮。
   两盏灯读的是全应用同一个门禁结论（和侧栏、启动页同一份）。 */

const STEPS: readonly GuideStepItem[] = [
  {
    title: "点一次「一键检测并修复」",
    detail: "等它跑完，中途别切节点。",
    action: { label: "去检测", to: "/detect" },
  },
  {
    title: "红了的按清单逐条修",
    detail: "能一键修的先修，剩下的按提示去改。",
    action: { label: "看修复清单", to: "/detect" },
  },
  {
    title: "改完再跑一轮",
    detail: "时区、语言、网卡动过，上一轮结论就作废了。",
    action: { label: "重新检测", to: "/detect" },
  },
];

const AVOID: readonly string[] = [
  "检测中途切节点 —— 这一轮基本白跑",
  "只盯评分 —— 门禁看八项，不看分数",
  "拿页头的旧结论当现在 —— 换过设置就要重测",
];

export default function DetectOnboarding() {
  const { pass, scanned, scanning, issues } = useEnvironmentVerdict();

  const checks: readonly GuideCheckItem[] = [
    {
      label: "页头显示「就绪」",
      detail: "不是「上次检测 …」，也不是「需处理」",
      tone: scanning
        ? "idle"
        : scanned && pass
          ? "ok"
          : scanned
            ? "todo"
            : "idle",
    },
    {
      label: "修复清单为空",
      detail: "还剩一条待处理，就先别往下走",
      tone: scanned ? (issues.length === 0 ? "ok" : "todo") : "idle",
    },
    {
      label: "八项全绿",
      detail: "内核、TUN、系统代理、全局、节点、出口地区、Claude 连通、时区",
      tone: "idle",
    },
    {
      label: "出口干净",
      detail: "DNS 没有境内解析器、IPv6 没暴露、IP 与时区语言自洽",
      tone: "idle",
    },
  ];

  return (
    <div className="g-guide">
      <GuideSection title="怎么做" hint="一轮跑完、修干净、再跑一轮确认">
        <GuideSteps items={STEPS} />
      </GuideSection>
      <GuideSection title="做到就算过" hint="前两盏灯跟着检测结果自己变">
        <GuideChecks items={checks} />
      </GuideSection>
      <GuideSection title="别踩这三条" tone="warn">
        <GuideAvoid items={AVOID} />
      </GuideSection>
    </div>
  );
}
