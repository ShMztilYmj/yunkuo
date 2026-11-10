import { Globe2, Info, Loader2, ScanLine } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { api, type IpIntel, type NativeDetect } from "../../../services/api";
import {
  abuseLevel,
  bandwidthLabel,
  blacklistView,
  deepChecks,
  formatCount,
  nativeVerdict,
  peeringdbDetail,
  rpkiView,
  scenarioScores,
  threatHeadline,
  trafficSplit,
} from "../policies/ipIntelPolicy";
import {
  computeIpScore,
  IP_BAND_META,
  type IpScoreFactor,
} from "../policies/ipScore";
import type { ExitResult, Verdict } from "../signals/exit";
import { relativeTime } from "../lastScan";
import type { ExitSnapshot } from "../exitSnapshot";
import CcBadge from "./CcBadge";

/**
 * 出口通道卡：顶部 IP 评分条 + 场景/ASN 两栏 + 技术/威胁/深度三栏。
 * 布局参照公开 IP 查询页（IP 原生性 / ASN 画像 / 威胁指标 / 深度检测）的信息分组，
 * 所有推断项都在标题上标注「本地推断」，数据缺失时显示「—」而不是猜测。
 *
 * 本轮还没跑完时先用本机保存的上次快照渲染，并标出「上次检测」，避免一重启就空白。
 */

const TONE_CHIP: Record<Verdict, string> = {
  g: "border-emerald-500/25 bg-emerald-500/10 text-emerald-400",
  y: "border-amber-500/25 bg-amber-500/10 text-amber-400",
  r: "border-red-500/25 bg-red-500/10 text-red-400",
  n: "border-zinc-700/70 bg-zinc-800/50 text-zinc-400",
};

const TONE_BAR: Record<Verdict, string> = {
  g: "bg-emerald-400",
  y: "bg-amber-400",
  r: "bg-red-400",
  n: "bg-zinc-600",
};

const TONE_RING: Record<Verdict, string> = {
  g: "stroke-emerald-400",
  y: "stroke-amber-400",
  r: "stroke-red-400",
  n: "stroke-zinc-600",
};

/** 评分条底色：0 高危 → 100 可信 */
const SCORE_GRADIENT =
  "linear-gradient(90deg,#ef4444 0%,#f97316 30%,#facc15 52%,#84cc16 74%,#22c55e 100%)";

function Chip({
  tone,
  title,
  children,
}: {
  tone: Verdict;
  title?: string;
  children: ReactNode;
}) {
  return (
    <span
      title={title || undefined}
      className={`inline-flex max-w-full items-center gap-1 truncate rounded-md border px-1.5 py-[1px] text-[10.5px] leading-[16px] ${TONE_CHIP[tone]}`}
    >
      <span className={`h-1 w-1 shrink-0 rounded-full ${TONE_BAR[tone]}`} />
      <span className="truncate">{children}</span>
    </span>
  );
}

function Row({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-3 border-t border-dashed border-zinc-700/60 py-[7px] first:border-t-0 first:pt-0 last:pb-0">
      <span className="flex shrink-0 items-center gap-1 text-[11px] text-zinc-500">
        {label}
        {hint && (
          <span title={hint} aria-label={hint} className="inline-flex">
            <Info className="h-3 w-3 cursor-help text-zinc-600" />
          </span>
        )}
      </span>
      <span className="flex min-w-0 items-center justify-end gap-1.5 text-[11px] text-zinc-300">
        {children}
      </span>
    </div>
  );
}

/** 整行占满的字段块：给条形图 / 圆环这类需要横向空间的内容用 */
function Block({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <div className="border-t border-dashed border-zinc-700/60 py-[7px]">
      <div className="mb-1.5 flex items-center gap-1 text-[11px] text-zinc-500">
        {label}
        {hint && (
          <span title={hint} aria-label={hint} className="inline-flex">
            <Info className="h-3 w-3 cursor-help text-zinc-600" />
          </span>
        )}
      </div>
      {children}
    </div>
  );
}

function MiniPanel({
  title,
  hint,
  children,
}: {
  title: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <div className="rounded-xl border border-zinc-800/80 bg-zinc-900/40 p-3.5">
      <div className="mb-2 flex items-baseline justify-between gap-2">
        <h3 className="text-[11.5px] font-medium text-zinc-300">{title}</h3>
        {hint && (
          <span className="shrink-0 text-[10px] text-zinc-600">{hint}</span>
        )}
      </div>
      {children}
    </div>
  );
}

function FactorStrip({ factors }: { factors: IpScoreFactor[] }) {
  if (!factors.length) return null;
  return (
    <div className="mt-2 flex flex-wrap gap-1.5">
      {factors.map((f) => (
        <span
          key={f.label}
          className={`inline-flex items-center gap-1 rounded-md border px-1.5 py-[1px] text-[10px] leading-[15px] ${TONE_CHIP[f.tone]}`}
        >
          <span className="font-mono tabular-nums">
            {f.delta > 0 ? `+${f.delta}` : f.delta}
          </span>
          <span>{f.label}</span>
        </span>
      ))}
    </div>
  );
}

/** 5 分制小圆环：场景评估里的 TikTok / 社媒 / AI */
function MiniGauge({ score, tone }: { score: number; tone: Verdict }) {
  const radius = 9;
  const circumference = 2 * Math.PI * radius;
  const filled = (Math.max(0, Math.min(5, score)) / 5) * circumference;
  return (
    <div className="relative h-7 w-7 shrink-0">
      <svg viewBox="0 0 24 24" className="h-7 w-7 -rotate-90">
        <circle
          cx="12"
          cy="12"
          r={radius}
          fill="none"
          strokeWidth="2.5"
          className="stroke-zinc-700/70"
        />
        <circle
          cx="12"
          cy="12"
          r={radius}
          fill="none"
          strokeWidth="2.5"
          strokeLinecap="round"
          strokeDasharray={`${filled} ${circumference}`}
          className={TONE_RING[tone]}
        />
      </svg>
      <span className="absolute inset-0 flex items-center justify-center text-[10px] font-medium tabular-nums text-zinc-300">
        {score}
      </span>
    </div>
  );
}

function TrafficBar({ human, bot }: { human: number; bot: number }) {
  return (
    <div className="w-full">
      <div className="flex items-center justify-between text-[10px] leading-none">
        <span className="text-emerald-400">人类 {human}%</span>
        <span className="text-red-400">机器 {bot}%</span>
      </div>
      <div className="mt-1.5 flex h-[6px] w-full overflow-hidden rounded-full bg-zinc-800">
        <div
          className="h-full bg-emerald-400/80"
          style={{ width: `${human}%` }}
        />
        <div className="h-full bg-red-400/80" style={{ width: `${bot}%` }} />
      </div>
    </div>
  );
}

function dash(value: string | null | undefined, fallback = "—") {
  return value && value.trim() ? value : fallback;
}

export default function ExitChannelsSection({
  done: liveDone,
  native,
  exit: liveExit,
  cached,
  intel: liveIntel,
  intelBusy,
}: {
  done: boolean;
  native: NativeDetect | null;
  exit: ExitResult | null;
  /** 本机保存的上次结果：本轮没拿到情报时顶上 */
  cached: ExitSnapshot | null;
  intel: IpIntel | null;
  intelBusy: boolean;
}) {
  const liveIp = liveExit?.primary?.ip || native?.exitIp || "";
  // 只在「本轮没有情报」且「快照 IP 与当前出口一致」时复用：
  // 换了节点还拿旧画像描述新出口，比空着更糟。
  const reuse =
    cached && !liveIntel && (!liveIp || cached.ip === liveIp) ? cached : null;

  const done = liveDone || reuse !== null;
  const exit = liveExit ?? reuse?.exit ?? null;
  const intel = liveIntel ?? reuse?.intel ?? null;
  const countryAllowed =
    native?.countryAllowed ?? reuse?.countryAllowed ?? null;

  const geo = exit?.primary ?? null;
  const targetIp = liveIp || reuse?.ip || "";
  const ip = targetIp || (done ? "未取得出口" : "待检测");
  // 出口层探测失败时仍用后端已知的出口 IP 打分（见 computeIpScore 的降级分支）。
  const scored = computeIpScore({
    done,
    ip: targetIp || null,
    exit,
    countryAllowed,
    intel,
  });
  const band = scored ? IP_BAND_META[scored.band] : null;

  const cc = (geo?.cc ?? native?.exitCountry ?? "").toUpperCase();
  const countryName = geo?.country || native?.exitCountryName || "地区未知";
  const city = geo?.city ?? "";
  const isp = intel?.isp || geo?.isp || "";
  const asn = intel?.asn ? `AS${intel.asn}` : (geo?.asn ?? "");
  const cls = intel?.networkClass ?? exit?.asnClass ?? "unknown";
  const asnType =
    intel?.asnType ?? (cls === "dc" ? "hosting" : cls === "res" ? "isp" : null);

  const traffic = trafficSplit(intel);
  const scenarios = scenarioScores(intel);
  const abuse = abuseLevel(intel?.riskScore ?? null);
  const abuseHint = intel?.abuserScore
    ? `ipapi.is 滥用分 ${intel.abuserScore}；本地分档基于 ${intel.riskScore ?? "—"}`
    : "proxycheck.io 风险分分档";
  const honey = blacklistView(intel);
  const rpki = rpkiView(intel?.rpki ?? null);
  const origin = nativeVerdict(intel, cc);
  const deep = deepChecks(intel);
  const threat = threatHeadline(intel);
  const bandwidth = bandwidthLabel(intel);

  const tri = exit?.triangleVerdict ?? "n";
  const v6 = exit?.v6Verdict ?? "n";

  const [openPorts, setOpenPorts] = useState<number[] | null>(null);
  const [scanBusy, setScanBusy] = useState(false);
  const [scanError, setScanError] = useState<string | null>(null);

  // 出口换 IP 后，上一轮的端口结果必须失效
  useEffect(() => {
    setOpenPorts(null);
    setScanError(null);
  }, [targetIp]);

  const scanPorts = async () => {
    if (!targetIp || scanBusy) return;
    setScanBusy(true);
    setScanError(null);
    try {
      const ports = await api.scanPorts(targetIp);
      setOpenPorts(ports);
    } catch (error) {
      setOpenPorts(null);
      setScanError(String(error));
    } finally {
      setScanBusy(false);
    }
  };

  return (
    <section
      aria-labelledby="exit-channels-title"
      className="ui-panel rounded-2xl border border-zinc-800 bg-zinc-900/40 p-6"
    >
      <div className="mb-1 flex items-center justify-between gap-3">
        <h2
          id="exit-channels-title"
          className="flex items-center gap-2 text-sm font-medium"
        >
          <Globe2 className="h-4 w-4 text-sky-400" />
          出口通道
        </h2>
        <span className="flex shrink-0 items-center gap-2 text-[10px] text-zinc-600">
          {intelBusy && (
            <span className="flex items-center gap-1 text-zinc-500">
              <Loader2 className="h-3 w-3 animate-spin" />
              情报查询中
            </span>
          )}
          {reuse && (
            <span
              title="本轮还没拿到情报，先显示本机保存的上次检测结果"
              className="rounded-md border border-zinc-700/70 bg-zinc-800/50 px-1.5 py-[1px] text-[10px] text-zinc-400"
            >
              上次检测 · {relativeTime(reuse.at)}
            </span>
          )}
          IP 评分 · 本地推断
        </span>
      </div>
      <div className="mb-3 text-[11px] text-zinc-600">
        出口 IP 与公开 IP 库画像；本轮没拿到情报时沿用本机保存的上次结果
      </div>

      {/* ---------- 顶栏：出口 IP + 0-100 评分条 ---------- */}
      <div
        className="rounded-xl border border-zinc-800 px-4 py-3.5"
        style={{
          background:
            "linear-gradient(90deg, color-mix(in srgb, var(--color-zinc-100) 7%, transparent), color-mix(in srgb, var(--color-zinc-100) 2%, transparent) 45%, color-mix(in srgb, var(--color-zinc-100) 6%, transparent))",
        }}
      >
        <div className="flex flex-wrap items-center gap-x-6 gap-y-3">
          <div className="min-w-0 flex-1">
            <div className="truncate font-mono text-[22px] font-semibold leading-tight tracking-tight text-zinc-100 select-text">
              {ip}
            </div>
            <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px] text-zinc-500">
              <span className="flex min-w-0 items-center gap-1.5">
                <CcBadge cc={cc} />
                <span className="truncate text-zinc-400">{countryName}</span>
                {city ? <span className="truncate">· {city}</span> : null}
              </span>
              {isp && (
                <>
                  <span className="text-zinc-700">/</span>
                  <span className="truncate">{isp}</span>
                </>
              )}
              {asn && (
                <>
                  <span className="text-zinc-700">/</span>
                  <span className="font-mono">{asn}</span>
                </>
              )}
            </div>
          </div>

          <div className="flex w-full shrink-0 items-center gap-3 sm:w-[320px]">
            <div className="min-w-0 flex-1">
              <div
                className="relative h-[7px] w-full rounded-full"
                style={{ background: SCORE_GRADIENT }}
                role="img"
                aria-label={
                  scored ? `IP 评分 ${scored.score} 分` : "IP 评分待检测"
                }
              >
                {scored && (
                  <span
                    className="absolute -top-[3px] h-[13px] w-[2px] rounded-full bg-white shadow-[0_0_0_1px_rgba(0,0,0,0.45)]"
                    style={{ left: `calc(${scored.score}% - 1px)` }}
                  />
                )}
              </div>
              <div className="mt-1 flex justify-between text-[9px] text-zinc-600">
                <span>0 高危</span>
                <span>25</span>
                <span>50</span>
                <span>75</span>
                <span>100 可信</span>
              </div>
            </div>
            <div
              className={`flex h-10 w-14 shrink-0 flex-col items-center justify-center rounded-lg border ${band?.chip ?? "border-zinc-700 bg-zinc-800/50 text-zinc-500"}`}
            >
              <span className="text-[16px] font-semibold leading-none tabular-nums">
                {scored ? scored.score : "--"}
              </span>
              <span className="mt-[3px] text-[9px] leading-none">
                {band?.label ?? (done ? "无法评分" : "待检测")}
              </span>
            </div>
          </div>
        </div>
        {scored && <FactorStrip factors={scored.factors} />}
      </div>

      {/* ---------- 两栏：使用场景 / ASN ---------- */}
      <div className="mt-2.5 grid gap-2.5 md:grid-cols-2">
        <MiniPanel title="使用场景 / 类型">
          <Row label="IP 原生性" hint={origin.detail}>
            <Chip tone={origin.tone}>{origin.label}</Chip>
          </Row>
          <Row label="标记" hint="机房 / 住宅由 IP 库的 hosting 标记推断">
            <Chip tone={cls === "dc" ? "y" : cls === "res" ? "g" : "n"}>
              {cls === "dc"
                ? "机房 IP"
                : cls === "res"
                  ? "住宅 IP"
                  : "属性未知"}
            </Chip>
          </Row>
          <Row label="运营商类型">
            <span className="truncate font-mono text-[11px]">
              {asnType === "hosting"
                ? "hosting（机房/托管）"
                : asnType === "isp"
                  ? "isp（宽带）"
                  : "未知"}
            </span>
          </Row>
          <Block label="人机流量" hint="按机房属性与滥用风险折算，本地推断">
            {traffic ? (
              <TrafficBar human={traffic.human} bot={traffic.bot} />
            ) : (
              <span className="text-[11px] text-zinc-600">—</span>
            )}
          </Block>
          <Block label="场景评估" hint="按出口类型与风险特征折算，本地推断">
            <div className="grid grid-cols-3 gap-2">
              {scenarios.map((s) => (
                <div
                  key={s.key}
                  className="flex min-w-0 items-center gap-2"
                  title={`${s.label}：${s.score}/5`}
                >
                  <MiniGauge score={s.score} tone={s.tone} />
                  <div className="min-w-0">
                    <div className="truncate text-[10.5px] leading-tight text-zinc-400">
                      {s.label}
                    </div>
                    <div
                      className={`text-[10px] leading-tight ${
                        s.tone === "n"
                          ? "text-zinc-500"
                          : s.tone === "g"
                            ? "text-emerald-400"
                            : s.tone === "y"
                              ? "text-amber-400"
                              : "text-red-400"
                      }`}
                    >
                      {s.tone === "n"
                        ? "待检测"
                        : s.score >= 3
                          ? "推荐"
                          : "不推荐"}
                    </div>
                  </div>
                </div>
              ))}
            </div>
          </Block>
          <Row label="服务商">
            <span className="truncate">{dash(isp)}</span>
          </Row>
        </MiniPanel>

        <MiniPanel title="ASN / 运营商">
          <Row label="ASN">
            <span className="truncate font-mono">{dash(asn)}</span>
          </Row>
          <Row label="ASN 归属">
            <span className="truncate">{dash(intel?.asnOrg)}</span>
          </Row>
          <Row label="ASN 自报类型">
            <span className="truncate font-mono">
              {dash(intel?.asnType)}{" "}
              <span className="text-zinc-600">
                （{cls === "dc" ? "机房/托管" : cls === "res" ? "宽带" : "未知"}
                ）
              </span>
            </span>
          </Row>
          <Row
            label="ASN IPv4 总量"
            hint="该 ASN 宣告的 IPv4 地址总数（RIPEstat）"
          >
            <span className="truncate font-mono tabular-nums">
              {formatCount(intel?.asnIpv4Count ?? null)}
            </span>
          </Row>
          <Row
            label="预估带宽"
            hint={
              peeringdbDetail(intel) ?? "PeeringDB 未收录，按宣告 IPv4 规模折算"
            }
          >
            <span className="truncate">{dash(bandwidth)}</span>
          </Row>
          <Row label="ASN 注册日期" hint="RDAP 注册事件时间">
            <span className="truncate font-mono">
              {dash(intel?.asnRegistered)}
            </span>
          </Row>
          <Row
            label="企业信息"
            hint={
              intel?.abuseEmail ? `滥用联系：${intel.abuseEmail}` : undefined
            }
          >
            <span className="truncate">
              {dash(intel?.org || intel?.asnName)}
            </span>
          </Row>
        </MiniPanel>
      </div>

      {/* ---------- 三栏：技术指标 / 威胁指标 / 深度检测 ---------- */}
      <div className="mt-2.5 grid gap-2.5 md:grid-cols-3">
        <MiniPanel title="技术指标">
          <Row label="Bogon / 广播" hint="保留、私有或未分配地址">
            <Chip
              tone={
                intel?.isBogon === true
                  ? "r"
                  : intel?.isBogon === false
                    ? "g"
                    : "n"
              }
            >
              {intel?.isBogon === true
                ? "是（保留/未分配）"
                : intel?.isBogon === false
                  ? "否（公网可达）"
                  : "未知"}
            </Chip>
          </Row>
          <Row label="反向 DNS">
            <span
              className="truncate font-mono text-[10.5px]"
              title={intel?.reverseDns ?? undefined}
            >
              {dash(intel?.reverseDns)}
            </span>
          </Row>
          <Row
            label="开放端口"
            hint="仅在点击后探测常见端口，直连单次 1.2 秒超时"
          >
            {!targetIp ? (
              <span className="text-zinc-600">待检测</span>
            ) : openPorts ? (
              <span className="flex flex-wrap justify-end gap-1">
                {openPorts.length ? (
                  openPorts.map((p) => (
                    <span
                      key={p}
                      className="rounded border border-amber-500/25 bg-amber-500/10 px-1.5 py-[1px] font-mono text-[10px] text-amber-400"
                    >
                      {p}
                    </span>
                  ))
                ) : (
                  <Chip tone="g">未发现常见端口</Chip>
                )}
              </span>
            ) : (
              <button
                type="button"
                onClick={() => void scanPorts()}
                disabled={scanBusy}
                className="flex items-center gap-1 rounded-md border border-zinc-700 bg-zinc-800/70 px-2 py-[3px] text-[10.5px] text-zinc-200 hover:border-zinc-600 hover:bg-zinc-700/70 disabled:opacity-50"
              >
                {scanBusy ? (
                  <Loader2 className="h-3 w-3 animate-spin" />
                ) : (
                  <ScanLine className="h-3 w-3" />
                )}
                {scanBusy ? "探测中…" : "检测常见端口"}
              </button>
            )}
          </Row>
          {scanError && (
            <div className="mt-1.5 break-words text-[10px] text-red-400">
              探测失败：{scanError}
            </div>
          )}
          <Row label="所属前缀">
            <span className="truncate font-mono text-[10.5px]">
              {dash(intel?.prefix)}
            </span>
          </Row>
          <Row label="注册机构">
            <span className="truncate">
              {intel?.rir ? `${intel.rir} · ${intel.rirCountry ?? "?"}` : "—"}
            </span>
          </Row>
        </MiniPanel>

        <MiniPanel title="IP 情报（威胁指标）" hint="公开画像库">
          <Row label={threat.label} hint={threat.hint}>
            <Chip tone={threat.tone}>{threat.text}</Chip>
          </Row>
          <Row label="滥用等级" hint={abuseHint}>
            <Chip tone={abuse.tone}>
              {abuse.label}
              {intel?.riskScore != null ? ` · ${intel.riskScore}` : ""}
            </Chip>
          </Row>
          <Row label="滥用黑名单" hint={honey.detail}>
            <Chip tone={honey.tone}>{honey.label}</Chip>
          </Row>
          <Row label="RPKI 状态" hint="RIPEstat 路由起源校验">
            <Chip tone={rpki.tone}>{rpki.label}</Chip>
          </Row>
          <Row label="在地可用性">
            <Chip
              tone={
                countryAllowed === true
                  ? "g"
                  : countryAllowed === false
                    ? "r"
                    : "n"
              }
            >
              {countryAllowed === true
                ? "地区可用"
                : countryAllowed === false
                  ? "受限地区"
                  : done
                    ? "无法确认"
                    : "待检测"}
            </Chip>
          </Row>
        </MiniPanel>

        <MiniPanel title="风险深度检测" hint="本地判定">
          {deep.map((row) => (
            <Row key={row.label} label={row.label} hint={row.hint}>
              <Chip tone={row.tone}>{row.text}</Chip>
            </Row>
          ))}
          <Row label="三角一致性">
            <Chip tone={tri} title={exit?.triangleMsg ?? ""}>
              {tri === "g"
                ? "IP / 时区 / 语言一致"
                : tri === "y"
                  ? "时区与出口不符"
                  : tri === "r"
                    ? "身份矛盾"
                    : "未知"}
            </Chip>
          </Row>
          <Row label="IPv6 泄露">
            <Chip tone={v6} title={exit?.v6Msg ?? ""}>
              {v6 === "g"
                ? "无公开 v6"
                : v6 === "y"
                  ? "v6 可直连"
                  : v6 === "r"
                    ? "v6 已泄漏"
                    : "未知"}
            </Chip>
          </Row>
          {intel?.ipapiisState === "failed" && (
            <div className="mt-2 rounded-md border border-amber-500/25 bg-amber-500/10 px-2 py-1 text-[10px] leading-[15px] text-amber-400">
              ipapi.is 未连通（已配置 Key）—— 本轮只用本地公开源，VPN /
              爬虫旗标可能缺失
            </div>
          )}
        </MiniPanel>
      </div>
    </section>
  );
}
