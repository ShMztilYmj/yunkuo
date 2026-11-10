import test from "node:test";
import assert from "node:assert/strict";
import {
  parseSelfcheckReport,
  selfcheckSignals,
  selfcheckSummary,
} from "../src/features/diagnostics/selfcheck/report.ts";
import { buildSelfcheckScript } from "../src/features/diagnostics/selfcheck/probe.ts";
import { SIGNAL_WEIGHTS } from "../src/features/diagnostics/signals/web.ts";

const weightOf = (id) =>
  SIGNAL_WEIGHTS.find((item) => item.id === id)?.weight ?? 0;

const UA =
  "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.7778.215 Safari/537.36";

/** 指纹浏览器（Windows 主机、en-US、纽约时区、无国产字体） */
const report = {
  at: 1_700_000_000_000,
  ua: UA,
  brands: "Chromium Google Chrome",
  platform: "Win32",
  languages: ["en-US", "en"],
  timezone: "America/New_York",
  locale: "en-US",
  offset: 300,
  hardwareConcurrency: 8,
  deviceMemory: 8,
  screen: [1920, 1080, 1],
  fonts: {
    sc: ["Microsoft YaHei", "SimSun"],
    tc: [],
    vendor: [],
  },
  emojiFlag: false,
  webgl: "ANGLE (NVIDIA)",
  webdriver: false,
};

test("回传数据缺项时按空值处理，不抛异常", () => {
  assert.equal(parseSelfcheckReport(null), null);
  assert.equal(parseSelfcheckReport("nope"), null);
  assert.equal(parseSelfcheckReport({ ua: UA }), null, "没有 at 视为无效回传");

  const sparse = parseSelfcheckReport({ at: 1, fonts: { sc: ["SimSun"] } });
  assert.ok(sparse);
  assert.deepEqual(sparse.fonts, { sc: ["SimSun"], tc: [], vendor: [] });
  assert.deepEqual(sparse.languages, []);
  assert.equal(sparse.emojiFlag, null);
  assert.equal(sparse.webdriver, false);
});

test("回传里的非字符串/非数字字段被丢弃", () => {
  const parsed = parseSelfcheckReport({
    at: 1,
    languages: ["en-US", 42, null, "en"],
    hardwareConcurrency: "8",
    emojiFlag: "false",
    screen: [1920, "x", 1],
  });
  assert.ok(parsed);
  assert.deepEqual(parsed.languages, ["en-US", "en"]);
  assert.equal(parsed.hardwareConcurrency, null);
  assert.equal(parsed.emojiFlag, null);
  assert.deepEqual(parsed.screen, [1920, 1]);
});

test("两栏用同一套计分：字体命中直接换算成分数", () => {
  const parsed = parseSelfcheckReport(report);
  assert.ok(parsed);
  const rows = selfcheckSignals(parsed);
  assert.deepEqual(
    rows.map((row) => row.id),
    ["fonts", "vendorFonts", "cnBrowser", "deviceVendor", "emoji"],
  );
  const fonts = rows[0];
  assert.equal(fonts.name, "中文字体");
  assert.equal(fonts.score, Math.min(1, 0.75 + 0.08 * 2));
  assert.equal(fonts.contribution, Math.round(fonts.score * weightOf("fonts")));
  assert.equal(rows[1].contribution, 0);
  assert.equal(rows[2].raw, "未检测到");
  assert.equal(rows[3].raw, "未检测到");
});

test("Windows 风格 Emoji：国旗不渲染按 0.4 计", () => {
  const parsed = parseSelfcheckReport(report);
  assert.ok(parsed);
  const emoji = selfcheckSignals(parsed).find((row) => row.id === "emoji");
  assert.ok(emoji);
  assert.equal(emoji.raw, "Microsoft 风格 · 国旗不渲染");
  assert.equal(emoji.score, 0.4);
  assert.equal(emoji.contribution, 1);
});

test("摘要给出语言、时区、核数与引擎版本", () => {
  const parsed = parseSelfcheckReport(report);
  assert.ok(parsed);
  assert.equal(
    selfcheckSummary(parsed),
    "en-US · America/New_York · 8 核 · Chrome 148",
  );
});

test("下发的采集脚本是合法 JS，且不会截断宿主页面", () => {
  const script = buildSelfcheckScript();
  assert.ok(!script.includes("</script"), "脚本不能包含结束标签");
  assert.doesNotThrow(() => new Function(script), "序列化后的函数体必须能解析");
  assert.match(script, /function|=>/, "字体探针应以函数体形式内联");
  assert.match(script, /Microsoft YaHei/, "字体清单随脚本一起下发");
});
