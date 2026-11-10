import { Globe2, Loader2, RefreshCw } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../../../services/api";
import { errorMessage } from "../../../services/api/invoke";
import { SignalRow } from "./SignalRow";
import { buildSelfcheckScript } from "../selfcheck/probe";
import {
  parseSelfcheckReport,
  selfcheckSignals,
  selfcheckSummary,
  type SelfcheckReport,
} from "../selfcheck/report";

type Phase = "idle" | "starting" | "waiting" | "done" | "error";

const sleep = (ms: number) =>
  new Promise<void>((resolve) => {
    setTimeout(resolve, ms);
  });

/**
 * 设备与字体指纹卡。
 *
 * 只展示指纹浏览器自己回传的实测结果——它是另一个 Chromium 进程，UA / 语言 /
 * 时区 / 硬件并发都可能与本窗口不同，所以不在本窗口 WebView2 里做同类自检。
 */
export default function FingerprintSection() {
  const [phase, setPhase] = useState<Phase>("idle");
  const [report, setReport] = useState<SelfcheckReport | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const alive = useRef(true);

  useEffect(
    () => () => {
      alive.current = false;
    },
    [],
  );

  // 换身份、换指纹种子、改时区都会关闭浏览器，旧的自检结论随即失效。
  useEffect(() => {
    const unlisten = listen("environment-invalidated", () => {
      setPhase("idle");
      setReport(null);
      setMessage(null);
    });
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, []);

  const busy = phase === "starting" || phase === "waiting";

  const run = async () => {
    if (busy) return;
    alive.current = true;
    setPhase("starting");
    setMessage(null);
    setReport(null);
    try {
      const started = await api.browserSelfcheckStart(buildSelfcheckScript());
      setPhase("waiting");
      const deadline = Date.now() + 60_000;
      while (alive.current && Date.now() < deadline) {
        await sleep(800);
        if (!alive.current) return;
        const status = await api.browserSelfcheckStatus(started.ticket);
        if (!alive.current) return;
        if (status.ready) {
          const parsed = parseSelfcheckReport(status.report);
          if (!parsed) {
            setPhase("error");
            setMessage("回传的数据不完整，请重新自检。");
            return;
          }
          setReport(parsed);
          setPhase("done");
          return;
        }
        if (status.expiresIn === 0) break;
      }
      if (alive.current) {
        setPhase("error");
        setMessage(
          "自检未完成：请确认指纹浏览器已打开，并允许它访问 127.0.0.1 上的本地页面。",
        );
      }
    } catch (error) {
      if (alive.current) {
        setPhase("error");
        setMessage(errorMessage(error));
      }
    }
  };

  const remote = report ? selfcheckSignals(report) : [];

  return (
    <div className="ui-panel rounded-2xl border border-zinc-800 bg-zinc-900/40 p-6">
      <div className="mb-3 flex flex-wrap items-center gap-2 text-sm font-medium">
        <Globe2 className="h-4 w-4 text-cyan-400" />
        <h2>设备与字体指纹</h2>
        <button
          type="button"
          onClick={() => void run()}
          disabled={busy}
          className="ml-auto flex items-center gap-1.5 rounded-lg border border-zinc-700 bg-zinc-800 px-3 py-1.5 text-[11px] text-zinc-200 hover:bg-zinc-700 disabled:cursor-not-allowed disabled:opacity-40"
        >
          {busy ? (
            <Loader2 className="h-3 w-3 animate-spin" />
          ) : (
            <RefreshCw className="h-3 w-3" />
          )}
          {phase === "done" ? "重新自检" : "在指纹浏览器中自检"}
        </button>
      </div>

      <div className="mb-1.5 flex items-center gap-1.5 text-[10.5px] text-zinc-500">
        <span
          className={
            "h-1.5 w-1.5 shrink-0 rounded-full " +
            (phase === "done" && report ? "bg-emerald-500" : "bg-zinc-600")
          }
        />
        指纹浏览器 · 实测回传
      </div>
      {phase === "done" && report ? (
        <div className="space-y-2">
          <div className="rounded-lg border border-zinc-800/70 bg-zinc-900/40 px-3 py-2">
            <p className="break-words text-[11px] leading-relaxed text-zinc-500">
              {selfcheckSummary(report)}
            </p>
            <p className="mt-0.5 break-words text-[10px] leading-relaxed text-zinc-600">
              {[
                report.screen.length >= 2
                  ? `${report.screen[0]}×${report.screen[1]}`
                  : "",
                report.deviceMemory ? `${report.deviceMemory} GB` : "",
                report.webgl,
              ]
                .filter(Boolean)
                .join(" · ")}
            </p>
          </div>
          <div className="grid grid-cols-[repeat(auto-fit,minmax(320px,1fr))] gap-1.5">
            {remote.map((result) => (
              <SignalRow key={result.id} result={result} />
            ))}
          </div>
        </div>
      ) : busy ? (
        <div className="flex items-center gap-2 rounded-lg bg-zinc-900/60 px-3 py-2 text-[11px] text-zinc-400">
          <Loader2 className="h-3 w-3 shrink-0 animate-spin" />
          {phase === "starting"
            ? "正在启动指纹浏览器…"
            : "等待指纹浏览器回传（最多 60 秒）…"}
        </div>
      ) : phase === "error" ? (
        <div className="rounded-lg border border-red-500/30 bg-red-500/5 px-3 py-2 text-[11px] leading-5 text-red-300">
          {message}
        </div>
      ) : (
        <div className="rounded-lg bg-zinc-900/60 px-3 py-2 text-[11px] leading-5 text-zinc-500">
          尚未自检。点击右上角按钮会在指纹浏览器里打开一个临时页面，采完即回传并自动关闭；
          字体与 Emoji 来自系统字形。
        </div>
      )}
    </div>
  );
}
