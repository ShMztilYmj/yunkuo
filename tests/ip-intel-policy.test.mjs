import test from "node:test";
import assert from "node:assert/strict";
import {
  abuseLevel,
  bandwidthLabel,
  bandwidthTier,
  blacklistView,
  deepChecks,
  nativeVerdict,
  peeringdbDetail,
  rpkiView,
  scenarioScores,
  threatHeadline,
  trafficSplit,
} from "../src/features/diagnostics/policies/ipIntelPolicy.ts";

/** 与参考页同一颗出口 IP 的情报（新加坡 FDCservers 机房） */
const intel = {
  ip: "50.7.253.170",
  state: "ok",
  error: null,
  asn: 30058,
  asnOrg: "FDCSERVERS",
  asnName: "FDCservers.net",
  isp: "FDCservers.net",
  org: "FDCservers.net",
  asnType: "hosting",
  networkClass: "dc",
  rir: "ARIN",
  rirCountry: "US",
  prefix: "50.7.252.0/23",
  asnIpv4Count: 374272,
  asnRegistered: "2003-07-10",
  isBogon: false,
  reverseDns: null,
  mobile: false,
  hosting: true,
  proxy: true,
  vpn: true,
  tor: false,
  proxyType: "VPN",
  riskScore: 66,
  rpki: "invalid",
  peeringdbTraffic: "5-10Tbps",
  peeringdbType: "Content",
  peeringdbRatio: "Heavy Outbound",
  peeringdbScope: "Global",
  peeringdbPrefixes4: 30000,
  peeringdbName: "FDCServers.Net",
  torExit: false,
  torSource: "torbulkexitlist",
  blacklistState: "clean",
  blacklists: [],
  blacklistNote: null,
  abuserScore: "0.0605 (High)",
  isAbuser: true,
  isCrawler: false,
  abuseEmail: "abuse@fdcservers.net",
  ipapiisState: "ok",
  sources: [],
};

test("人机流量：机房 + 高风险偏机器，住宅偏人类", () => {
  const hosting = trafficSplit(intel);
  assert.ok(hosting.bot > hosting.human);
  assert.equal(Math.round(hosting.human + hosting.bot), 100);

  const residential = trafficSplit({
    ...intel,
    networkClass: "res",
    hosting: false,
    riskScore: 8,
  });
  assert.ok(residential.human > residential.bot);
  assert.equal(trafficSplit(null), null);
});

test("场景评估：机房代理出口在 TikTok / 社媒上不推荐", () => {
  const rows = scenarioScores(intel);
  assert.deepEqual(
    rows.map((r) => `${r.key}:${r.score}`),
    ["tiktok:1", "social:1", "ai:2"],
  );
  assert.ok(rows.every((r) => r.score < 3));

  const clean = scenarioScores({
    ...intel,
    networkClass: "res",
    proxy: false,
    vpn: false,
    riskScore: 5,
  });
  assert.ok(clean.every((r) => r.score >= 3));

  // 情报缺失时不能因为「没命中」就给出满分
  const unknown = scenarioScores(null);
  assert.ok(unknown.every((r) => r.score === 0 && r.tone === "n"));
});

test("滥用等级按风险分分档，黑名单看实时查询结果", () => {
  assert.deepEqual(abuseLevel(66), { label: "中风险", tone: "y" });
  assert.deepEqual(abuseLevel(10), { label: "低风险", tone: "g" });
  assert.deepEqual(abuseLevel(95), { label: "高风险", tone: "r" });
  assert.deepEqual(abuseLevel(null), { label: "未知", tone: "n" });

  const listed = blacklistView({
    ...intel,
    blacklistState: "listed",
    blacklists: ["StopForumSpam", "dnsbl.dronebl.org"],
    blacklistNote: "频次 11，最近 2026-07-22",
  });
  assert.equal(listed.tone, "r");
  assert.match(listed.label, /StopForumSpam/);

  assert.deepEqual(
    blacklistView({ ...intel, blacklistState: "clean" }).label,
    "未命中",
  );
  // 查询源不可达时必须是「未查询」，不能因为没查到就说干净
  const unknown = blacklistView({ ...intel, blacklistState: "unavailable" });
  assert.equal(unknown.label, "未查询");
  assert.equal(unknown.tone, "n");
});

test("带宽优先用 PeeringDB 自报值，缺失才按 ASN 规模折算", () => {
  assert.equal(bandwidthTier(374272), "5-10 Tbps");
  assert.equal(bandwidthTier(20_000), "＜1 Tbps");
  assert.equal(bandwidthTier(120_000), "1-5 Tbps");
  assert.equal(bandwidthTier(2_000_000), "10 Tbps 以上");
  assert.equal(bandwidthTier(null), null);

  assert.equal(
    bandwidthLabel({ ...intel, peeringdbTraffic: "5-10Tbps" }),
    "5-10Tbps",
  );
  assert.equal(
    bandwidthLabel({ ...intel, peeringdbTraffic: null }),
    "5-10 Tbps",
  );
  assert.equal(bandwidthLabel(null), null);
  assert.match(peeringdbDetail(intel) ?? "", /Heavy Outbound/);
  assert.equal(
    peeringdbDetail({
      ...intel,
      peeringdbType: null,
      peeringdbRatio: null,
      peeringdbScope: null,
    }),
    null,
  );
});

test("IP 原生性：注册国与出口国不一致就是广播 IP", () => {
  assert.equal(nativeVerdict(intel, "SG").label, "广播 IP（美国）");
  assert.equal(
    nativeVerdict({ ...intel, rirCountry: "SG" }, "sg").label,
    "SG 原生",
  );
  assert.equal(nativeVerdict(null, "SG").label, "未知");
});

test("深度检测与威胁结论不把缺失数据说成“未检测到”", () => {
  const rows = deepChecks(intel);
  assert.equal(rows.find((r) => r.label === "VPN").text, "已检测到");
  assert.equal(rows.find((r) => r.label === "Tor").text, "未检测到");

  const unknown = deepChecks({ ...intel, vpn: null, proxy: null, tor: null });
  assert.equal(unknown.find((r) => r.label === "VPN").text, "未知");
  // ipapi.is 的爬虫旗标优先于流量占比推导
  assert.equal(
    deepChecks({ ...intel, isCrawler: true }).find(
      (r) => r.label === "爬虫 / 机器人",
    ).text,
    "已检测到",
  );
  assert.equal(threatHeadline(intel).tone, "y");
  assert.equal(threatHeadline(null).tone, "n");
  assert.deepEqual(rpkiView("invalid"), { label: "Invalid", tone: "r" });
  assert.deepEqual(rpkiView("unknown"), { label: "未登记 ROA", tone: "n" });
});
