import type { AppConfig } from "../services/api/config";
import { aiToolCards } from "../features/ai/services/aiToolCards";
import { setAiUsageScope } from "../features/ai/services/aiUsage";

interface ConfigSource {
  getState: () => { config: AppConfig | null };
  subscribe: (
    listener: (
      state: { config: AppConfig | null },
      previous: { config: AppConfig | null },
    ) => void,
  ) => () => void;
}

/** 跨模块副作用由应用挂载/卸载，导入store不再隐式启动AI监测。 */
export function bindAiUsageScope(store: ConfigSource): () => void {
  const update = (config: AppConfig | null) =>
    setAiUsageScope(aiToolCards(config).map((tool) => tool.key));
  update(store.getState().config);
  return store.subscribe((state, previous) => {
    if (state.config !== previous.config) update(state.config);
  });
}
