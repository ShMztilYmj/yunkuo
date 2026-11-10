import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import {
  api,
  type NativeDetect,
  type IpIntel,
  type LastScanSummary,
  type LastScanSystem,
} from "../../services/api";
import { useDiagnosticsStore } from "./state";
import { useAppStore } from "../../store";
import { useDiagnosticsConfiguration } from "./configuration";
import { proxyConfiguration } from "../proxy/configuration";
import { runAllSignals, type SignalResult } from "./signals/web";
import { useEnvironmentVerdict } from "./hooks/useEnvironmentVerdict";
import { runExitLayer } from "./signals/exit";
import {
  runTurnstileTest,
  initialItems,
  type TurnItem,
} from "./signals/turnstile";
import { computeTrust } from "./policies/trust";
import { staleReason, type StaleReason } from "./lastScan";
import { domesticResolverCount } from "./signals/dnsDeep";
import { useDnsDeep } from "./hooks/useDnsDeep";
import DetectHeader from "./components/DetectHeader";
import TrustPanel from "./components/TrustPanel";
import FixPanel from "./components/FixPanel";
import SystemDetectionSection from "./components/SystemDetectionSection";
import ExitChannelsSection from "./components/ExitChannelsSection";
import DnsSection from "./components/DnsSection";
import TurnstileSection from "./components/TurnstileSection";
import FingerprintSection from "./components/FingerprintSection";
import { SYSTEM_SIGNAL_IDS } from "./components/constants";
import { useExitSnapshot } from "./hooks/useExitSnapshot";
import type { FixItem, FixState, TurnstileMounts } from "./components/types";

/** 超时包装：超时返回 onTimeout()，不阻塞整体流程 */
async function withTimeout<T>(
  p: Promise<T>,
  ms: number,
  onTimeout: () => T,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const fallback = new Promise<T>((resolve) => {
    timer = setTimeout(() => resolve(onTimeout()), ms);
  });
  try {
    return await Promise.race([p, fallback]);
  } finally {
    if (timer) clearTimeout(timer);
  }
}

function settleExit(
  settled: PromiseSettledResult<unknown>,
): import("./signals/exit").ExitResult | null {
  if (settled.status !== "fulfilled") return null;
  return (settled.value as import("./signals/exit").ExitResult | null) ?? null;
}

export default function Detect() {
  const config = useDiagnosticsConfiguration();
  const detect = useDiagnosticsStore((s) => s.detect);
  const setDetect = useDiagnosticsStore((s) => s.setDetect);
  // 深度 DNS 测试的唯一状态源：一键检测与卡片按钮都走这里。
  const deep = useDnsDeep();
  const nav = useNavigate();
  const [scanning, setScanning] = useState(false);
  const [scanError, setScanError] = useState<string | null>(null);
  // 上次检测摘要与现场系统事实：只服务展示，不参与启动判定。
  const [history, setHistory] = useState<LastScanSummary | null>(null);
  const [systemNow, setSystemNow] = useState<LastScanSystem | null>(null);
  const [computerBusy, setComputerBusy] = useState(false);
  const [tsItems, setTsItems] = useState<TurnItem[]>(initialItems);
  const [turnRunning, setTurnRunning] = useState(false);
  const turnRunningRef = useRef(false);
  const turnMounts = useRef<TurnstileMounts>({
    managed: null,
    nonInteractive: null,
  });
  // 逐项修复状态：key → "fixing" | "done" | "error"
  const [fixStates, setFixStates] = useState<Record<string, FixState>>({});
  const [fixMsg, setFixMsg] = useState<Record<string, string>>({});
  const [intel, setIntel] = useState<IpIntel | null>(null);
  const [intelBusy, setIntelBusy] = useState(false);
  // 上次出口快照：本轮还没拿到情报时先顶上，避免重启后卡片一片空白。
  const { snapshot: exitSnapshot, remember: rememberExit } = useExitSnapshot();
  const detailRunningRef = useRef(false);

  // 上次检测摘要只在挂载时读一次；读不到或版本不认识就当没有历史。
  useEffect(() => {
    let active = true;
    void api
      .lastScan()
      .then((value) => {
        if (active) setHistory(value);
      })
      .catch(() => {
        /* 历史摘要读不到不影响本轮检测 */
      });
    return () => {
      active = false;
    };
  }, []);

  const runDetection = useCallback(async (options?: { deep?: boolean }) => {
    if (runningRef.current || useDiagnosticsStore.getState().detect.scanning)
      return;
    runningRef.current = true;
    setScanning(true);
    setScanError(null);
    const prev = useDiagnosticsStore.getState().detect;
    setDetect({ ...prev, finished: false, scanning: true });
    try {
      // 新一轮检测先撤销旧评分；只有本轮必要检测完整返回后才允许启动。
      const scanId = await api.beginEnvironmentScan();
      // 浏览器这一路是否走代理：决定出口层能不能拿页面探测的 IP 去比时区。
      const coreStatus = useAppStore.getState().coreStatus;
      const browserProxied = Boolean(
        coreStatus?.running && (coreStatus.tun || coreStatus.system_proxy),
      );
      // 展示层可保留旧数据，启动判定必须使用本轮浏览器及原生检测。
      const settled = await Promise.allSettled([
        withTimeout(
          runAllSignals(),
          20_000,
          () =>
            null as unknown as {
              results: SignalResult[];
              total: number;
            },
        ),
        withTimeout(
          api.detectNative(),
          45_000,
          () => null as unknown as NativeDetect,
        ),
        withTimeout(
          runExitLayer(browserProxied),
          12_000,
          () => null as unknown as import("./signals/exit").ExitResult,
        ),
        // 一键检测同时跑深度 DNS 测试；修单项后的复测不重复消耗 32 次探测。
        options?.deep === false
          ? Promise.resolve(null)
          : withTimeout(
              deep.run(),
              45_000,
              () =>
                null as unknown as import("../../services/api").DnsDeepReport,
            ),
      ]);
      const webRes =
        settled[0].status === "fulfilled"
          ? (settled[0].value as { results: SignalResult[]; total: number })
          : null;
      const native =
        settled[1].status === "fulfilled"
          ? (settled[1].value as NativeDetect)
          : null;
      const exit =
        settled[2].status === "fulfilled" ? settleExit(settled[2]) : null;
      const nativeOk = native != null;
      const complete = webRes != null && nativeOk;
      if (complete) {
        await api.finishEnvironmentScan(scanId, webRes.total);
      } else {
        setScanError("本轮环境检测未完成，启动已暂停。请检查网络后重新检测。");
      }
      void api.uiLog(
        `ui-run: nativeOk=${nativeOk} webOk=${webRes != null} exitOk=${
          exit != null
        } coreRunning=${native?.coreRunning ?? "-"} sysproxy=${native?.sysproxyOn ?? "-"}`,
      );
      setDetect({
        finished: complete,
        scanning: false,
        webResults: webRes?.results ?? prev.webResults,
        webTotal: webRes?.total ?? prev.webTotal,
        native: native ?? prev.native,
        exit: exit ?? prev.exit,
        at: complete ? Date.now() : prev.at,
      });
      void runDetailLayers();
    } catch (e) {
      console.error(e);
      void api.uiLog(`ui-run unhandled: ${e}`);
      setScanError("环境检测失败，请重新检测后再启动。");
      setDetect({
        ...useDiagnosticsStore.getState().detect,
        finished: false,
        scanning: false,
      });
    } finally {
      runningRef.current = false;
      setScanning(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const runningRef = useRef(false);

  // 详细展示层：出口 IP 情报（快照落盘）
  async function runDetailLayers() {
    if (detailRunningRef.current) return;
    detailRunningRef.current = true;
    setIntelBusy(true);
    try {
      const detectNow = useDiagnosticsStore.getState().detect;
      const exitIp = detectNow.native?.exitIp ?? null;
      const data = await withTimeout<IpIntel | null>(
        api.ipIntel(exitIp),
        20_000,
        () => null,
      );
      setIntel(data ?? null);
      // 只在本轮真正拿到情报时覆盖快照：一次失败不该抹掉上次的好数据。
      rememberExit(
        detectNow.exit,
        data ?? null,
        detectNow.native?.countryAllowed ?? null,
      );
    } finally {
      setIntelBusy(false);
      detailRunningRef.current = false;
    }
  }

  // 手动模式下不做自动检测：只有点击「一键检测并修复」才会跑（内核事件也不触发）

  // Cloudflare Turnstile 双模式验证（托管/非交互）
  const runTurn = useCallback(async () => {
    if (turnRunningRef.current) return;
    const m = turnMounts.current;
    if (!m.managed || !m.nonInteractive) return;
    turnRunningRef.current = true;
    setTurnRunning(true);
    try {
      const items = await runTurnstileTest(m, (it) => setTsItems([...it]));
      setTsItems([...items]);
    } catch {
      /* 保持最近一次状态 */
    } finally {
      turnRunningRef.current = false;
      setTurnRunning(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 每次检测完成后自动跑一轮验证测试
  useEffect(() => {
    if (detect.finished && detect.at && !turnRunningRef.current) {
      const t = setTimeout(() => void runTurn(), 400);
      return () => clearTimeout(t);
    }
  }, [detect.finished, detect.at, runTurn]);

  const nd = detect.native;
  const done = detect.finished && nd != null;
  const systemSignals = detect.webResults.filter((signal) =>
    SYSTEM_SIGNAL_IDS.has(signal.id),
  );
  const trust = computeTrust({
    webTotal: detect.webTotal,
    native: nd,
    exit: detect.exit,
    dnsDeep: deep.report,
    turnstile: tsItems,
  });
  const { pass } = useEnvironmentVerdict();

  // 修复一个单项：执行动作 → 马上重测该项快检 → 全量重测刷新
  const fixOne = async (f: FixItem) => {
    if (!f.run || !f.key) return;
    setFixStates((s) => ({ ...s, [f.key]: "fixing" }));
    setFixMsg((m) => ({ ...m, [f.key]: "" }));
    try {
      await f.run();
      // 全量重测刷新状态（不重跑深度 DNS：换节点后由用户主动重测）
      await runDetection({ deep: false });
      const fresh = useDiagnosticsStore.getState().detect.native;
      const ok = fresh && f.quickCheck ? f.quickCheck(fresh) : true;
      setFixStates((s) => ({ ...s, [f.key]: ok ? "done" : "error" }));
      if (!ok)
        setFixMsg((m) => ({
          ...m,
          [f.key]: "动作已执行，但该项仍显示异常 —— 可能需要等待几秒或人工处理",
        }));
    } catch (e) {
      setFixStates((s) => ({ ...s, [f.key]: "error" }));
      setFixMsg((m) => ({ ...m, [f.key]: String(e) }));
      // 失败后也重测，让 UI 回到真实状态
      await runDetection({ deep: false });
    }
  };

  // ---------- 修复清单（逐项生成，每项独立修复按钮）----------
  const fixes: FixItem[] = [];
  if (nd) {
    if (!nd.coreRunning)
      fixes.push({
        key: "core",
        text: "代理内核未运行",
        actionLabel: "启动内核",
        run: async () => {
          await api.coreStart();
        },
        quickCheck: (n) => n.coreRunning,
      });
    if (!nd.tunOn && !nd.sysproxyOn)
      fixes.push({
        key: "sysproxy",
        text: "系统代理未开启（或 TUN 未开启）",
        actionLabel: "开系统代理",
        run: async () => {
          await proxyConfiguration.setSystemProxy(true);
        },
        quickCheck: (n) => n.sysproxyOn || n.tunOn,
      });
    if (nd.countryAllowed === false)
      fixes.push({
        key: "country",
        text: `出口 IP 在受限地区 ${nd.exitCountry ?? ""}`,
        actionLabel: "去换节点",
        navigateTo: "/proxies",
        manualHint: "在「代理 → 节点」页选择允许地区的节点后，回到此页复测",
      });
    if (nd.countryAllowed == null)
      fixes.push({
        key: "exitip",
        text: "无法获取出口 IP（代理可能未生效）",
        actionLabel: "开系统代理并复测",
        run: async () => {
          await proxyConfiguration.setSystemProxy(true);
          await new Promise((r) => setTimeout(r, 1500));
        },
        quickCheck: (n) => n.exitIp != null,
        manualHint: "若仍失败，请检查订阅与节点（代理页 → 测速）",
      });
    if (
      (config?.require_claude_reachable ?? true) &&
      nd.claudeReachable !== true
    )
      fixes.push({
        key: "claude-connectivity",
        text: "恢复 Claude 连通性",
        actionLabel: "去换节点",
        navigateTo: "/proxies",
        manualHint:
          "选择可连接 claude.ai 的节点后重新检测；可在设置中关闭连通性要求",
      });
    // 只把「确实对不上」的黄/红算成待复核；n 表示探测线路不同源，已在
    // 「系统代理未开启」那条提示里体现，不重复要求用户处理。
    if (
      nd.tzIsCn ||
      nd.timezoneMatches !== true ||
      detect.exit?.triangleVerdict === "y" ||
      detect.exit?.triangleVerdict === "r"
    )
      fixes.push({
        key: "tz",
        text: "语言、时区与出口的一致性需要复核",
        targetId: "system-timezone",
        manualHint:
          "在「时区合规」中查看详细结果，按需调整地区或同步出口时区。",
      });
    if (detect.exit?.v6Verdict === "r" || detect.exit?.v6Verdict === "y")
      fixes.push(
        config?.network_mode
          ? {
              key: "v6",
              text: "处理 IPv6 暴露风险",
              manualHint:
                "网络方案已在代理内核关闭 IPv6；剩下的 v6 暴露来自网卡直连，请在系统网络设置里关闭 IPv6，或改用「AI 严格保护」。",
            }
          : {
              key: "v6",
              text: "处理 IPv6 暴露风险",
              actionLabel: "关闭 IPv6（内核级）",
              run: async () => {
                await proxyConfiguration.patch({ ipv6: false });
                await new Promise((r) => setTimeout(r, 800));
              },
              manualHint: "关闭 IPv6 后内核自动重启，稍后复测",
            },
      );
    const deepDomestic = domesticResolverCount(deep.report);
    if (deepDomestic > 0)
      fixes.push({
        key: "dns",
        text: `DNS 深度测试发现 ${deepDomestic} 个境内解析器`,
        actionLabel: "去换节点",
        navigateTo: "/proxies",
        manualHint:
          "深度测试经当前浏览器路径发起；换到由境外解析的节点后，回到此页重新检测。",
      });
    if (trust.hits.length && !pass)
      fixes.push({
        key: "fingerprint",
        text: "浏览器语言/中文字体/国产软件指纹无法自动修改",
        manualHint:
          "建议将系统显示语言与浏览器首选语言调整为 en-US，必要时卸载国产输入法/软件后复测",
      });
    // Claude 连通性仅作展示参考，不列入必须修复项
  }

  // 本轮完成、拿到系统事实后落盘一次摘要；只写展示字段，不写门控凭据。
  const savedScanRef = useRef<number | null>(null);
  useEffect(() => {
    if (!done || !detect.at || !nd) return;
    if (savedScanRef.current === detect.at) return;
    savedScanRef.current = detect.at;
    void api
      .saveLastScan({
        score: trust.score,
        failures: fixes.map((item) => item.text),
        system: {
          timezone: nd.currentTz,
          languages: systemNow?.languages ?? [],
          culture: systemNow?.culture ?? "",
        },
      })
      .then(() => api.lastScan())
      .then((value) => setHistory(value))
      .catch(() => {
        /* 历史摘要属于展示缓存，写失败不影响本轮结论 */
      });
  }, [done, detect.at, nd, trust.score, fixes, systemNow]);

  const stale: StaleReason = history ? staleReason(history, systemNow) : null;

  return (
    <div className="detect-page flex flex-col">
      <DetectHeader
        at={detect.at}
        history={history}
        stale={stale}
        scanning={scanning}
        detectScanning={detect.scanning}
        computerBusy={computerBusy}
        scanError={scanError}
        pass={pass}
        failures={fixes.length}
        signals={detect.webResults.length}
        score={detect.at ? trust.score : (history?.score ?? null)}
        onRun={() => void runDetection()}
      />

      {/* 信任评分 + 修复清单 */}
      <div className="detect-summary grid grid-cols-1 gap-4 md:grid-cols-[280px_1fr]">
        <TrustPanel
          done={done}
          trust={trust}
          previous={history?.score ?? null}
        />

        <FixPanel
          done={done}
          fixes={fixes}
          fixStates={fixStates}
          fixMsg={fixMsg}
          pass={pass}
          onFix={fixOne}
          navigate={nav}
          previous={
            history ? { at: history.at, failures: history.failures } : null
          }
        />
      </div>

      <SystemDetectionSection
        native={done ? nd : null}
        exit={done ? detect.exit : null}
        signals={done ? systemSignals : []}
        requireClaudeReachable={config?.require_claude_reachable ?? true}
        refreshKey={detect.at}
        disabled={scanning || detect.scanning}
        onBusyChange={setComputerBusy}
        onSystem={setSystemNow}
      />

      <ExitChannelsSection
        done={done}
        native={nd}
        exit={detect.exit}
        cached={exitSnapshot}
        intel={intel}
        intelBusy={intelBusy}
      />

      <DnsSection
        done={done}
        deep={deep}
        exitIp={nd?.exitIp ?? null}
        exitCountryName={nd?.exitCountryName ?? null}
      />

      <TurnstileSection
        items={tsItems}
        running={turnRunning}
        onRun={runTurn}
        mounts={turnMounts}
      />

      <FingerprintSection />
    </div>
  );
}
