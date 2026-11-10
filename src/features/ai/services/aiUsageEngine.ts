import { call } from "../../../services/api/invoke";

/** 统一用量采集引擎的运行状态（随包 tokscale，由 aimon 引擎维护）。 */
export interface AiUsageEngineStatus {
  available: boolean;
  running: boolean;
  lastSuccessAt: number | null;
  issue: string | null;
}

export const aiUsageEngine = {
  status: () => call<AiUsageEngineStatus>("ai_usage_engine_status"),
  refresh: () => call<AiUsageEngineStatus>("ai_usage_engine_refresh"),
};
