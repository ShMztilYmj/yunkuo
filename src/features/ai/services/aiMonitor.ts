import { create } from "zustand";
import { api, type AiMonitorStatus } from "../../../services/api";

const empty: AiMonitorStatus = {
  enabled: false,
  notices: [],
  summary: { todayTokens: 0, running: 0, scannedAt: null },
  deliveryError: null,
};
export const useAiMonitor = create<{
  status: AiMonitorStatus;
  error: string | null;
  loaded: boolean;
}>(() => ({ status: empty, error: null, loaded: false }));
let pending: Promise<void> | undefined;
export function refreshAiMonitor(force = false): Promise<void> {
  if (pending) return force ? pending.then(() => refreshAiMonitor()) : pending;
  const request = api
    .aiMonitorStatus()
    .then((status) => {
      useAiMonitor.setState({
        status: {
          ...empty,
          ...status,
          notices: status?.notices ?? [],
          summary: status?.summary ?? empty.summary,
        },
        loaded: true,
        error: null,
      });
    })
    .catch((error) => {
      useAiMonitor.setState({
        loaded: true,
        error: `读取提醒状态失败：${String(error)}`,
      });
    })
    .finally(() => {
      if (pending === request) pending = undefined;
    });
  pending = request;
  return request;
}
