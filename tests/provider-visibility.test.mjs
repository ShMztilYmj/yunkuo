/**
 * 移除清单：只影响界面展示，工具内的配置不动。
 * 这里用假的 localStorage 验证「移除 → 各视图过滤 → 撤销/持久化」这条链路。
 */
import test from "node:test";
import assert from "node:assert/strict";

const memory = new Map();
globalThis.window = {
  localStorage: {
    getItem: (key) => (memory.has(key) ? memory.get(key) : null),
    setItem: (key, value) => void memory.set(key, String(value)),
  },
};

const {
  providerEntryKey,
  removedProviderKeys,
  removeProvider,
  restoreAllProviders,
  restoreProvider,
  subscribeRemovedProviders,
  visibleProviders,
} = await import("../src/features/ai/services/providerVisibility.ts");

const STORAGE_KEY = "yunkuo.ai.removed-providers.v1";

test("移除后只过滤该 Agent 的这一项，键与登记 id 同构", () => {
  const providers = [
    { id: "glm02", label: "GLM02" },
    { id: "glm01", label: "GLM01" },
  ];
  const key = providerEntryKey("glm", providers[0]);
  assert.equal(key, "glm:provider:glm02");

  removeProvider(key);
  assert.deepEqual(
    visibleProviders("glm", providers).map((provider) => provider.id),
    ["glm01"],
  );
  // 同一个服务商 id 出现在别的 Agent 列时不受影响。
  assert.equal(visibleProviders("deepseek", providers).length, 2);
  assert.deepEqual(JSON.parse(memory.get(STORAGE_KEY)), [key]);
});

test("撤销与全部恢复都会让订阅方重算并回写存储", () => {
  let notified = 0;
  const unsubscribe = subscribeRemovedProviders(() => {
    notified += 1;
  });
  const key = providerEntryKey("glm", { id: "glm01" });
  removeProvider(key);
  assert.equal(notified, 1);
  assert.equal(removedProviderKeys().size, 2);
  assert.deepEqual(JSON.parse(memory.get(STORAGE_KEY)), [
    "glm:provider:glm02",
    key,
  ]);

  restoreProvider(key);
  assert.equal(notified, 2);
  assert.deepEqual(JSON.parse(memory.get(STORAGE_KEY)), ["glm:provider:glm02"]);

  restoreAllProviders();
  assert.equal(notified, 3);
  assert.equal(removedProviderKeys().size, 0);
  assert.deepEqual(JSON.parse(memory.get(STORAGE_KEY)), []);

  // 重复操作不发通知，也不产生多余写入。
  restoreProvider(key);
  restoreAllProviders();
  assert.equal(notified, 3);
  unsubscribe();
});

test("身份缺失的服务商退回名称，空名称不产生可移除键", () => {
  assert.equal(providerEntryKey("glm", { label: "GLM" }), "glm:provider:GLM");
  assert.equal(providerEntryKey("glm", { id: "x" }), "glm:provider:x");
  assert.equal(visibleProviders("glm", [{ id: "", label: "" }]).length, 1);
});
