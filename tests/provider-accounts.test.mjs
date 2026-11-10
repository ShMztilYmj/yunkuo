import test from "node:test";
import assert from "node:assert/strict";
import {
  activeProviderOf,
  balanceLabel,
  buildAgentColumns,
  columnRows,
  officialProvider,
  providerFrom,
  providerQuotaView,
  quotaSummary,
  quotaSyncText,
  registeredCount,
  summarizeColumns,
  toolQuotaView,
  withoutRemovedProviders,
} from "../src/features/ai/services/providerAccounts.ts";

const meta = (key, label = key) => ({
  key,
  label,
  short: label,
  hint: `${label} 的说明`,
});

const account = (overrides = {}) => ({
  email: null,
  name: null,
  plan: null,
  planLabel: null,
  workspace: null,
  billingType: null,
  switchable: false,
  ...overrides,
});

const item = (key, overrides = {}) => ({
  key,
  account: account(),
  providers: [],
  ...overrides,
});

const entry = (source, endpoint = null, key = "claude") => ({
  key,
  installed: true,
  running: false,
  configured: true,
  shortcutValid: true,
  candidates: [],
  connection: { key, model: null, source, endpoint, note: "" },
});

const provider = (overrides = {}) => ({
  id: "provider",
  label: "服务商",
  kind: "balance",
  status: "ok",
  note: "",
  windows: [],
  balances: [],
  models: [],
  capturedAt: 0,
  ...overrides,
});

test("连接方式决定服务商名称：官方直连、自定义接口与未读取", () => {
  assert.equal(
    providerFrom(entry("native"), "Anthropic 官方").name,
    "Anthropic 官方",
  );
  assert.equal(
    providerFrom(
      entry("external", "https://api.example.com:8443"),
      "Anthropic 官方",
    ).name,
    "自定义接口 · api.example.com:8443",
  );
  assert.equal(
    providerFrom(undefined, "Anthropic 官方").name,
    "连接配置未读取",
  );
  assert.equal(
    providerFrom(entry("tool"), "OpenAI 官方").name,
    "由工具自身管理",
  );
});

test("Claude 列：当前登录 + 身份备份，按服务商与账号去重", () => {
  const [column] = buildAgentColumns([
    {
      meta: meta("claude", "Claude Code"),
      item: item("claude", {
        account: account({ email: "me@example.com", planLabel: "Max" }),
      }),
      entry: entry("native"),
      claudeIdentities: [
        { name: "工作账号", email: "work@example.com" },
        { name: "me@example.com", email: "me@example.com" },
      ],
    },
  ]);
  assert.equal(column.bindings.length, 2);
  assert.deepEqual(
    column.bindings.map((binding) => [binding.account, binding.state]),
    [
      ["me@example.com", "active"],
      ["工作账号", "configured"],
    ],
  );
  assert.match(column.bindings[0].detail, /Max/);
  assert.equal(registeredCount(column), 2);
});

test("Claude 列：没有登录信息时给出未登录占位而不是伪造账号", () => {
  const [column] = buildAgentColumns([
    { meta: meta("claude"), item: item("claude"), entry: entry("native") },
  ]);
  assert.equal(column.bindings.length, 1);
  assert.equal(column.bindings[0].state, "empty");
  assert.equal(column.bindings[0].account, "尚未登录");
  assert.equal(column.bindings[0].placeholder, true);
  assert.equal(registeredCount(column), 0);
});

test("Codex 列：当前账号优先，旧版快照标为待迁移", () => {
  const [column] = buildAgentColumns([
    {
      meta: meta("chatgpt", "Codex"),
      item: item("chatgpt", { account: account({ email: "me@example.com" }) }),
      entry: entry("external", "https://relay.example.com", "chatgpt"),
      codexAccounts: [
        {
          legacy: false,
          which: "a",
          email: "me@example.com",
          label: "me@example.com",
        },
        {
          legacy: true,
          which: "b",
          email: "old@example.com",
          label: "old@example.com",
        },
      ],
    },
  ]);
  assert.equal(column.bindings.length, 2);
  assert.equal(column.bindings[0].state, "active");
  assert.equal(column.bindings[0].provider, "自定义接口 · relay.example.com");
  assert.equal(column.bindings[1].state, "error");
  assert.match(column.bindings[1].detail, /加密迁移/);
});

test("模型源列：额度状态映射为账号状态，失败不等于未配置", () => {
  const [column] = buildAgentColumns([
    {
      meta: meta("deepseek", "DSH"),
      item: item("deepseek", {
        providers: [
          provider({
            id: "ds",
            label: "DeepSeek 官方",
            kind: "balance",
            models: ["deepseek-v4"],
          }),
          provider({
            id: "oc",
            label: "OpenCode Go",
            kind: "subscription",
            status: "unconfigured",
            note: "未找到 OPENCODE_GO_API_KEY",
          }),
          provider({
            id: "relay",
            label: "中转站",
            kind: "subscription",
            status: "error",
            note: "密钥无效或已过期",
          }),
          provider({
            id: "other",
            label: "其他",
            kind: "unsupported",
            status: "unsupported",
          }),
        ],
      }),
    },
  ]);
  assert.deepEqual(
    column.bindings.map((binding) => [binding.account, binding.state]),
    [
      ["按量付费", "active"],
      ["未配置凭据", "empty"],
      ["密钥或网络异常", "error"],
      ["由服务商控制台管理", "managed"],
    ],
  );
  assert.equal(column.bindings[0].detail, "deepseek-v4");
  assert.equal(column.bindings[1].detail, "未找到 OPENCODE_GO_API_KEY");
  assert.equal(registeredCount(column), 3);
  // 暂不支持查询的服务商仍算已配置；只有真正有凭据的条目才算账号。
  assert.deepEqual(summarizeColumns([column]), {
    agents: 1,
    providers: 4,
    accounts: 2,
  });
});

test("没有模型源时只给占位，不猜服务商；工具自管列不计入汇总", () => {
  const columns = buildAgentColumns([
    { meta: meta("glm", "ZCode"), item: item("glm") },
    { meta: meta("cursor", "Cursor"), item: item("cursor") },
  ]);
  assert.equal(columns[0].bindings[0].account, "未读取到服务商");
  assert.equal(columns[0].bindings[0].state, "empty");
  assert.equal(columns[1].bindings[0].provider, "由工具自身管理");
  assert.equal(registeredCount(columns[1]), 0);
  assert.deepEqual(summarizeColumns(columns), {
    agents: 2,
    providers: 0,
    accounts: 0,
  });
  assert.equal(columnRows(columns), 1);
});

test("只有官方账号只读：用户添加的模型源都能移除", () => {
  const [dsh, zcode] = buildAgentColumns([
    {
      meta: meta("deepseek", "DSH"),
      item: item("deepseek", {
        providers: [
          provider({
            id: "deepseek-account-platform",
            label: "DeepSeek 官方",
            kind: "balance",
          }),
          provider({
            id: "opencode-go",
            label: "OpenCode Go",
            kind: "subscription",
          }),
          provider({
            id: "relay",
            label: "自建中转",
            kind: "subscription",
            status: "error",
          }),
        ],
      }),
    },
    {
      meta: meta("glm", "ZCode"),
      item: item("glm", {
        providers: [
          provider({
            id: "glm02",
            label: "GLM02",
            kind: "unsupported",
            status: "unsupported",
          }),
        ],
      }),
    },
  ]);
  assert.deepEqual(
    dsh.bindings.map((binding) => [binding.id, binding.deletable]),
    [
      ["deepseek:provider:deepseek-account-platform", false],
      ["deepseek:provider:opencode-go", true],
      ["deepseek:provider:relay", true],
    ],
  );
  assert.equal(zcode.bindings[0].id, "glm:provider:glm02");
  assert.equal(zcode.bindings[0].deletable, true);
  // 官方判定与展示解耦：余额制与名称带「官方」都算官方来源。
  assert.equal(
    officialProvider(provider({ id: "any", label: "某平台" })),
    true,
    "余额制默认是官方账号",
  );
  assert.equal(
    officialProvider(
      provider({ id: "glm", label: "GLM", kind: "unsupported" }),
    ),
    false,
  );
});

test("账号条目与占位行没有移除入口", () => {
  const columns = buildAgentColumns([
    {
      meta: meta("claude", "Claude Code"),
      item: item("claude", { account: account({ email: "me@example.com" }) }),
      entry: entry("external", "https://relay.example.com"),
    },
    { meta: meta("cursor", "Cursor"), item: item("cursor") },
  ]);
  assert.deepEqual(
    columns.map((column) =>
      column.bindings.map((binding) => binding.deletable),
    ),
    [[false], [false]],
  );
});

test("移除后的服务商不再进入对照表，整列移除时保留空列", () => {
  const columns = buildAgentColumns([
    {
      meta: meta("glm", "ZCode"),
      item: item("glm", {
        providers: [
          provider({
            id: "glm02",
            label: "GLM02",
            kind: "unsupported",
            status: "unsupported",
          }),
          provider({
            id: "deepseek-account-platform",
            label: "DeepSeek 官方",
            kind: "balance",
          }),
        ],
      }),
    },
  ]);
  const hidden = new Set(["glm:provider:glm02"]);
  const next = withoutRemovedProviders(columns, hidden);
  assert.deepEqual(
    next[0].bindings.map((binding) => binding.id),
    ["glm:provider:deepseek-account-platform"],
  );
  // 官方来源即使出现在移除清单里也不隐藏（清单只由可移除条目写入）。
  const official = withoutRemovedProviders(
    columns,
    new Set(["glm:provider:deepseek-account-platform"]),
  );
  assert.equal(official[0].bindings.length, 2);
  // 全部移除后不退化成「未读取到服务商」占位。
  const empty = withoutRemovedProviders(columns, new Set([...hidden, "x"]));
  assert.equal(empty[0].bindings.length, 1);
  assert.equal(withoutRemovedProviders(columns, new Set()).length, 1);
});

test("额度与使用中：账号额度来自官方快照，模型源额度跟随同步状态", () => {
  const [claude, zcode] = buildAgentColumns([
    {
      meta: meta("claude", "Claude Code"),
      item: item("claude", {
        account: account({ email: "me@example.com" }),
        scannedAt: 1_800_000_000,
        rate: {
          metrics: [],
          primary: {
            utilization: 0.42,
            limitWindowSeconds: 604800,
            resetAfterSeconds: 0,
            resetAt: 1_800_100_000,
          },
          secondary: null,
          capturedAt: 1_800_000_000,
          accountVerified: true,
        },
      }),
      entry: entry("native"),
    },
    {
      meta: meta("glm", "ZCode"),
      item: item("glm", {
        providerSync: {
          live: false,
          intervalSeconds: 60,
          lastSyncedAt: 1_800_000_000,
        },
        providers: [
          provider({
            id: "glm02",
            label: "GLM02",
            kind: "unsupported",
            status: "unsynced",
            capturedAt: 0,
          }),
          provider({
            id: "plan",
            label: "BigModel",
            kind: "plan",
            status: "ok",
            capturedAt: 1_800_000_000,
            active: true,
            windows: [
              { label: "每周", usedPercent: 77, resetsAt: 1_800_100_000 },
            ],
          }),
        ],
      }),
    },
  ]);

  const current = claude.bindings[0];
  assert.equal(current.active, true, "当前登录账号就是正在使用的");
  assert.equal(current.quota.from, "account");
  assert.equal(current.quota.billing, "subscription");
  assert.deepEqual(
    current.quota.windows.map((row) => [
      row.label,
      Math.round(row.usedPercent),
    ]),
    [["每周", 42]],
  );
  assert.equal(current.quota.sync, null, "账号额度不参与 Agent 进程同步");

  const [unsynced, plan] = zcode.bindings;
  assert.equal(unsynced.state, "unsynced");
  assert.equal(unsynced.account, "等待首次同步");
  assert.equal(unsynced.quota.capturedAt, 0);
  assert.deepEqual(unsynced.quota.sync, {
    live: false,
    intervalSeconds: 60,
    lastSyncedAt: 1_800_000_000,
  });
  assert.equal(plan.state, "active");
  assert.equal(plan.active, true, "默认/已选服务商显示使用中");
  assert.equal(plan.quota.windows[0].usedPercent, 77);
  assert.equal(plan.quota.billing, "plan", "编程套餐不属于订阅窗口");
});

test("按量付费只展示余额：把充值余额叫作余额，不出百分比窗口", () => {
  const quota = {
    windows: [],
    balances: [
      { label: "充值余额", amount: "36.80", currency: "CNY" },
      { label: "赠金", amount: "5.00", currency: "CNY" },
    ],
    capturedAt: 1_800_000_000,
    billing: "balance",
    from: "provider",
    sync: { live: true, intervalSeconds: 60, lastSyncedAt: 1_800_000_000 },
  };
  assert.equal(balanceLabel("充值余额"), "余额");
  assert.equal(balanceLabel("赠金"), "赠金");
  assert.equal(quotaSummary(quota), "余额 ¥36.80 · 赠金 ¥5.00");
  assert.equal(
    quotaSummary({ ...quota, balances: [] }),
    null,
    "没有余额数据时不出空摘要",
  );
});

test("同步状态文案区分实时、暂停与尚未同步", () => {
  const base = {
    windows: [],
    balances: [],
    from: "provider",
    sync: { live: true, intervalSeconds: 60, lastSyncedAt: 1_800_000_000 },
  };
  assert.match(
    quotaSyncText({ ...base, capturedAt: 1_800_000_000 }),
    /^实时同步 · 每 60 秒 · 最近 /,
  );
  assert.match(
    quotaSyncText({
      ...base,
      capturedAt: 1_800_000_000,
      sync: { ...base.sync, live: false },
    }),
    /同步已暂停 · 最后同步 /,
  );
  assert.match(
    quotaSyncText({
      ...base,
      capturedAt: 0,
      sync: { ...base.sync, live: false },
    }),
    /^尚未同步/,
  );
  assert.match(
    quotaSyncText({
      windows: [],
      balances: [],
      from: "account",
      capturedAt: 1_800_000_000,
      sync: null,
    }),
    /^官方额度快照 · 采集于 /,
  );
});

test("悬浮条只给当前服务商：标记优先，其次取有读数的来源", () => {
  const failed = provider({
    id: "relay",
    label: "中转",
    status: "error",
    capturedAt: 5,
  });
  const active = provider({
    id: "opencode-go",
    label: "OpenCode Go",
    kind: "subscription",
    status: "ok",
    active: true,
    capturedAt: 5,
    windows: [{ label: "每周", usedPercent: 30, resetsAt: null }],
  });
  assert.equal(activeProviderOf([failed, active]).id, "opencode-go");
  assert.equal(
    activeProviderOf([failed]).id,
    "relay",
    "没有使用中标记时也给出可展示的来源",
  );
  assert.equal(activeProviderOf([]), null);

  const view = providerQuotaView(active, {
    live: true,
    intervalSeconds: 60,
    lastSyncedAt: 5,
  });
  assert.equal(view.billing, "subscription");
  assert.equal(view.from, "provider");
  assert.equal(view.capturedAt, 5);
  assert.deepEqual(view.sync, {
    live: true,
    intervalSeconds: 60,
    lastSyncedAt: 5,
  });
});

test("汇总按服务商去重、按账号计数，并给出矩阵行数", () => {
  const columns = buildAgentColumns([
    {
      meta: meta("deepseek", "DSH"),
      item: item("deepseek", {
        providers: [
          provider({ id: "ds", label: "DeepSeek 官方" }),
          provider({ id: "ds2", label: "DeepSeek 官方" }),
        ],
      }),
    },
    {
      meta: meta("glm", "ZCode"),
      item: item("glm", {
        providers: [
          provider({ id: "oc", label: "OpenCode Go", kind: "subscription" }),
        ],
      }),
    },
  ]);
  assert.deepEqual(summarizeColumns(columns), {
    agents: 2,
    providers: 2,
    accounts: 3,
  });
  assert.equal(columnRows(columns), 2);
});

test("工具卡上的服务商额度：账号额度优先，其次当前 / 上次使用的模型源", () => {
  const usage = item("claude", {
    rate: {
      primary: {
        utilization: 0.58,
        limitWindowSeconds: 604800,
        resetAfterSeconds: 0,
        resetAt: 1_800_100_000,
      },
      secondary: null,
      capturedAt: 1_800_000_000,
      accountVerified: true,
    },
    scannedAt: 1_800_000_000,
  });

  const official = toolQuotaView("claude", usage, entry("native"));
  assert.equal(official.provider, "Anthropic 官方");
  assert.equal(official.state, "active", "官方直连时账号额度就是使用中");
  assert.equal(official.quota.from, "account");
  assert.equal(Math.round(official.quota.windows[0].usedPercent), 58);

  const relay = toolQuotaView(
    "claude",
    usage,
    entry("external", "https://api.example.com"),
  );
  assert.equal(
    relay.state,
    "last",
    "连接已指向自定义接口时，账号快照属于上一次使用",
  );

  const balance = provider({
    id: "deepseek",
    label: "DeepSeek 官方",
    kind: "balance",
    active: true,
    capturedAt: 1_800_000_000,
    balances: [{ label: "充值余额", amount: "36.80", currency: "CNY" }],
  });
  const dsh = toolQuotaView(
    "deepseek",
    item("deepseek", { providers: [balance] }),
    undefined,
  );
  assert.equal(dsh.provider, "DeepSeek 官方");
  assert.equal(dsh.state, "active");
  assert.equal(dsh.quota.billing, "balance");
  assert.equal(dsh.quota.balances[0].amount, "36.80");

  const idle = toolQuotaView("chatgpt", item("chatgpt"), entry("native"));
  assert.equal(idle.quota, null, "没有读数时不编造额度");
  assert.equal(idle.state, "unknown");
  assert.equal(idle.provider, "OpenAI 官方");
});
