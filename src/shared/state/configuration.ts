import type { AppConfig } from "../../services/api/config";

/** 白名单也校验动态对象，避免模块无意修改其他模块的字段。 */
export function configurationDomain<
  const Keys extends readonly (keyof AppConfig)[],
  const Writable extends readonly Keys[number][],
>(keys: Keys, writable: Writable) {
  type View = Pick<AppConfig, Keys[number]>;
  type Patch = Partial<Pick<AppConfig, Writable[number]>>;
  const allowed = new Set<string>(writable);
  return {
    view(config: AppConfig | null): View | null {
      if (!config) return null;
      return Object.fromEntries(keys.map((key) => [key, config[key]])) as View;
    },
    patch(patch: Patch): Patch {
      for (const key of Object.keys(patch)) {
        if (!allowed.has(key))
          throw new Error(`配置字段不属于当前模块：${key}`);
      }
      return structuredClone(patch);
    },
  };
}
