import { create } from "zustand";
import { shallow } from "zustand/shallow";
import { api } from "./services/api";
import {
  createConfigurationSlice,
  type ConfigurationState,
} from "./app/state/configuration";
import {
  createProxySlice,
  type ProxyState,
} from "./features/proxy/state/slice";
import {
  createDiagnosticsSlice,
  projectProtection,
  type DiagnosticsState,
} from "./features/diagnostics/state/slice";

export type { ScanResult } from "./features/diagnostics/state/slice";

export interface AppState
  extends ConfigurationState, ProxyState, DiagnosticsState {
  windowVisible: boolean;
  load: () => Promise<void>;
}

/** 兼容入口只组合各模块，业务状态仍只有这一份。 */
export const useAppStore = create<AppState>((set, get) => ({
  ...createConfigurationSlice({ set, get }, api),
  ...createProxySlice({ set, get }, api, (coreStatus, privacyStatus) =>
    set((state) => {
      const detect = projectProtection(state.detect, privacyStatus);
      if (
        shallow(state.coreStatus, coreStatus) &&
        shallow(state.privacyStatus, privacyStatus) &&
        detect === state.detect
      )
        return state;
      return { coreStatus, privacyStatus, detect };
    }),
  ),
  ...createDiagnosticsSlice({ set, get }),
  windowVisible: true,
  load: async () => {
    void Promise.allSettled([get().refreshCore(), get().refreshProfile()]);
    await get().refreshConfig();
  },
}));
