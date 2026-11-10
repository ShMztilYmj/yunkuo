import type { SignalResult } from "../signals/web";
import type { ExitResult } from "../signals/exit";
import type { NativeDetect } from "../../../services/api/detect";
import type { PrivacyStatus } from "../../../services/api/network";
import type { SlicePort } from "../../../shared/state/slice";

export interface ScanResult {
  finished: boolean;
  scanning: boolean;
  webResults: SignalResult[];
  webTotal: number;
  native: NativeDetect | null;
  exit: ExitResult | null;
  at: number | null;
}

export interface DiagnosticsState {
  detect: ScanResult;
  setDetect: (result: ScanResult) => void;
  resetDetect: () => void;
}

const emptyScan = (): ScanResult => ({
  finished: false,
  scanning: false,
  webResults: [],
  webTotal: 0,
  native: null,
  exit: null,
  at: null,
});

/** 应用层组合代理快照时同步投影，避免订阅者观察到保护已失效但检测仍健康。 */
export function projectProtection(
  detect: ScanResult,
  privacy: PrivacyStatus | null,
): ScanResult {
  const native = detect.native;
  if (!native) return detect;
  const projection = {
    privacyArmed: privacy?.armed ?? false,
    privacyHealthy: privacy?.healthy === true && privacy?.tunnelOn === true,
    privacyError: privacy?.error ?? (privacy ? null : "网络保护服务未就绪"),
  };
  if (
    native.privacyArmed === projection.privacyArmed &&
    native.privacyHealthy === projection.privacyHealthy &&
    native.privacyError === projection.privacyError
  )
    return detect;
  return { ...detect, native: { ...native, ...projection } };
}

export function createDiagnosticsSlice({
  set,
}: SlicePort<DiagnosticsState>): DiagnosticsState {
  return {
    detect: emptyScan(),
    setDetect: (detect) => set({ detect }),
    resetDetect: () => set({ detect: emptyScan() }),
  };
}
