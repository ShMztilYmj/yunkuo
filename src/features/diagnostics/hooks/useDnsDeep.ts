import { useCallback, useEffect, useRef, useState } from "react";
import { runDnsDeep, type DnsDeepProgress } from "../signals/dnsDeep";
import type { DnsDeepReport } from "../../../services/api";
import { useProxyStore } from "../../proxy/state";

export interface DnsDeepController {
  report: DnsDeepReport | null;
  busy: boolean;
  progress: DnsDeepProgress | null;
  message: string | null;
  at: number | null;
  /** 跑一轮深度测试；已有任务在跑时直接返回 null，不排队。 */
  run: () => Promise<DnsDeepReport | null>;
  stop: () => void;
}

/**
 * DNS 深度测试的会话状态：3 轮 × 12 个唯一域名，经 WebView 真实路径发起。
 * 一键检测与卡片上的按钮共用这一份状态，避免两处各自跑出不同结论。
 */
export function useDnsDeep(): DnsDeepController {
  const [report, setReport] = useState<DnsDeepReport | null>(null);
  const [progress, setProgress] = useState<DnsDeepProgress | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [at, setAt] = useState<number | null>(null);
  const runRef = useRef<AbortController | null>(null);
  const networkKey = useProxyStore((state) =>
    [
      state.config?.enable_tun,
      state.config?.enable_system_proxy,
      state.config?.mode,
      state.config?.mixed_port,
      state.config?.ipv6,
      state.currentProfile?.uid,
    ].join("|"),
  );

  useEffect(() => {
    runRef.current?.abort("network");
    setReport(null);
    setAt(null);
  }, [networkKey]);

  useEffect(() => {
    const changed = () => {
      runRef.current?.abort("network");
      setReport(null);
      setAt(null);
      setMessage("联网状态发生变化，请重新进行深度检测。");
    };
    window.addEventListener("offline", changed);
    window.addEventListener("online", changed);
    return () => {
      window.removeEventListener("offline", changed);
      window.removeEventListener("online", changed);
    };
  }, []);

  useEffect(
    () => () => {
      runRef.current?.abort();
      runRef.current = null;
    },
    [],
  );

  const stop = useCallback(() => {
    runRef.current?.abort();
  }, []);

  const run = useCallback(async () => {
    if (runRef.current) return null;
    const control = new AbortController();
    runRef.current = control;
    setBusy(true);
    setReport(null);
    setMessage(null);
    setAt(null);
    setProgress(null);
    const timeout = setTimeout(() => control.abort("timeout"), 90_000);
    try {
      const result = await runDnsDeep(control.signal, (value) => {
        if (runRef.current === control) setProgress(value);
      });
      if (runRef.current !== control) return null;
      setReport(result);
      setAt(Date.now());
      return result;
    } catch (error) {
      if (runRef.current !== control) return null;
      setMessage(
        control.signal.aborted
          ? control.signal.reason === "timeout"
            ? "深度检测超过 90 秒，尚未完成核验，请稍后重试。"
            : control.signal.reason === "network"
              ? "联网或代理设置发生变化，本轮已停止，请重新测试。"
              : "深度检测已停止，尚未完成核验。"
          : `深度检测未完成：${String(error)}`,
      );
      return null;
    } finally {
      clearTimeout(timeout);
      if (runRef.current === control) {
        runRef.current = null;
        setBusy(false);
      }
    }
  }, []);

  return { report, busy, progress, message, at, run, stop };
}
