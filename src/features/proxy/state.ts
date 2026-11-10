import { moduleStore } from "../../shared/state/moduleStore";
import { useAppStore } from "../../store";
import type { ProxyConfiguration } from "./configuration";
import type { ProxyState } from "./state/slice";

type ProxyView = ProxyState & {
  config: Readonly<ProxyConfiguration> | null;
  refreshConfig: () => Promise<void>;
  configLoading: boolean;
  configError: string | null;
};

/** 页面从模块接口读取同一份状态；不开放任意setState或跨模块patch。 */
export const useProxyStore = moduleStore<ProxyView>()(useAppStore);
