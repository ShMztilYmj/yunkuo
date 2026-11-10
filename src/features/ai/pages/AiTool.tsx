import { invalidateAiUsage, readAiUsage } from "../services/aiUsage";
import { useCallback, useEffect, useRef, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api, type AiUsageItem } from "../../../services/api";
import { useAppStore } from "../../../store";
import { aiConfiguration } from "../configuration";
import { usePolling } from "../../../shared/hooks/usePolling";
import { useSearchParams } from "react-router-dom";
import type { AgentBotState } from "../services/agentBots";
import {
  AI_TOOL_CATALOG,
  DEFAULT_AI_CARDS,
  aiToolCards,
  shortcutPatch,
  toolShortcut,
  type AiToolMeta,
} from "../services/aiToolCards";
import { useToolSetup } from "../hooks/useToolSetup";
import {
  toolSetup,
  type ToolConnectivity,
  type ToolInspection,
} from "../services/toolSetup";
import AiToolHeader from "../components/tools/AiToolHeader";
import GateWarning from "../components/tools/GateWarning";
import ToolCard from "../components/tools/ToolCard";
import ToolPicker from "../components/tools/ToolPicker";
import { CARD_MAKERS } from "../components/tools/constants";

export interface AiGateView {
  /** 启动条件是否全部满足；判定在检测模块完成，AI 页面只消费结果。 */
  pass: boolean;
  failures: string[];
  scanned: boolean;
}

export default function AiTool({ gate }: { gate: AiGateView }) {
  const [params] = useSearchParams();
  /** 环境未全绿时的启动确认框：确认后仍会启动，只有系统代理与 TUN 是硬性要求。 */
  const [launchGate, setLaunchGate] = useState<AiToolMeta | null>(null);
  const [accountTool, setAccountTool] = useState(() =>
    params.get("view") === "accounts" ? params.get("tool") : null,
  );
  const [openedAccounts, setOpenedAccounts] = useState<Record<string, boolean>>(
    {},
  );
  const [resumeOpen, setResumeOpen] = useState<string | null>(null);
  const [resumeMounted, setResumeMounted] = useState<Record<string, boolean>>(
    {},
  );
  const [resumeBusy, setResumeBusy] = useState<Record<string, boolean>>({});
  /** 深链 view=accounts 跟随地址栏；页面保持挂载时也要重新定位到目标卡片。 */
  useEffect(() => {
    if (params.get("view") !== "accounts") return;
    const tool = params.get("tool");
    setAccountTool(tool === "claude" || tool === "chatgpt" ? tool : "claude");
  }, [params]);
  useEffect(() => {
    if (accountTool !== "claude" && accountTool !== "chatgpt") return;
    setOpenedAccounts((value) => ({ ...value, [accountTool]: true }));
    document
      .querySelector(`[data-tool-card="${accountTool}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [accountTool]);
  const openAccounts = (tool: string) => {
    if (resumeOpen === tool) setResumeOpen(null);
    setOpenedAccounts((value) => ({ ...value, [tool]: true }));
    setAccountTool(tool);
  };
  const config = useAppStore((s) => s.config);
  const [launchingKey, setLaunchingKey] = useState<string | null>(null);
  const [accountBusy, setAccountBusy] = useState<Record<string, boolean>>({});
  const [launchStates, setLaunchStates] = useState<
    Record<string, AgentBotState>
  >({});
  const [msgs, setMsgs] = useState<Record<string, string | null>>({});
  const [details, setDetails] = useState<Record<string, boolean>>({});
  const setup = useToolSetup();
  const [inspections, setInspections] = useState<
    Record<string, ToolInspection>
  >({});
  const [connectionTests, setConnectionTests] = useState<
    Record<string, ToolConnectivity>
  >({});
  const [setupErrors, setSetupErrors] = useState<Record<string, string>>({});
  const [setupBusy, setSetupBusy] = useState<Record<string, string>>({});
  const checkingTools = useRef(new Set<string>());
  const cards = aiToolCards(config);
  const availableTools = AI_TOOL_CATALOG.filter(
    (tool) => !cards.some((card) => card.key === tool.key),
  );
  const [adding, setAdding] = useState(false);
  const [cardBusy, setCardBusy] = useState(false);
  const cardSaving = useRef(false);
  const [cardError, setCardError] = useState("");
  const updateCards = async (keys: string[]) => {
    if (cardSaving.current || !config) return;
    cardSaving.current = true;
    setCardBusy(true);
    setCardError("");
    try {
      await aiConfiguration.patch({ ai_tool_cards: keys });
    } catch (error) {
      setCardError(`卡片保存失败：${String(error)}`);
    } finally {
      cardSaving.current = false;
      setCardBusy(false);
    }
  };

  // ----- 用量 / 套餐 / 账号（token-monitor 风格，本地数据优先）-----
  const [usage, setUsage] = useState<AiUsageItem[] | null>(null);
  const usageLoading = useRef(0);
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const [usageError, setUsageError] = useState<string | null>(null);
  const [usageRefreshing, setUsageRefreshing] = useState(false);

  const loadUsage = useCallback(
    async (isCurrent: () => boolean = () => true, force = true) => {
      usageLoading.current++;
      setUsageRefreshing(true);
      try {
        const snap = await readAiUsage(force);
        if (!mounted.current || !isCurrent()) return;
        setUsage(snap.items);
        setUsageError(null);
      } catch (error) {
        if (mounted.current && isCurrent())
          setUsageError(`读取失败：${String(error)}`);
      } finally {
        usageLoading.current--;
        if (mounted.current && usageLoading.current === 0)
          setUsageRefreshing(false);
      }
    },
    [],
  );

  usePolling(
    (current) => loadUsage(current, false),
    60_000,
    true,
    !!config,
    cards.map((card) => card.key).join(","),
  );

  const { pass, failures, scanned } = gate;

  const setMsg = (k: string, text: string | null) =>
    setMsgs((m) => ({ ...m, [k]: text }));

  const saveShortcut = async (meta: AiToolMeta, path: string) => {
    await aiConfiguration.patch(
      shortcutPatch(useAppStore.getState().config, meta.key, path),
    );
    setMsg(meta.key, `${meta.label} 启动方式已更新`);
    setup.refresh();
  };
  const inspect = async (meta: AiToolMeta, action: "inspect" | "test") => {
    if (checkingTools.current.has(meta.key)) return;
    checkingTools.current.add(meta.key);
    setSetupBusy((value) => ({ ...value, [meta.key]: action }));
    setSetupErrors((value) => ({ ...value, [meta.key]: "" }));
    setDetails((value) => ({ ...value, [meta.key]: true }));
    if (accountTool === meta.key) setAccountTool(null);
    try {
      if (action === "inspect") {
        const value = await toolSetup.inspect(meta.key);
        setInspections((values) => ({ ...values, [meta.key]: value }));
        setup.refresh();
      } else {
        const value = await toolSetup.test(meta.key);
        setConnectionTests((values) => ({ ...values, [meta.key]: value }));
      }
    } catch (reason) {
      setSetupErrors((value) => ({ ...value, [meta.key]: String(reason) }));
    } finally {
      checkingTools.current.delete(meta.key);
      setSetupBusy((value) => ({ ...value, [meta.key]: "" }));
    }
  };

  /** 自动定位：后端按安装目录挑选可执行入口，这里只负责保存与反馈。 */
  const autoDetect = async (meta: AiToolMeta) => {
    if (checkingTools.current.has(meta.key)) return;
    checkingTools.current.add(meta.key);
    setSetupBusy((value) => ({ ...value, [meta.key]: "autodetect" }));
    setSetupErrors((value) => ({ ...value, [meta.key]: "" }));
    setMsg(meta.key, null);
    try {
      const found = await toolSetup.autodetect(meta.key);
      setDetails((value) => ({ ...value, [meta.key]: true }));
      if (!found.path) {
        setSetupErrors((value) => ({ ...value, [meta.key]: found.note }));
        setMsg(meta.key, "未找到可用的安装入口，请手动选择快捷方式");
        return;
      }
      const current = toolShortcut(useAppStore.getState().config, meta.key);
      if (current === found.path) {
        setMsg(meta.key, "已通过：当前启动入口有效，无需更换");
        return;
      }
      await saveShortcut(meta, found.path);
      setMsg(meta.key, "启动方式已更新：自动定位成功");
    } catch (reason) {
      setSetupErrors((value) => ({ ...value, [meta.key]: String(reason) }));
      setMsg(meta.key, `自动检测失败：${String(reason)}`);
    } finally {
      checkingTools.current.delete(meta.key);
      setSetupBusy((value) => ({ ...value, [meta.key]: "" }));
    }
  };

  const pick = async (meta: AiToolMeta) => {
    try {
      const path = await openDialog({
        multiple: false,
        filters: [
          { name: "启动方式", extensions: ["lnk", "exe", "bat", "cmd"] },
        ],
      });
      if (typeof path === "string") {
        await saveShortcut(meta, path);
      }
    } catch (error) {
      setMsg(meta.key, `启动方式保存失败：${String(error)}`);
    }
  };

  const launch = async (meta: AiToolMeta, confirmed = false) => {
    setLaunchingKey(meta.key);
    setLaunchStates((states) => ({ ...states, [meta.key]: "working" }));
    setMsg(meta.key, null);
    try {
      const r = await api.launchAi(meta.key, confirmed);
      if (r.ok) {
        setLaunchStates((states) => ({ ...states, [meta.key]: "idle" }));
        setMsg(meta.key, `${meta.label} 已通过所选运行方案启动`);
        setup.refresh();
      } else {
        setLaunchStates((states) => ({ ...states, [meta.key]: "asking" }));
        if (accountTool === meta.key) setAccountTool(null);
        setMsg(meta.key, `未通过环境校验：\n${r.failures.join("\n")}`);
        setDetails((value) => ({ ...value, [meta.key]: true }));
        setSetupErrors((value) => ({
          ...value,
          [meta.key]: r.failures.join("；"),
        }));
      }
    } catch (e) {
      setLaunchStates((states) => ({ ...states, [meta.key]: "asking" }));
      if (accountTool === meta.key) setAccountTool(null);
      setMsg(meta.key, String(e));
      setDetails((value) => ({ ...value, [meta.key]: true }));
      setSetupErrors((value) => ({ ...value, [meta.key]: String(e) }));
    } finally {
      setLaunchingKey(null);
    }
  };

  /** 启动入口：环境未全绿时先弹确认框，确认后仍然启动。 */
  const requestLaunch = (meta: AiToolMeta) => {
    if (!scanned) {
      setMsg(meta.key, "请先完成环境检测，再启动");
      return;
    }
    if (pass) {
      void launch(meta);
      return;
    }
    setLaunchGate(meta);
  };

  return (
    <div className="ai-tools-page">
      <AiToolHeader
        adding={adding}
        canAdd={!!config && !cardBusy && availableTools.length > 0}
        count={cards.length}
        scanned={scanned}
        pass={pass}
        onToggleAdd={() => setAdding((value) => !value)}
      />

      {cardError && (
        <p
          role="alert"
          className="rounded-xl border border-red-800 bg-red-950/30 p-3 text-xs text-red-300"
        >
          {cardError}
        </p>
      )}
      {adding && (
        <ToolPicker
          tools={availableTools}
          busy={cardBusy}
          onAdd={(key) =>
            void updateCards([...cards.map((card) => card.key), key])
          }
          onClose={() => setAdding(false)}
        />
      )}

      {scanned && !pass && <GateWarning failures={failures} />}

      {/* 默认两张，其余工具由用户选择。 */}
      <div className="ai-tool-deck" aria-label="工具卡组">
        {cards.map((meta) => {
          const shortcut = toolShortcut(config, meta.key);
          const accountWorking =
            (accountBusy[meta.key] ?? false) || (resumeBusy[meta.key] ?? false);
          const entry = setup.snapshot?.tools.find(
            (tool) => tool.key === meta.key,
          );
          const configured = shortcut.length > 0;
          const canLaunch =
            scanned &&
            configured &&
            !accountWorking &&
            entry?.shortcutValid !== false;
          const busy = launchingKey === meta.key;
          const item = usage?.find((item) => item.key === meta.key);
          const supportsAccounts =
            meta.key === "claude" || meta.key === "chatgpt";
          const showAccounts = supportsAccounts && accountTool === meta.key;
          const showResume = resumeOpen === meta.key;
          const showDetails =
            showResume || showAccounts || (details[meta.key] ?? false);
          const usagePending = usage === null && usageError === null;
          return (
            <ToolCard
              key={meta.key}
              meta={meta}
              maker={CARD_MAKERS[meta.key]}
              view={{
                shortcut,
                configured,
                canLaunch,
                busy,
                accountWorking,
                supportsAccounts,
                showAccounts,
                showResume,
                showDetails,
              }}
              data={{
                entry,
                item,
                launchState: launchStates[meta.key],
                inspection: inspections[meta.key],
                connectionTest: connectionTests[meta.key],
                setupBusy: setupBusy[meta.key],
                setupError: setupErrors[meta.key] || setup.error,
                setupFailure: setup.error,
                message: msgs[meta.key],
                resumeMounted: resumeMounted[meta.key] ?? false,
                accountPanelOpened: openedAccounts[meta.key] ?? false,
                usageLoading: usagePending,
                usageError,
                usageRefreshing,
                scanned,
                pass,
                launching: launchingKey != null,
                cardBusy,
                configReady: !!config,
                isDefaultCard: DEFAULT_AI_CARDS.includes(meta.key),
              }}
              actions={{
                onToggleResume: () => {
                  setResumeMounted((v) => ({ ...v, [meta.key]: true }));
                  setResumeOpen(meta.key);
                  if (accountTool === meta.key) setAccountTool(null);
                },
                onOpenAccounts: () => openAccounts(meta.key),
                onToggleDetails: () => {
                  if (showResume) setResumeOpen(null);
                  if (showAccounts) setAccountTool(null);
                  setDetails((value) => ({
                    ...value,
                    [meta.key]: !showDetails,
                  }));
                },
                onRemove: () =>
                  void updateCards(
                    cards
                      .filter((card) => card.key !== meta.key)
                      .map((card) => card.key),
                  ),
                onInspect: () => void inspect(meta, "inspect"),
                onTest: () => void inspect(meta, "test"),
                onUseShortcut: (path) =>
                  void saveShortcut(meta, path).catch((reason) =>
                    setSetupErrors((value) => ({
                      ...value,
                      [meta.key]: `入口保存失败：${String(reason)}`,
                    })),
                  ),
                onPickShortcut: () => void pick(meta),
                onAutoDetectShortcut: () => void autoDetect(meta),
                onRefreshUsage: () => void loadUsage(),
                onLaunch: () => requestLaunch(meta),
                onAccountBusyChange: (next) =>
                  setAccountBusy((value) => ({ ...value, [meta.key]: next })),
                onResumeBusyChange: (next) =>
                  setResumeBusy((value) => ({ ...value, [meta.key]: next })),
                onCodexAccountsChanged: () => void loadUsage(undefined, false),
                onClaudeIdentityChanged: () => {
                  invalidateAiUsage();
                  void loadUsage();
                },
              }}
            />
          );
        })}
      </div>

      {launchGate && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6">
          <div
            role="dialog"
            aria-modal="true"
            aria-label="启动确认"
            className="w-full max-w-xl rounded-2xl border border-zinc-800 bg-zinc-950 p-5 shadow-2xl"
          >
            <GateWarning failures={failures} />
            <p className="mt-3 text-[11px] leading-relaxed text-zinc-400">
              系统代理和 TUN
              是硬性要求，缺任意一项都无法启动；其余项确认后可以继续。
            </p>
            <div className="mt-4 flex justify-end gap-2">
              <button
                type="button"
                className="rounded-lg px-3 py-1.5 text-xs text-zinc-300 hover:bg-zinc-800"
                onClick={() => setLaunchGate(null)}
              >
                取消
              </button>
              <button
                type="button"
                className="rounded-lg bg-amber-500 px-3 py-1.5 text-xs font-medium text-zinc-950 hover:bg-amber-400"
                onClick={() => {
                  const meta = launchGate;
                  setLaunchGate(null);
                  void launch(meta, true);
                }}
              >
                仍然启动
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
