import { useSyncExternalStore } from "react";
import {
  removedProviderKeys,
  subscribeRemovedProviders,
} from "../services/providerVisibility";

/** 被移除的服务商键：移除与撤销都会让订阅它的页面立刻重算列表。 */
export function useRemovedProviders(): ReadonlySet<string> {
  return useSyncExternalStore(
    subscribeRemovedProviders,
    removedProviderKeys,
    removedProviderKeys,
  );
}

export default useRemovedProviders;
