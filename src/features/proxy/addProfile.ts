import { create } from "zustand";

/**
 * 「添加订阅」弹窗的状态。
 *
 * 入口是订阅页右下角的圆形「+」，空态也复用它；弹窗随订阅页挂载。
 * `revision` 在导入成功后自增，订阅页据此重新拉取列表——同一路由下不会重新挂载组件。
 */
interface AddProfileState {
  open: boolean;
  revision: number;
  show: () => void;
  hide: () => void;
  committed: () => void;
}

export const useAddProfile = create<AddProfileState>((set) => ({
  open: false,
  revision: 0,
  show: () => set({ open: true }),
  hide: () => set({ open: false }),
  committed: () => set((state) => ({ revision: state.revision + 1 })),
}));
