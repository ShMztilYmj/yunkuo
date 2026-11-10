import { useCallback, useEffect, useRef, useState } from "react";
import {
  CheckCircle2,
  CircleDashed,
  Monitor,
  RefreshCw,
  RotateCcw,
  XCircle,
} from "lucide-react";
import { api, type NativeDetect } from "../../../services/api";
import {
  computerEnvironment as environment,
  invalidateEnvironment,
  type ComputerStatus,
} from "../services/environment";
import type { ExitResult, Verdict } from "../signals/exit";
import type { SignalResult } from "../signals/web";
import DetectionFinding from "./DetectionFinding";
import ConfirmDialog from "../../../shared/ui/ConfirmDialog";

const button =
  "rounded-lg border border-zinc-700 bg-zinc-800 px-3 py-2 text-xs text-zinc-200 hover:bg-zinc-700 disabled:cursor-not-allowed disabled:opacity-40";

export default function SystemDetectionPanel({
  native,
  exit,
  signals,
  requireClaudeReachable,
  refreshKey,
  disabled = false,
  onBusyChange,
  onSystem,
}: {
  native: NativeDetect | null;
  exit: ExitResult | null;
  signals: SignalResult[];
  requireClaudeReachable: boolean;
  refreshKey: number | null;
  disabled?: boolean;
  onBusyChange: (busy: boolean) => void;
  /** 把现场读到的系统事实交给页面，仅用于判断「上次结论是否还成立」 */
  onSystem?: (current: ComputerStatus["current"]) => void;
}) {
  const [state, setState] = useState<ComputerStatus | null>(null);
  const [region, setRegion] = useState("US");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [pending, setPending] = useState<{ restore: boolean } | null>(null);
  const [applying, setApplying] = useState(false);
  const [applyError, setApplyError] = useState<string | null>(null);
  const running = useRef(false);
  // 回调每次渲染都会换新身份，用 ref 持有，避免把加载 effect 拖成循环。
  const onSystemRef = useRef(onSystem);
  onSystemRef.current = onSystem;
  const accept = useCallback((value: ComputerStatus) => {
    setState(value);
    onSystemRef.current?.(value.current);
  }, []);
  const load = useCallback(
    async () => accept(await environment.computer()),
    [accept],
  );

  useEffect(() => {
    if (disabled) return;
    let active = true;
    environment
      .computer()
      .then((value) => {
        if (active) accept(value);
      })
      .catch((error) => {
        if (active) setMessage(String(error));
      });
    return () => {
      active = false;
    };
  }, [refreshKey, disabled, accept]);

  const run = async (action: () => Promise<void>) => {
    if (running.current || disabled) return;
    running.current = true;
    setBusy(true);
    onBusyChange(true);
    setMessage(null);
    try {
      await action();
    } catch (error) {
      setMessage(String(error));
    } finally {
      running.current = false;
      setBusy(false);
      onBusyChange(false);
    }
  };

  /** 应用/恢复都要先过一道应用内确认框，不再弹系统 TaskDialog。 */
  const apply = (restore = false) => {
    if (running.current || disabled) return;
    setApplyError(null);
    setPending({ restore });
  };

  const confirmApply = async () => {
    const target = pending;
    if (!target || applying) return;
    setApplying(true);
    setApplyError(null);
    running.current = true;
    setBusy(true);
    onBusyChange(true);
    setMessage(null);
    try {
      invalidateEnvironment();
      const result = await environment.applyComputer(region, target.restore);
      await api.recordWebScan(null);
      await load();
      setMessage(result.message);
      setPending(null);
    } catch (error) {
      setApplyError(String(error));
    } finally {
      running.current = false;
      setBusy(false);
      onBusyChange(false);
      setApplying(false);
    }
  };

  const syncTimezone = () =>
    run(async () => {
      invalidateEnvironment();
      const result = await api.syncTimezone();
      await api.recordWebScan(null);
      await load();
      setMessage(result.message);
    });
  const unread = message ? "读取失败" : "读取中";
  const signalDetails = (ids: SignalResult["id"][]) =>
    signals
      .filter((signal) => ids.includes(signal.id))
      .map((signal) => (
        <div
          key={signal.id}
          className={`mt-1 break-words text-[11px] ${signal.contribution > 0 ? "text-amber-400" : "text-zinc-500"}`}
        >
          {signal.name}：{signal.raw}
          {signal.contribution > 0 ? ` · 风险 +${signal.contribution}` : ""}
        </div>
      ));
  const timezoneChanged = !!(
    native &&
    state &&
    native.currentTz !== state.current.timezone
  );
  const timezoneVerdict: Verdict =
    !native || timezoneChanged
      ? "n"
      : native.tzIsCn || native.timezoneMatches === false
        ? "r"
        : native.timezoneMatches !== true
          ? "n"
          : (exit?.triangleVerdict ?? "g");
  const timezoneMessage = !native
    ? "完成检测后核对语言、系统时区与出口地区。"
    : timezoneChanged
      ? "系统时区与上次结果不同，请重新检测。"
      : [
          native.timezoneMatches !== true || native.tzIsCn
            ? "系统时区与出口尚未对齐。"
            : "",
          exit?.triangleMsg ??
            (native.timezoneMatches === true
              ? "系统时区与出口一致。"
              : "出口时区暂时无法确认。"),
        ]
          .filter(Boolean)
          .join(" ");
  const current = state?.current ?? null;
  const preset = state?.presets.find((item) => item.id === region) ?? null;

  return (
    <div className="space-y-3">
      <div className="grid grid-cols-1 gap-2 text-xs sm:grid-cols-2 md:grid-cols-3">
        <SystemItem
          label="内核运行"
          status={native?.coreRunning}
          value={
            native
              ? native.coreRunning
                ? "external-controller 在线"
                : "离线"
              : "待检测"
          }
        />
        <SystemItem
          label="代理生效"
          status={native ? native.tunOn || native.sysproxyOn : null}
          value={
            native
              ? native.tunOn
                ? "TUN 全局接管"
                : native.sysproxyOn
                  ? `系统代理 ${native.sysproxyPort}`
                  : "未设置"
              : "待检测"
          }
        />
        {requireClaudeReachable && (
          <SystemItem
            label="Claude 连通性"
            status={native?.claudeReachable}
            value={
              native
                ? native.claudeReachable
                  ? `可达（HTTP ${native.claudeStatus}）`
                  : (native.claudeError ?? "未通过检测")
                : "待检测"
            }
          />
        )}
      </div>
      <div
        id="system-timezone"
        role="group"
        aria-label="时区合规"
        className="check-row space-y-3 rounded-lg bg-zinc-900/60 p-3"
      >
        <div className="flex flex-wrap items-center justify-between gap-2 text-xs">
          <span className="font-medium text-zinc-300">时区合规</span>
          <button
            className={`${button} flex items-center gap-1.5`}
            disabled={busy || disabled}
            onClick={() => void syncTimezone()}
          >
            <RefreshCw className="h-3 w-3" />
            按实际出口同步时区
          </button>
        </div>
        <dl className="grid grid-cols-1 gap-3 text-xs sm:grid-cols-2 lg:grid-cols-4">
          <div>
            <dt className="text-zinc-500">首选语言</dt>
            <dd className="mt-1 break-words select-text text-zinc-200">
              {state?.current.languages.join("、") || unread}
            </dd>
            {signalDetails(["language"])}
          </div>
          <div>
            <dt className="text-zinc-500">区域格式</dt>
            <dd className="mt-1 break-words select-text text-zinc-200">
              {state?.current.culture || unread}
            </dd>
            {signalDetails(["intlLocale"])}
          </div>
          <div>
            <dt className="text-zinc-500">显示语言偏好</dt>
            <dd className="mt-1 break-words select-text text-zinc-200">
              {state ? state.current.uiLanguage || "跟随系统" : unread}
            </dd>
          </div>
          <div>
            <dt className="text-zinc-500">时区</dt>
            <dd className="mt-1 break-words select-text text-zinc-200">
              {state?.current.timezone || native?.currentTz || unread}
            </dd>
            {native?.exitTimezone && (
              <div className="mt-1 break-words text-[11px] text-zinc-500">
                出口时区：{native.exitTimezone}
              </div>
            )}
            {native && (
              <div className="mt-1 break-words text-[11px] text-zinc-500">
                浏览器线路：
                {native.tunOn
                  ? "TUN 接管"
                  : native.sysproxyOn
                    ? "系统代理"
                    : "直连（探测到的是本地线路）"}
              </div>
            )}
            {signalDetails(["timezone", "timezoneOffset"])}
          </div>
        </dl>
        <DetectionFinding verdict={timezoneVerdict} msg={timezoneMessage} />
      </div>
      <div
        role="group"
        aria-label="地区设置"
        className="flex flex-wrap items-center gap-2 pt-1"
      >
        <label className="flex min-w-0 flex-1 basis-64 items-center gap-2 text-xs text-zinc-500">
          <span className="shrink-0">地区预设</span>
          <select
            aria-label="Windows 地区预设"
            className="min-w-0 flex-1 rounded-lg border border-zinc-700 bg-zinc-950 px-3 py-2 text-xs text-zinc-200 disabled:opacity-40"
            value={region}
            disabled={busy || disabled || !state}
            onChange={(event) => setRegion(event.target.value)}
          >
            {state?.presets.map((preset) => (
              <option key={preset.id} value={preset.id}>
                {preset.name}
              </option>
            ))}
          </select>
        </label>
        <button
          className={`${button} btn-primary`}
          disabled={busy || disabled || !state}
          onClick={() => void apply()}
        >
          备份并应用地区
        </button>
        <button
          className={button}
          disabled={busy || disabled || !state?.backup}
          onClick={() => void apply(true)}
        >
          恢复原语言、地区与时区
        </button>
      </div>
      <p className="text-[11px] leading-5 text-zinc-500">
        应用地区会备份并更新语言、区域格式和时区，修改后需重新检测。时区以实际出口为准；部分设置需要权限、语言包或重新登录。
      </p>
      {message && (
        <div
          role="status"
          className="whitespace-pre-wrap rounded-lg border border-zinc-700 bg-zinc-800/60 p-3 text-xs leading-6 text-zinc-300"
        >
          {message}
        </div>
      )}
      {pending && (
        <ConfirmDialog
          tone="warning"
          icon={pending.restore ? RotateCcw : Monitor}
          eyebrow="系统设置"
          title={
            pending.restore
              ? "恢复首次操作前的电脑环境？"
              : "将修改 Windows 语言、区域与格式"
          }
          description={
            pending.restore
              ? "用第一次应用前保存的备份覆盖当前语言、区域格式和时区；之后新增的语言和输入法保持不动。"
              : "会保留其他语言和输入法，改动前先保存恢复资料。显示语言可能需要语言包或重新登录。"
          }
          details={
            pending.restore
              ? [
                  {
                    label: "当前语言",
                    value: current?.languages.join("、") || "—",
                  },
                  { label: "当前区域格式", value: current?.culture || "—" },
                  { label: "当前时区", value: current?.timezone || "—" },
                ]
              : [
                  { label: "地区预设", value: preset?.name ?? region },
                  { label: "首选语言", value: preset?.language ?? "—" },
                  { label: "区域格式", value: preset?.language ?? "—" },
                  { label: "时区", value: preset?.timezone ?? "—" },
                ]
          }
          note={{
            tone: "neutral",
            text: "选择地区不会更改公网 IP；改完需要重新检测，部分设置需要管理员权限。",
          }}
          cancelLabel="取消"
          confirmLabel={pending.restore ? "恢复设置" : "备份并应用"}
          busyLabel={pending.restore ? "正在恢复…" : "正在应用…"}
          busy={applying}
          error={applyError}
          onCancel={() => {
            if (applying) return;
            setPending(null);
            setApplyError(null);
          }}
          onConfirm={() => void confirmApply()}
        />
      )}
    </div>
  );
}

function SystemItem({
  label,
  value,
  status,
}: {
  label: string;
  value: string;
  status?: boolean | null;
}) {
  return (
    <div
      role="group"
      aria-label={label}
      className="check-row flex min-w-0 items-center gap-3 rounded-lg bg-zinc-900/60 px-3 py-3"
    >
      <span className="shrink-0 text-zinc-500 [&>svg]:h-3.5 [&>svg]:w-3.5">
        {status === true ? (
          <CheckCircle2 className="text-emerald-400" />
        ) : status === false ? (
          <XCircle className="text-red-400" />
        ) : (
          <CircleDashed />
        )}
      </span>
      <div className="min-w-0 flex-1 basis-28">
        <div className="text-zinc-300">{label}</div>
        <div className="mt-0.5 break-words select-text text-[11px] text-zinc-500">
          {value}
        </div>
      </div>
    </div>
  );
}
