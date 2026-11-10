import { TRUST_BAND_TEXT, type TrustResult } from "../policies/trust";
import { BAND_COLORS } from "./constants";
import TrustRing from "./TrustRing";

/** 信任评分卡：环形评分 + 档位文字 + 风险说明 */
export default function TrustPanel({
  done,
  trust,
  previous = null,
}: {
  done: boolean;
  trust: TrustResult;
  /** 上次评分：只做参照，不填当前进度 */
  previous?: number | null;
}) {
  return (
    <div className="trust-panel flex flex-col items-center justify-center gap-3 ui-panel rounded-2xl border border-zinc-800 bg-zinc-900/40 p-6">
      <TrustRing
        score={done ? trust.score : null}
        previous={done ? null : previous}
      />
      <div
        className={`text-lg font-semibold ${done ? BAND_COLORS[trust.band].text : "text-zinc-500"}`}
      >
        {done ? TRUST_BAND_TEXT[trust.band] : "待检测"}
      </div>
      <div className="text-[11px] text-zinc-500">
        <div>
          {done
            ? `综合风险 ${trust.risk}/100`
            : previous != null
              ? `上次 ${previous} 分 · 需重新检测`
              : "完成检测后显示评分与风险"}
        </div>
        <div className="mt-1">信任评分越高，环境一致性越好</div>
      </div>
    </div>
  );
}
