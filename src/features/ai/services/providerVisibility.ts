/**
 * 「服务商与 Agent」里由用户移除的服务商。
 *
 * 模型源来自各工具自己的配置，本应用只读不写；这里的「移除」只落到界面状态：
 * 对照表与各 Agent 详情都不再显示这一项，工具内的配置保持不变。
 * 键与登记 id 同构（`agent:provider:id`），刷新与重启后仍指向同一项。
 */
const STORAGE_KEY = "yunkuo.ai.removed-providers.v1";
/** 只存键名；上限防止异常数据把 localStorage 撑满。 */
const MAX_ENTRIES = 200;

let removed: ReadonlySet<string> = load();

const listeners = new Set<() => void>();

function storage(): Storage | null {
  try {
    return typeof window === "undefined" ? null : window.localStorage;
  } catch {
    // 隐私模式等禁用存储的场景：只在本次会话内生效。
    return null;
  }
}

function load(): ReadonlySet<string> {
  try {
    const raw = storage()?.getItem(STORAGE_KEY);
    if (!raw) return new Set();
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return new Set();
    return new Set(
      parsed
        .filter((value): value is string => typeof value === "string")
        .slice(0, MAX_ENTRIES),
    );
  } catch {
    return new Set();
  }
}

function publish(next: ReadonlySet<string>) {
  removed = next;
  try {
    storage()?.setItem(STORAGE_KEY, JSON.stringify([...next]));
  } catch {
    // 写入失败时保留内存状态，至少本次会话内一致。
  }
  for (const listener of [...listeners]) listener();
}

/** 服务商条目的稳定身份：id 缺失时退回名称，避免同一列出现多个空键。 */
export function providerIdentity(provider: {
  id?: string;
  label?: string;
}): string {
  return provider.id || provider.label || "";
}

/** 与 providerAccounts 的登记 id 同构：`agent:provider:id`。 */
export function providerEntryKey(
  agentKey: string,
  provider: { id?: string; label?: string },
): string {
  return `${agentKey}:provider:${providerIdentity(provider)}`;
}

export function removedProviderKeys(): ReadonlySet<string> {
  return removed;
}

export function subscribeRemovedProviders(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** 各 Agent 详情与统计页共用同一份过滤：移除过的服务商不再出现在任何列表里。 */
export function visibleProviders<T extends { id?: string; label?: string }>(
  agentKey: string,
  providers: readonly T[],
  removedKeys: ReadonlySet<string> = removed,
): T[] {
  if (!removedKeys.size) return [...providers];
  return providers.filter(
    (provider) => !removedKeys.has(providerEntryKey(agentKey, provider)),
  );
}

export function removeProvider(key: string) {
  if (removed.has(key)) return;
  const next = new Set(removed);
  next.add(key);
  publish(next);
}

export function restoreProvider(key: string) {
  if (!removed.has(key)) return;
  const next = new Set(removed);
  next.delete(key);
  publish(next);
}

export function restoreAllProviders() {
  if (!removed.size) return;
  publish(new Set());
}
