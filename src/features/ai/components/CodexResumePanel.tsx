import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Clock3, RefreshCw, ShieldCheck, Trash2, Zap } from "lucide-react";
import { usePolling } from "../../../shared/hooks/usePolling";
import {
  codexResume,
  claudeResume,
  resumeCountdown,
  resumeStateLabel,
  type ResumeCandidate,
  type ResumeStatus,
} from "../services/codexResume";
import { cliToolName } from "../services/aiToolCards";

export default function CodexResumePanel({
  active,
  onBusyChange,
  tool = "codex",
}: {
  active: boolean;
  onBusyChange?: (busy: boolean) => void;
  tool?: "codex" | "claude";
}) {
  const client = tool === "claude" ? claudeResume : codexResume;
  const toolName = cliToolName(tool);
  const [status, setStatus] = useState<ResumeStatus | null>(null);
  const [candidates, setCandidates] = useState<ResumeCandidate[]>([]);
  const [thread, setThread] = useState("");
  const [manual, setManual] = useState("");
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [now, setNow] = useState(Date.now() / 1000);
  const working = useRef(false),
    revision = useRef(0),
    mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const refresh = useCallback(
    async (isCurrent: () => boolean = () => true) => {
      const version = revision.current;
      try {
        const next = await client.status();
        if (
          mounted.current &&
          isCurrent() &&
          version === revision.current &&
          !working.current
        ) {
          setStatus(next);
          setError("");
        }
      } catch {
        if (mounted.current && isCurrent() && version === revision.current)
          setError("自动续跑状态读取失败，请重试。");
      }
    },
    [client],
  );
  usePolling(refresh, 5000, true, active);
  useEffect(() => {
    if (!active) return;
    let disposed = false,
      unlisten: (() => void) | undefined;
    void listen(`${tool}-resume-changed`, () => {
      if (!disposed) void refresh(() => !disposed);
    })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch(() => {});
    const timer = setInterval(() => setNow(Date.now() / 1000), 1000);
    return () => {
      disposed = true;
      clearInterval(timer);
      unlisten?.();
    };
  }, [active, refresh, tool]);
  const run = async (
    name: string,
    operation: () => Promise<void>,
    success = "",
  ) => {
    if (working.current) return;
    working.current = true;
    revision.current += 1;
    setBusy(name);
    setError("");
    setMessage("");
    onBusyChange?.(true);
    try {
      await operation();
      if (mounted.current) setMessage(success);
    } catch (reason) {
      if (mounted.current) setError(String(reason));
    } finally {
      working.current = false;
      if (mounted.current) setBusy("");
      onBusyChange?.(false);
    }
  };
  const loadCandidates = () =>
    run("sessions", async () => setCandidates(await client.candidates()));
  const add = (id: string) =>
    run(
      "watch",
      async () => {
        setStatus(await client.watch(id.trim()));
        setThread("");
        setManual("");
      },
      "已守候此会话一次；额度中断后会自动核验。",
    );
  const compatible = status?.compatibility.supported ?? false;
  return (
    <div className="codex-resume-panel" aria-label={`${toolName}自动续跑设置`}>
      <div className="resume-heading">
        <span>
          <Zap size={16} />
          额度恢复后继续
        </span>
        <p>在原会话接着完成未完成的工作。</p>
      </div>
      <div className="resume-consent">
        <div>
          <strong>自动续跑</strong>
          <span>
            {status?.enabled ? "已开启 · 仅限所选会话" : "默认关闭，按会话授权"}
          </span>
        </div>
        <button
          type="button"
          role="switch"
          aria-label={`${toolName}自动续跑总开关`}
          aria-checked={status?.enabled ?? false}
          disabled={!!busy || !status || (!status.enabled && !compatible)}
          onClick={() =>
            void run(
              "enable",
              async () => setStatus(await client.enable(!status?.enabled)),
              status?.enabled
                ? tool === "claude"
                  ? "已关闭，正在停止本软件启动的恢复进程。"
                  : "已关闭；已提交消息仍需核实撤回结果。"
                : "已开启，请选择要守候的会话。",
            )
          }
          className="resume-switch"
        >
          <i />
        </button>
      </div>
      <div className={`resume-compatibility ${compatible ? "is-ready" : ""}`}>
        <ShieldCheck size={14} />
        <div>
          <strong>{compatible ? "本机接口可用" : "先检查本机兼容性"}</strong>
          <p>
            {status?.compatibility.message ||
              "检查引擎、会话记录和实时额度，不会发送消息。"}
          </p>
          {status?.compatibility.engineVersion && (
            <small>{status.compatibility.engineVersion}</small>
          )}
        </div>
      </div>
      <button
        type="button"
        className="resume-secondary"
        disabled={!!busy}
        onClick={() =>
          void run("check", async () => setStatus(await client.check()))
        }
      >
        <RefreshCw
          size={13}
          className={busy === "check" ? "animate-spin" : ""}
        />
        {busy === "check" ? "正在检查…" : "检查兼容性 / 重新核验"}
      </button>
      {error && (
        <p className="resume-error" role="alert">
          {error}
        </p>
      )}
      {message && (
        <p className="resume-feedback" role="status">
          {message}
        </p>
      )}
      <div className="resume-section-heading">
        <strong>守候的会话</strong>
        <span>单次恢复 · 7 天有效</span>
      </div>
      {!status?.watches.length && (
        <div className="resume-empty">
          <Clock3 size={20} />
          <span>还没有守候的会话</span>
          <p>开启后选择原会话，云阔会等待它的额度恢复。</p>
        </div>
      )}
      {status?.watches.map((watch) => {
        const record = status.recoveries.find(
          (r) => r.threadId === watch.threadId,
        );
        const listening = watch.enabled && !watch.consumed && status.enabled;
        return (
          <article
            key={watch.threadId}
            className={`resume-watch ${record?.state === "running" || record?.state === "completed" ? "is-running" : ""}`}
          >
            <div className="resume-watch-title">
              <strong>{watch.title}</strong>
              <button
                type="button"
                aria-label={`移除守候 ${watch.title}`}
                disabled={!!busy || record?.mayBeQueued}
                onClick={() =>
                  void run("forget", async () =>
                    setStatus(await client.forget(watch.threadId)),
                  )
                }
              >
                <Trash2 size={12} />
              </button>
            </div>
            <span className="resume-project">
              {watch.project || "本机会话"} · {watch.threadId.slice(0, 8)}
            </span>
            <div className="resume-state">
              <i />
              {record
                ? resumeStateLabel(record.state)
                : listening
                  ? "等待此会话发生额度中断"
                  : "守候已关闭"}
            </div>
            {record && (
              <>
                <p>{record.reason}</p>
                {record.resetAt && (
                  <div className="resume-reset">
                    <Clock3 size={12} />
                    <span>
                      {resumeCountdown(record.resetAt, now)}
                      <small>
                        预计{" "}
                        {new Date(record.resetAt * 1000).toLocaleString(
                          "zh-CN",
                        )}
                      </small>
                    </span>
                  </div>
                )}
                {record.checkedAt && (
                  <small>
                    额度核验：
                    {new Date(record.checkedAt * 1000).toLocaleTimeString(
                      "zh-CN",
                    )}
                  </small>
                )}
                {record.cancelRequested && record.mayBeQueued && (
                  <p className="resume-pending-cancel">
                    {tool === "claude"
                      ? "正在停止恢复进程，请等待停止结果；已完成的操作会保留。"
                      : "已停止后续恢复，消息仍可能送达。请查看核实结果。"}
                  </p>
                )}
              </>
            )}
            <div className="resume-watch-actions">
              {listening || record?.mayBeQueued ? (
                <button
                  type="button"
                  disabled={!!busy || record?.cancelRequested}
                  onClick={() =>
                    void run("cancel", async () =>
                      setStatus(await client.cancel(watch.threadId)),
                    )
                  }
                >
                  取消本次恢复
                </button>
              ) : (
                <button
                  type="button"
                  disabled={!!busy || !status.enabled || !compatible}
                  onClick={() => void add(watch.threadId)}
                >
                  重新守候一次
                </button>
              )}
            </div>
          </article>
        );
      })}
      <details
        className="resume-add"
        open={status?.enabled && !status.watches.length ? true : undefined}
      >
        <summary>选择要守候的会话</summary>
        <button
          type="button"
          className="resume-secondary"
          disabled={!!busy}
          onClick={() => void loadCandidates()}
        >
          <RefreshCw size={12} />
          {busy === "sessions"
            ? "读取中…"
            : tool === "claude"
              ? "读取本地 CLI 会话"
              : "读取最近桌面会话"}
        </button>
        <label>
          最近会话
          <select
            aria-label={`选择 ${toolName} 守候会话`}
            value={thread}
            onChange={(e) => setThread(e.target.value)}
            disabled={!!busy}
          >
            <option value="">请选择会话</option>
            {candidates.map((c) => (
              <option key={c.threadId} value={c.threadId}>
                {c.title} ·{" "}
                {c.quotaStopped ? "额度中断" : c.project || "本地会话"}
              </option>
            ))}
          </select>
        </label>
        <button
          type="button"
          className="resume-primary"
          disabled={!!busy || !status?.enabled || !compatible || !thread}
          onClick={() => void add(thread)}
        >
          守候所选会话一次
        </button>
        <label>
          或输入完整会话 UUID
          <input
            aria-label={`${toolName} 会话 UUID`}
            placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
            value={manual}
            onChange={(e) => setManual(e.target.value)}
            disabled={!!busy}
            spellCheck={false}
          />
        </label>
        <button
          type="button"
          className="resume-secondary"
          disabled={
            !!busy ||
            !status?.enabled ||
            !compatible ||
            !/^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$/.test(manual.trim())
          }
          onClick={() => void add(manual)}
        >
          按 UUID 守候一次
        </button>
      </details>
      {!!status?.events.length && (
        <details className="resume-history">
          <summary>恢复记录 · {status.events.length}</summary>
          <ol>
            {status.events.slice(0, 16).map((event, i) => (
              <li key={`${event.at}-${i}`}>
                <time>
                  {new Date(event.at * 1000).toLocaleTimeString("zh-CN")}
                </time>
                <span>{event.reason}</span>
              </li>
            ))}
          </ol>
        </details>
      )}
      <p className="resume-footnote">
        {tool === "claude"
          ? "保持云阔在托盘运行。仅恢复明确选定的本地 Claude Code CLI 会话；恢复前请关闭其他 Claude 实例。订阅额度恢复后在原目录续跑，遇到审批会暂停；不接管桌面 Code、Chat 或 Cowork。"
          : "保持云阔在托盘运行，并在 Codex 中打开目标会话。只处理额度耗尽；用户停止、待审批及已完成的任务不会自动启动。继续消息会带一行用于核实送达的标记。"}
      </p>
    </div>
  );
}
