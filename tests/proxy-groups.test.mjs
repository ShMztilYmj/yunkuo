import test from "node:test";
import assert from "node:assert/strict";
import { buildGroups, resolveChain } from "../src/features/proxy/grouping.ts";

const node = (overrides = {}) => ({
  protocol: "Vmess",
  selected: null,
  choices: [],
  ...overrides,
});

test("GLOBAL 置顶并改名为全局节点，链路解析到真实出口", () => {
  const groups = buildGroups({
    ChatGPT: node({
      protocol: "Selector",
      selected: "节点选择",
      choices: ["节点选择", "美国LA-优化-GPT"],
    }),
    GLOBAL: node({
      protocol: "Selector",
      selected: "节点选择",
      choices: ["节点选择", "日本-优化"],
    }),
    节点选择: node({
      protocol: "Selector",
      selected: "新加坡-优化2-GPT",
      choices: ["新加坡-优化2-GPT"],
    }),
    "新加坡-优化2-GPT": node(),
  });
  assert.deepEqual(
    groups.map((g) => g.id),
    ["GLOBAL", "ChatGPT", "节点选择"],
  );
  assert.equal(groups[0].label, "全局节点");
  assert.equal(groups[0].now, "节点选择");
  assert.deepEqual(groups[0].chain, ["节点选择", "新加坡-优化2-GPT"]);
  assert.equal(groups[0].global, true);
  assert.equal(groups[1].label, "ChatGPT");
  assert.equal(groups[1].global, false);
  // 普通组的链路不含自身。
  assert.deepEqual(groups[2].chain, ["新加坡-优化2-GPT"]);
});

test("节点、DIRECT 与空候选组都不出卡片", () => {
  const groups = buildGroups({
    GLOBAL: node({ protocol: "Selector", choices: ["DIRECT"] }),
    DIRECT: node({ protocol: "Direct", choices: [] }),
    空组: node({ protocol: "Selector", choices: [] }),
    节点: node({ protocol: "Vmess", choices: [] }),
  });
  assert.deepEqual(
    groups.map((g) => g.id),
    ["GLOBAL"],
  );
});

test("选择链遇到环或缺失就停，不会死循环", () => {
  const cyclic = {
    A: node({ protocol: "Selector", selected: "B", choices: ["B"] }),
    B: node({ protocol: "Selector", selected: "A", choices: ["A"] }),
  };
  assert.deepEqual(resolveChain(cyclic, "A"), ["A", "B"]);
  assert.deepEqual(
    resolveChain({ A: node({ protocol: "Selector", selected: "缺失", choices: ["缺失"] }) }, "A"),
    ["A"],
  );
  assert.deepEqual(resolveChain({}, "GLOBAL"), []);
});
