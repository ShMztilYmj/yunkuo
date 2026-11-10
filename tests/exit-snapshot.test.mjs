import test from "node:test";
import assert from "node:assert/strict";
import {
  EXIT_SNAPSHOT_TTL_MS,
  clearExitSnapshot,
  normalizeExit,
  normalizeIntel,
  parseExitSnapshot,
  readExitSnapshot,
  saveExitSnapshot,
} from "../src/features/diagnostics/exitSnapshot.ts";

const at = 1_700_000_000_000;

/** 与 ip_intel 命令同形的一颗住宅出口（Comcast，US） */
const intel = {
  ip: "48.46.43.197",
  state: "ok",
  error: null,
  asn: 7922,
  asnOrg: "COMCAST-7922",
  asnName: "Comcast Cable Communications, LLC",
  isp: "Comcast Cable Communications, LLC",
  org: "Comcast Cable Communications, LLC",
  asnType: "isp",
  networkClass: "res",
  rir: "ARIN",
  rirCountry: "US",
  prefix: "48.46.42.0/23",
  asnIpv4Count: 68_337_408,
  asnRegistered: "1997-02-14",
  isBogon: false,
  reverseDns: null,
  mobile: false,
  hosting: false,
  proxy: false,
  vpn: false,
  tor: false,
  proxyType: null,
  riskScore: 0,
  rpki: "valid",
  peeringdbTraffic: "1-5Tbps",
  peeringdbType: "Cable/DSL/ISP",
  peeringdbRatio: "Balanced",
  peeringdbScope: "Regional",
  peeringdbPrefixes4: 1100,
  peeringdbName: "Comcast",
  torExit: false,
  torSource: "torbulkexitlist",
  blacklistState: "clean",
  blacklists: [],
  blacklistNote: null,
  abuserScore: null,
  isAbuser: false,
  isCrawler: false,
  abuseEmail: "abuse@comcast.net",
  ipapiisState: "off",
  sources: ["rdap.arin.net", "peeringdb.com"],
};

const exit = {
  state: "ok",
  primary: {
    ip: "48.46.43.197",
    cc: "us",
    country: "United States",
    city: "Dallas",
    asn: "AS7922",
    isp: "Comcast Cable Communications, LLC",
    tzId: "America/Chicago",
    src: "ipwho.is",
  },
  ip6: null,
  v6Verdict: "g",
  v6Msg: "无公网 IPv6 暴露",
  triangleVerdict: "y",
  triangleMsg: "IP 反推时区与系统时区不一致",
  asnClass: "res",
  at,
};

const snapshot = {
  v: 1,
  at,
  ip: "48.46.43.197",
  countryAllowed: true,
  exit,
  intel,
};

test("快照能读回，国家码统一大写", () => {
  const parsed = parseExitSnapshot(snapshot, at + 60_000);
  assert.ok(parsed);
  assert.equal(parsed.ip, "48.46.43.197");
  assert.equal(parsed.countryAllowed, true);
  assert.equal(parsed.exit.primary.cc, "US");
  assert.equal(parsed.intel.asn, 7922);
  assert.equal(parsed.intel.networkClass, "res");
});

test("超过一天或来自未来都不算可用快照", () => {
  assert.ok(parseExitSnapshot(snapshot, at + EXIT_SNAPSHOT_TTL_MS));
  assert.equal(
    parseExitSnapshot(snapshot, at + EXIT_SNAPSHOT_TTL_MS + 1),
    null,
    "过期快照不能冒充当前出口",
  );
  assert.equal(
    parseExitSnapshot({ ...snapshot, at: at + 6 * 60_000 }, at),
    null,
    "本地时间被调早时不认未来快照",
  );
});

test("版本、IP 对不上就整条丢弃", () => {
  assert.equal(parseExitSnapshot({ ...snapshot, v: 2 }, at + 60_000), null);
  assert.equal(
    parseExitSnapshot({ ...snapshot, ip: "1.1.1.1" }, at + 60_000),
    null,
    "情报 IP 与快照 IP 必须一致",
  );
  assert.equal(
    parseExitSnapshot(
      {
        ...snapshot,
        exit: { ...exit, primary: { ...exit.primary, ip: "1.1.1.1" } },
      },
      at + 60_000,
    ),
    null,
    "出口地理库的 IP 也要与快照一致",
  );
  assert.equal(
    parseExitSnapshot({ ...snapshot, exit: null, intel: null }, at + 60_000),
    null,
  );
});

test("坏字段逐项退化成 null，不让 UI 读到字符串数字", () => {
  const dirty = normalizeIntel({
    ip: "1.2.3.4",
    state: "weird",
    riskScore: "high",
    asn: "7922",
    blacklists: "StopForumSpam",
    networkClass: "ddos",
    rpki: "maybe",
    sources: ["a", 2, ""],
  });
  assert.ok(dirty);
  assert.equal(dirty.state, "partial");
  assert.equal(dirty.riskScore, null);
  assert.equal(dirty.asn, null);
  assert.deepEqual(dirty.blacklists, []);
  assert.equal(dirty.networkClass, null);
  assert.equal(dirty.rpki, null);
  assert.deepEqual(dirty.sources, ["a"]);
  assert.equal(normalizeIntel({ ip: "" }), null);
});

test("出口层的未知判定回落到 n / unknown", () => {
  const dirty = normalizeExit({
    state: "ok",
    primary: null,
    v6Verdict: "leak",
    triangleVerdict: 1,
    asnClass: "???",
    at,
  });
  assert.ok(dirty);
  assert.equal(dirty.v6Verdict, "n");
  assert.equal(dirty.triangleVerdict, "n");
  assert.equal(dirty.asnClass, "unknown");
  assert.equal(dirty.primary, null);
  assert.equal(normalizeExit({ state: "boom", at }), null);
});

test("没有 WebView 存储时读写都不抛错，只是不缓存", () => {
  assert.equal(readExitSnapshot(), null);
  assert.equal(
    saveExitSnapshot({
      ip: "48.46.43.197",
      countryAllowed: true,
      exit,
      intel,
    }),
    null,
    "Node 环境没有 localStorage，写入应安静失败",
  );
  clearExitSnapshot();
});
