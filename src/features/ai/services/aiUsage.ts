import { api, type AiSnapshot } from "../../../services/api";

let cached: { snapshot: AiSnapshot; at: number } | undefined;
let pending: { request: Promise<AiSnapshot>; generation: number } | undefined;
let generation = 0;
let scope: ReadonlySet<string> | undefined;
export function setAiUsageScope(keys: readonly string[]) {
  if (
    scope &&
    scope.size === keys.length &&
    keys.every((key) => scope!.has(key))
  )
    return;
  scope = new Set(keys);
  invalidateAiUsage();
}
const scoped = (snapshot: AiSnapshot): AiSnapshot =>
  scope
    ? { items: snapshot.items.filter((item) => scope!.has(item.key)) }
    : snapshot;
/** 账号变更后丢弃旧缓存；正在读取的旧结果不能重新填入缓存。 */
export function invalidateAiUsage() {
  generation++;
  cached = undefined;
}
/** 首页与统计页切换时复用最近一次读取，同一时刻只运行一个本地扫描。 */
export function readAiUsage(force = false): Promise<AiSnapshot> {
  if (pending) {
    if (pending.generation === generation) return pending.request;
    return pending.request
      .catch(() => undefined)
      .then(() => readAiUsage(force));
  }
  if (!force && cached && Date.now() - cached.at < 20_000)
    return Promise.resolve(cached.snapshot);
  const started = generation;
  const request = api
    .aiUsageStatus()
    .then((snapshot) => {
      const selected = scoped(snapshot);
      if (started === generation)
        cached = { snapshot: selected, at: Date.now() };
      return selected;
    })
    .finally(() => {
      if (pending?.request === request) pending = undefined;
    });
  pending = { request, generation: started };
  return request;
}
