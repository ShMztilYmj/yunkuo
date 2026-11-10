import type { StoreApi, UseBoundStore } from "zustand";

/** 与兼容 store 共享订阅和对象，只暴露模块状态及动作。 */
export function moduleStore<ModuleState>() {
  return <State extends ModuleState>(store: UseBoundStore<StoreApi<State>>) =>
    Object.assign(
      <Value>(selector: (state: ModuleState) => Value) => store(selector),
      {
        getState: (): ModuleState => store.getState(),
      },
    );
}
