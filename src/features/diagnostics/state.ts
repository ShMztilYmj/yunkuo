import { moduleStore } from "../../shared/state/moduleStore";
import { useAppStore } from "../../store";
import type { DiagnosticsState } from "./state/slice";

export type { ScanResult } from "./state/slice";
export const useDiagnosticsStore = moduleStore<DiagnosticsState>()(useAppStore);
