import {
  api,
  type DnsDeepChallenge,
  type DnsDeepReport,
} from "../../../services/api";

export interface DnsDeepProgress {
  phase: "preparing" | "probing" | "collecting";
  round: number;
  /** 本轮计划轮数：会话建不起来时会少于 DNS_DEEP_ROUNDS。 */
  rounds: number;
  attempted: number;
  total: number;
}

export type DnsDeepTone = "idle" | "good" | "bad";

export interface DnsDeepVerdict {
  tone: DnsDeepTone;
  /** 一句话结论，标题徽章与结论横幅共用 */
  label: string;
}

/** 境内解析器数量：徽章与横幅都从这里取，避免两处判定漂移。 */
export function domesticResolverCount(
  report: DnsDeepReport | null | undefined,
): number {
  return (report?.resolvers ?? []).filter((row) => row.cc === "CN").length;
}

/** 与结论横幅同一套判定：出现境内解析器才算红，拿不到解析器不算通过。 */
export function dnsDeepVerdict(
  report: DnsDeepReport | null | undefined,
): DnsDeepVerdict {
  if (!report) return { tone: "idle", label: "未测试" };
  const domestic = domesticResolverCount(report);
  if (domestic > 0)
    return { tone: "bad", label: `发现 ${domestic} 个境内解析器，可能泄露` };
  if (report.resolvers.length > 0)
    return { tone: "good", label: "未检测到 DNS 泄露" };
  return { tone: "idle", label: "未取得解析器，不能判定为通过" };
}

/** 权威观察站每次会话发 12 个一次性域名（6 个 A + 6 个 AAAA），3 轮共 36 个。 */
export const DNS_DEEP_ROUNDS = 3;
export const DNS_DEEP_PER_ROUND = 12;
export const DNS_DEEP_TOTAL = DNS_DEEP_ROUNDS * DNS_DEEP_PER_ROUND;

function current(signal: AbortSignal) {
  if (signal.aborted) throw new DOMException("检测已取消", "AbortError");
}
export function validateDnsChallenge(plan: DnsDeepChallenge) {
  const total = plan.rounds * plan.perRound;
  if (
    !plan.id ||
    plan.rounds < 1 ||
    plan.rounds > DNS_DEEP_ROUNDS ||
    plan.perRound < 1 ||
    plan.perRound > DNS_DEEP_PER_ROUND ||
    total !== plan.hostnames.length ||
    new Set(plan.hostnames).size !== total ||
    plan.hostnames.some(
      (host) => !/^[a-z0-9]{6,64}-[aq]\d{2}\.t\.dnsleak\.dev$/.test(host),
    )
  ) {
    throw Error("DNS 测试域名不符合预期，已停止探测");
  }
}
function wait(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    current(signal);
    const cancel = () => {
      clearTimeout(timer);
      reject(new DOMException("检测已取消", "AbortError"));
    };
    const timer = setTimeout(() => {
      signal.removeEventListener("abort", cancel);
      resolve();
    }, ms);
    signal.addEventListener("abort", cancel, { once: true });
  });
}
function probe(host: string, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    current(signal);
    const image = new Image();
    const finish = () => {
      cleanup();
      resolve();
    };
    const cancel = () => {
      cleanup();
      reject(new DOMException("检测已取消", "AbortError"));
    };
    const cleanup = () => {
      clearTimeout(timer);
      image.onload = image.onerror = null;
      image.removeAttribute("src");
      signal.removeEventListener("abort", cancel);
    };
    const timer = setTimeout(finish, 2500);
    image.onload = image.onerror = finish;
    signal.addEventListener("abort", cancel, { once: true });
    // HTTPS 成败不能证明 DNS 是否到达；只认后端读取的权威观察结果。
    image.src = `https://${host}/pixel.gif`;
  });
}

export async function runDnsDeep(
  signal: AbortSignal,
  progress: (value: DnsDeepProgress) => void,
): Promise<DnsDeepReport> {
  let id: string | null = null;
  try {
    current(signal);
    progress({
      phase: "preparing",
      round: 0,
      rounds: DNS_DEEP_ROUNDS,
      attempted: 0,
      total: DNS_DEEP_TOTAL,
    });
    const plan = await api.dnsDeepStart();
    id = plan.id;
    current(signal);
    validateDnsChallenge(plan);
    for (let round = 0; round < plan.rounds; round++) {
      current(signal);
      progress({
        phase: "probing",
        round: round + 1,
        rounds: plan.rounds,
        attempted: round * plan.perRound,
        total: plan.hostnames.length,
      });
      await Promise.all(
        plan.hostnames
          .slice(round * plan.perRound, (round + 1) * plan.perRound)
          .map((host) => probe(host, signal)),
      );
      current(signal);
      progress({
        phase: "probing",
        round: round + 1,
        rounds: plan.rounds,
        attempted: (round + 1) * plan.perRound,
        total: plan.hostnames.length,
      });
      if (round + 1 < plan.rounds) await wait(600, signal);
    }
    progress({
      phase: "collecting",
      round: plan.rounds,
      rounds: plan.rounds,
      attempted: plan.hostnames.length,
      total: plan.hostnames.length,
    });
    await wait(2000, signal);
    let report: DnsDeepReport | null = null;
    // 空结果可能是异步观察尚未完成；有限重试，不能把空结果标为安全。
    for (let attempt = 0; attempt < 3; attempt++) {
      current(signal);
      report = await api.dnsDeepCollect(plan.id);
      current(signal);
      if (report.resolvers.length > 0 || report.filtered > 0) break;
      if (attempt < 2) await wait(2000, signal);
    }
    return report!;
  } finally {
    if (id) void api.dnsDeepCancel(id).catch(() => {});
  }
}
