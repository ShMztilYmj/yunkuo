/** 同一存储内的模块实现只读写自己的状态，应用层负责跨模块协调。 */
export interface SlicePort<State> {
  get: () => State;
  set: (
    change: Partial<State> | ((state: State) => Partial<State> | State),
  ) => void;
}
