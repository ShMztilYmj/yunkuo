import type { AppConfig } from "../../services/api/config";
import type { SlicePort } from "../../shared/state/slice";

export interface ConfigurationState {
  config: AppConfig | null;
  configLoading: boolean;
  configError: string | null;
  refreshConfig: () => Promise<void>;
  patch: (patch: Partial<AppConfig>) => Promise<void>;
  updateConfig: (config: AppConfig) => void;
  mutateConfig: (operation: () => Promise<AppConfig>) => Promise<void>;
}

interface ConfigurationApi {
  getConfig: () => Promise<AppConfig>;
  patchConfig: (patch: Partial<AppConfig>) => Promise<AppConfig>;
}

/** 配置事件、读取和所有写操作共用版本与队列，保持持久化文件兼容。 */
export function createConfigurationSlice(
  { set, get }: SlicePort<ConfigurationState>,
  api: ConfigurationApi,
): ConfigurationState {
  let refresh: Promise<void> | null = null;
  let revision = 0;
  let eventRevision = 0;
  let requestedReadRevision = 0;
  let writes: Promise<void> = Promise.resolve();
  const publish = (config: AppConfig) => {
    revision++;
    set({ config, configError: null });
  };
  return {
    config: null,
    configLoading: false,
    configError: null,
    updateConfig: (config) => {
      eventRevision++;
      publish(config);
    },
    mutateConfig: (operation) => {
      revision++;
      const task = writes.then(async () => {
        revision++;
        const observedEvent = eventRevision;
        const config = await operation();
        // 后续写仍在队列中，不能撤销当前成功结果；只有较新的权威事件可覆盖它。
        if (observedEvent === eventRevision) publish(config);
      });
      writes = task.catch(() => undefined);
      return task;
    },
    patch: (patch) => {
      const submitted = structuredClone(patch);
      return get().mutateConfig(() => api.patchConfig(submitted));
    },
    refreshConfig: () => {
      requestedReadRevision = revision;
      if (refresh) return refresh;
      set({ configLoading: true, configError: null });
      refresh = (async () => {
        for (;;) {
          await writes;
          const observed = revision;
          try {
            const config = await api.getConfig();
            if (observed === revision) {
              publish(config);
              return;
            }
          } catch (error) {
            if (requestedReadRevision <= observed) {
              if (observed === revision) set({ configError: String(error) });
              throw error;
            }
          }
          // 已有权威配置时只丢弃旧读；旧读失效后加入的刷新需要一次新读取。
          if (requestedReadRevision <= observed) return;
        }
      })().finally(() => {
        refresh = null;
        set({ configLoading: false });
      });
      return refresh;
    },
  };
}
