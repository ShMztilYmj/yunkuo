import assert from "node:assert/strict";
import test from "node:test";
import {
  DELAY_TTL_MS,
  filterNodes,
  freshDelay,
  presentRegions,
  pruneDelays,
  regionOf,
  sortNodes,
} from "../src/features/proxy/node-view.ts";

const NOW = 1_700_000_000_000;
const sample = (delay, age = 0) => ({ delay, at: NOW - age });

test("地区按名称里的中英文关键词归类，猜不出归入其他", () => {
  const cases = [
    ["🇭🇰 香港 01", "HK"],
    ["HK-02", "HK"],
    ["台湾 Hinet", "TW"],
    ["JP-东京-01", "JP"],
    ["新加坡 03", "SG"],
    ["US-01", "US"],
    ["United States 04", "US"],
    ["KR 首尔", "KR"],
    ["UK-London", "GB"],
    ["德国 法兰克福", "DE"],
    ["Canada-Toronto", "CA"],
    ["AU Sydney", "AU"],
    ["RUS-01", "OTHER"],
    ["备用线路", "OTHER"],
  ];
  for (const [name, expected] of cases) {
    assert.equal(regionOf(name), expected, name);
  }
});

test("短地区码不会误伤包含它的单词", () => {
  // US 出现在 RUS / AUS 里，SG 出现在 ASGARD 里，都不应被判成对应地区。
  assert.equal(regionOf("AUS-Melbourne"), "AU");
  assert.equal(regionOf("SG-01"), "SG");
  assert.equal(regionOf("ASGARD"), "OTHER");
});

test("只有出现过的地区才进筛选器，顺序固定", () => {
  const regions = presentRegions(["US-01", "香港 01", "US-02", "未知"]);
  assert.deepEqual(
    regions.map((region) => region.key),
    ["HK", "US", "OTHER"],
  );
});

test("延迟结果超过 TTL 视为未测，0 与负数同理", () => {
  const map = {
    fresh: sample(180, 60_000),
    stale: sample(120, DELAY_TTL_MS + 1),
    zero: sample(0),
    negative: sample(-5),
  };
  assert.equal(freshDelay(map, "fresh", NOW), 180);
  assert.equal(freshDelay(map, "stale", NOW), null);
  assert.equal(freshDelay(map, "zero", NOW), null);
  assert.equal(freshDelay(map, "negative", NOW), null);
  assert.equal(freshDelay(map, "missing", NOW), null);
});

test("过期样本在保存前被清掉，避免存储无限增长", () => {
  const pruned = pruneDelays(
    { keep: sample(90, 1_000), drop: sample(90, DELAY_TTL_MS + 1) },
    NOW,
  );
  assert.deepEqual(Object.keys(pruned), ["keep"]);
});

test("延迟排序把未测与过期节点放到最后，且保持组内相对顺序", () => {
  const delays = {
    slow: sample(420),
    fast: sample(60),
    stale: sample(30, DELAY_TTL_MS + 1),
  };
  const names = ["stale", "slow", "unknown", "fast"];
  assert.deepEqual(sortNodes(names, delays, "delay", NOW), [
    "fast",
    "slow",
    "stale",
    "unknown",
  ]);
  assert.deepEqual(names, ["stale", "slow", "unknown", "fast"]);
});

test("配置顺序与名称排序不改动原始数组", () => {
  const names = ["b", "a", "c"];
  assert.deepEqual(sortNodes(names, {}, "default", NOW), ["b", "a", "c"]);
  assert.deepEqual(sortNodes(names, {}, "name", NOW), ["a", "b", "c"]);
  assert.deepEqual(names, ["b", "a", "c"]);
});

test("地区筛选只保留匹配节点，全部地区原样返回", () => {
  const names = ["US-01", "香港 01", "JP-01"];
  assert.deepEqual(filterNodes(names, "all"), names);
  assert.deepEqual(filterNodes(names, "HK"), ["香港 01"]);
  assert.deepEqual(filterNodes(names, "DE"), []);
});
