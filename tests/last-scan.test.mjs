import test from "node:test";
import assert from "node:assert/strict";
import {
  HISTORY_FRESH_MS,
  relativeTime,
  staleReason,
  systemChanged,
} from "../src/features/diagnostics/lastScan.ts";

/** 与 last-scan.json 同形：只含展示字段，没有 finished / native */
const last = {
  at: 1_700_000_000_000,
  score: 95,
  failures: ["TUN 未实际开启"],
  system: {
    timezone: "Central Standard Time",
    languages: ["en-US", "en"],
    culture: "en-US",
  },
  configChanged: false,
};

const now = { ...last.system };
const at = last.at + 60_000;

test("配置变了优先于系统与时间原因", () => {
  assert.equal(
    staleReason(
      { ...last, configChanged: true },
      { ...now, timezone: "Tokyo Standard Time" },
      at + HISTORY_FRESH_MS,
    ),
    "config",
  );
});

test("系统时区、区域格式、首选语言任一变化都算系统已变", () => {
  assert.equal(
    staleReason(last, { ...now, timezone: "Tokyo Standard Time" }, at),
    "system",
  );
  assert.equal(staleReason(last, { ...now, culture: "en-GB" }, at), "system");
  assert.equal(
    staleReason(last, { ...now, languages: ["en-GB", "en"] }, at),
    "system",
  );
  assert.equal(staleReason(last, now, at), null);
});

test("拿不到当前系统事实时不判定系统已变", () => {
  assert.equal(systemChanged(last.system, null), false);
  assert.equal(staleReason(last, null, at), null);
});

test("历史里没存到的系统字段不参与比较", () => {
  const partial = {
    ...last,
    system: { timezone: "Central Standard Time", languages: [], culture: "" },
  };
  assert.equal(staleReason(partial, { ...now, culture: "en-GB" }, at), null);
});

test("超过 6 小时只提示时间较久，不隐藏数据", () => {
  assert.equal(staleReason(last, now, last.at + HISTORY_FRESH_MS), null);
  assert.equal(staleReason(last, now, last.at + HISTORY_FRESH_MS + 1), "age");
});

test("相对时间按分钟、小时、天分档", () => {
  assert.equal(relativeTime(at, at), "刚刚");
  assert.equal(relativeTime(at - 59_000, at), "刚刚");
  assert.equal(relativeTime(at - 5 * 60_000, at), "5 分钟前");
  assert.equal(relativeTime(at - 3 * 3600_000, at), "3 小时前");
  assert.equal(relativeTime(at - 2 * 86400_000, at), "2 天前");
});

test("未来时间不出现负数", () => {
  assert.equal(relativeTime(at + 60_000, at), "刚刚");
});
