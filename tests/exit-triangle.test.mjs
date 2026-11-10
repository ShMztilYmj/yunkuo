// 出口三角判定的口径：浏览器没走代理时，页面探测看到的是本地线路，
// 不能拿它去和「按出口同步过的系统时区」比，否则会误报时区矛盾。
import assert from "node:assert/strict";
import test from "node:test";

const { judgeTriangle } =
  await import("../src/features/diagnostics/signals/exit.ts");
const { computeTrust } =
  await import("../src/features/diagnostics/policies/trust.ts");

const geo = (over = {}) => ({
  ip: "203.0.113.10",
  cc: "US",
  country: "United States",
  city: "Chicago",
  asn: "",
  isp: "",
  tzId: "America/Chicago",
  src: "fixture",
  ...over,
});

const ctx = (over = {}) => ({
  cnTz: false,
  tz: "America/New_York",
  langCn: false,
  proxied: true,
  ...over,
});

test("走代理且时区对不上：黄色，提示对齐", () => {
  const result = judgeTriangle(geo(), ctx());
  assert.equal(result.cls, "y");
  assert.match(result.msg, /不一致/);
});

test("没走代理时同一份数据只做说明，不判矛盾", () => {
  const result = judgeTriangle(geo(), ctx({ proxied: false }));
  assert.equal(result.cls, "n");
  assert.match(result.msg, /没走代理/);
  assert.match(result.msg, /America\/Chicago/);
  assert.match(result.msg, /America\/New_York/);
});

test("没走代理且时区本来就一致：仍然是绿色", () => {
  const result = judgeTriangle(
    geo({ tzId: "America/New_York" }),
    ctx({ proxied: false }),
  );
  assert.equal(result.cls, "g");
});

test("出口落在中国：红色，优先于线路判断", () => {
  const result = judgeTriangle(
    geo({ cc: "CN", tzId: "Asia/Shanghai" }),
    ctx({ proxied: false }),
  );
  assert.equal(result.cls, "r");
});

test("系统时区是中国时区：红色身份矛盾", () => {
  const result = judgeTriangle(geo(), ctx({ cnTz: true, tz: "Asia/Shanghai" }));
  assert.equal(result.cls, "r");
  assert.match(result.msg, /身份矛盾/);
});

const exit = (over = {}) => ({
  state: "ok",
  primary: geo(),
  ip6: null,
  v6Verdict: "g",
  v6Msg: "",
  triangleVerdict: "n",
  triangleMsg: "浏览器当前没走代理：探测到的是本地线路。",
  asnClass: "res",
  at: 0,
  ...over,
});

const trustInput = (over = {}) => ({
  webTotal: 0,
  native: { countryAllowed: true, tzIsCn: false, timezoneMatches: true },
  exit: exit(over),
  dnsDeep: null,
  turnstile: null,
});

test("三角判定为 n 时不扣信任分，也不进问题清单", () => {
  const unknown = computeTrust(trustInput());
  const good = computeTrust(trustInput({ triangleVerdict: "g" }));
  assert.equal(unknown.risk, good.risk);
  assert.ok(!unknown.hits.some((hit) => hit.includes("没走代理")));
});

test("三角判定为 y / r 仍然扣分并记入问题清单", () => {
  const yellow = computeTrust(
    trustInput({ triangleVerdict: "y", triangleMsg: "时区不一致" }),
  );
  const red = computeTrust(
    trustInput({ triangleVerdict: "r", triangleMsg: "身份矛盾" }),
  );
  assert.ok(yellow.risk > 0);
  assert.ok(red.risk > yellow.risk);
  assert.ok(yellow.hits.includes("时区不一致"));
  assert.ok(red.hits.includes("身份矛盾"));
});
