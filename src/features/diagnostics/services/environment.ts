import { call } from "../../../services/api/invoke";
import { useDiagnosticsStore } from "../state";
import type { Region } from "../../../shared/types/environment";

export function invalidateEnvironment() {
  useDiagnosticsStore.getState().resetDetect();
}
export interface ComputerStatus {
  current: {
    languages: string[];
    culture: string;
    uiLanguage: string;
    timezone: string;
  };
  backup: unknown;
  presets: Region[];
}
export const computerEnvironment = {
  computer: () => call<ComputerStatus>("computer_environment_status"),
  applyComputer: (region: string, restore = false) =>
    call<{ message: string }>("computer_environment_apply", {
      region,
      restore,
      confirmed: true,
    }),
};
