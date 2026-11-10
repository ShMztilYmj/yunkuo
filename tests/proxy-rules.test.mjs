import test from "node:test";
import assert from "node:assert/strict";
import {
  isFallbackRule,
  missingRulesNotice,
  summarizeRules,
} from "../src/features/proxy/rule-summary.ts";

const rule = (kind, target, pattern = "") => ({
  kind,
  pattern,
  target,
  size: 0,
});

test("单节点串只有一条兜底 Match 时算没有规则", () => {
  const overview = summarizeRules([rule("Match", "节点选择")]);
  assert.equal(overview.total, 1);
  assert.equal(overview.routing, 0);
  assert.equal(overview.onlyFallback, true);
  assert.equal(overview.empty, false);
  assert.equal(overview.missing, true);
  assert.equal(overview.fallbackTarget, "节点选择");
});

test("空 rules 段也算没有规则，且没有兜底策略", () => {
  const overview = summarizeRules([]);
  assert.equal(overview.empty, true);
  assert.equal(overview.missing, true);
  assert.equal(overview.onlyFallback, false);
  assert.equal(overview.fallbackTarget, null);
});

test("带条件的分流规则不会被误判为没有规则", () => {
  const overview = summarizeRules([
    rule("DomainSuffix", "节点选择", "claude.ai"),
    rule("GeoIP", "DIRECT", "CN"),
    rule("Match", "节点选择"),
  ]);
  assert.equal(overview.total, 3);
  assert.equal(overview.routing, 2);
  assert.equal(overview.onlyFallback, false);
  assert.equal(overview.missing, false);
  assert.equal(overview.fallbackTarget, "节点选择");
});

test("兜底类型的大小写和空白不影响识别", () => {
  assert.equal(isFallbackRule(rule(" MATCH ", "REJECT")), true);
  assert.equal(isFallbackRule(rule("Match", "REJECT")), true);
  assert.equal(isFallbackRule(rule("Domain", "REJECT", "a.test")), false);
});

test("没有规则时提示说明规则模式等同全局模式", () => {
  const notice = missingRulesNotice(
    summarizeRules([rule("Match", "节点选择")]),
  );
  assert.equal(notice.title, "当前订阅没有规则");
  assert.match(notice.detail, /节点选择/);
  assert.match(notice.detail, /规则模式 ≈ 全局模式/);
  assert.match(notice.detail, /日常分流/);
});

test("没有 rules 段时也提示没有规则，且不带兜底策略名", () => {
  const notice = missingRulesNotice(summarizeRules([]));
  assert.equal(notice.title, "当前订阅没有规则");
  assert.doesNotMatch(notice.detail, /→/);
});

test("存在分流规则时不显示提示", () => {
  assert.equal(
    missingRulesNotice(
      summarizeRules([
        rule("GeoIP", "DIRECT", "CN"),
        rule("Match", "节点选择"),
      ]),
    ),
    null,
  );
});
