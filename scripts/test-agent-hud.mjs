import assert from "node:assert/strict";
import { chromium } from "playwright-core";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import { mkdirSync } from "node:fs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
mkdirSync(path.join(root, ".tmp"), { recursive: true });
process.env.TEMP = path.join(root, ".tmp");
process.env.TMP = process.env.TEMP;
const server = await createServer({
  root,
  server: {
    host: "127.0.0.1",
    port: 15421,
    strictPort: true,
    watch: { ignored: ["**/.tmp/**"] },
  },
});
let browser;
try {
  await server.listen();
  browser = await chromium.launch({
    executablePath: path.join(
      root,
      "src-tauri/resources/fingerprint-chromium/chrome.exe",
    ),
    headless: true,
  });
  const page = await browser.newPage({ viewport: { width: 480, height: 520 } });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error" && !message.text().includes("404"))
      errors.push(message.text());
  });
  page.setDefaultTimeout(8000);
  await page.goto("http://127.0.0.1:15421/tests/fixtures/agent-hud.html");
  await page.waitForFunction(() =>
    window.fixture?.calls.some((call) => call.command === "agent_hud_ready"),
  );
  await page.waitForSelector(".hud-agent");
  assert.equal(
    await page.evaluate(() => window.fixture.readyBeforeListeners()),
    false,
  );
  await page.locator(".hud-drag").focus();
  await page.waitForTimeout(300);
  await page.getByRole("button", { name: "固定展开", exact: true }).click();
  for (let i = 0; i < 5; i++) {
    await page.getByRole("button", { name: "当前在线", exact: true }).click();
    await page.waitForFunction(
      () =>
        document.querySelector(
          '.hud-display-options button[aria-pressed="true"]',
        )?.textContent === "当前在线",
    );
    await page.waitForFunction(
      () => document.querySelectorAll(".hud-agent").length === 1,
    );
    assert.equal(
      await page.locator(".hud-current-agent h2").textContent(),
      "Codex",
    );
    await page.getByRole("button", { name: "已添加工具", exact: true }).click();
    await page.waitForFunction(
      () => document.querySelectorAll(".hud-agent").length === 4,
    );
    assert.equal(
      await page.locator(".hud-details:visible").count(),
      1,
      "切换范围后详情必须保持展开",
    );
    assert.equal(
      await page.locator('[role="alert"]').count(),
      0,
      "切换不能报错",
    );
  }
  assert.deepEqual(errors, [], "悬浮条切换不能产生 React 或 IPC 异常");
  console.log("PASS：StrictMode 冷启动握手和当前在线 / 已添加工具反复切换。");
  await page.waitForTimeout(100);
  // 已发送的尺寸响应可以晚于新的设置事件，返回的旧 scope 不能覆盖用户选择。
  await page.evaluate(() => window.fixture.holdLayout());
  await page.getByRole("button", { name: "当前在线", exact: true }).click();
  await page.waitForFunction(() => window.fixture.layoutHeld());
  await page.getByRole("button", { name: "已添加工具", exact: true }).click();
  await page.waitForFunction(
    () => document.querySelectorAll(".hud-agent").length === 4,
  );
  await page.evaluate(() => {
    window.fixture.scopeRollbacks = [];
    new MutationObserver(() => {
      const label = document.querySelector(
        '.hud-display-options button[aria-pressed="true"]',
      )?.textContent;
      if (label !== "已添加工具") window.fixture.scopeRollbacks.push(label);
    }).observe(document.querySelector(".hud-display-options"), {
      attributes: true,
      subtree: true,
    });
    window.fixture.releaseLayout();
  });
  await page.waitForTimeout(250);
  assert.deepEqual(
    await page.evaluate(() => window.fixture.scopeRollbacks),
    [],
    "旧尺寸响应不得把已添加工具切回当前在线",
  );
  console.log("PASS：尺寸响应晚到不会撤销用户刚保存的显示范围。");
  await page.evaluate(async () => {
    window.fixture.toolsFail(true);
    await window.fixture.emit("config-changed", {
      ai_tool_cards: ["claude", "chatgpt", "cursor", "deepseek"],
      mode: "rule",
    });
  });
  await page.waitForFunction(() =>
    document.querySelector('[role="alert"]')?.textContent.includes("工具识别"),
  );
  assert.equal(
    await page.locator(".hud-agent").count(),
    4,
    "工具识别失败时已添加工具不能全部消失",
  );
  console.log("PASS：工具识别失败保留已添加工具和重试入口。");
  await page.evaluate(() => window.fixture.toolsFail(false));
  await page.getByRole("button", { name: "重新识别", exact: true }).click();
  await page.waitForFunction(() => !document.querySelector('[role="alert"]'));
  await page.getByRole("button", { name: "取消固定展开", exact: true }).click();
  await page.mouse.move(-10, -10);
  await page.evaluate(() => document.activeElement?.blur());
  await page.evaluate(() =>
    document.querySelector('button[aria-label="固定展开"]').click(),
  );
  await page.waitForTimeout(400);
  assert.equal(
    await page.locator(".hud-details:visible").count(),
    1,
    "离开后尚未触发的收起计时器不能撤销新设置的固定展开",
  );
  await page.evaluate(() => window.fixture.fail("测试：设置暂时无法保存"));
  await page.getByRole("button", { name: "当前在线", exact: true }).click();
  await page.waitForFunction(() =>
    document
      .querySelector('[role="alert"]')
      ?.textContent.includes("设置暂时无法保存"),
  );
  assert.equal(
    await page.locator(".hud-agent").count(),
    4,
    "保存失败应保持原显示范围",
  );
  await page
    .getByRole("button", { name: "刷新悬浮条统计", exact: true })
    .click();
  await page.waitForTimeout(100);
  assert.match(
    await page.locator(".hud-scroll").textContent(),
    /设置暂时无法保存/,
    "统计刷新成功不能清除设置保存错误",
  );
  await page.evaluate(() => window.fixture.fail(null));
  await page.getByRole("button", { name: "重试保存", exact: true }).click();
  await page.waitForFunction(
    () => document.querySelectorAll(".hud-agent").length === 1,
  );
  await page.evaluate(async () => {
    window.fixture.online([]);
    await window.fixture.emit("config-changed", {
      ai_tool_cards: ["claude", "chatgpt", "cursor", "deepseek"],
      mode: "rule",
    });
  });
  await page.waitForFunction(
    () => document.querySelectorAll(".hud-agent").length === 0,
  );
  assert.equal(
    await page.locator(".hud-details:visible").count(),
    1,
    "零在线工具时仍能切回已添加工具",
  );
  assert.match(
    await page.locator(".hud-scroll").textContent(),
    /没有识别到在线工具/,
  );
  await page.getByRole("button", { name: "已添加工具", exact: true }).click();
  await page.waitForFunction(
    () => document.querySelectorAll(".hud-agent").length === 4,
  );
  await page.getByRole("tab", { name: "会话详情", exact: true }).click();
  assert.equal(await page.locator(".hud-session").count(), 1);
  await page.keyboard.press("Escape");
  await page.waitForFunction(
    () => !document.querySelector(".agent-hud-viewport.is-expanded"),
  );
  assert.deepEqual(errors, [], "异常恢复、空列表和会话切换不得出现运行时异常");
  console.log(
    "PASS：识别重试、保存失败恢复、零在线工具、会话视图与 Esc 收起。",
  );
  await page.goto(
    "http://127.0.0.1:15421/tests/fixtures/agent-hud.html?tools-fail",
  );
  await page.waitForFunction(() =>
    window.fixture?.calls.some((call) => call.command === "agent_hud_tools"),
  );
  assert.equal(
    await page.locator(".hud-agent").count(),
    4,
    "首次识别失败也要显示配置中的已添加项",
  );
  await page.locator(".hud-drag").focus();
  await page.getByRole("button", { name: "固定展开", exact: true }).click();
  await page.evaluate(() => window.fixture.toolsFail(false));
  await page.getByRole("button", { name: "重新识别", exact: true }).click();
  await page.waitForFunction(() => !document.querySelector('[role="alert"]'));
  await page.evaluate(() =>
    window.fixture.cards([
      "claude",
      "chatgpt",
      "cursor",
      "antigravity",
      "opencode",
      "kimi",
      "grok",
      "glm",
      "deepseek",
      "gemini",
    ]),
  );
  await page.waitForFunction(
    () => document.querySelectorAll(".hud-agent").length === 10,
  );
  await page.waitForTimeout(100);
  await page.setViewportSize({ width: 656, height: 520 });
  await page.getByRole("tab", { name: "额度与用量", exact: true }).click();
  mkdirSync(path.join(root, "artifacts/hud-audit"), { recursive: true });
  await page.screenshot({
    path: path.join(root, "artifacts/hud-audit/expanded-top.png"),
  });
  for (const edge of ["left", "right"]) {
    await page.setViewportSize({ width: 480, height: 744 });
    await page.evaluate((edge) => window.fixture.dock(edge), edge);
    await page.waitForTimeout(100);
    const geometry = await page.locator(".hud-display-options").boundingBox();
    assert.ok(
      geometry.x >= 0 && geometry.x + geometry.width <= 480,
      `${edge} 停靠时切换控件必须在窗口内`,
    );
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth > window.innerWidth,
    );
    assert.equal(overflow, false, `${edge} 停靠不能产生横向溢出`);
    const strip = await page.locator(".agent-hud-strip").boundingBox();
    const details = await page.locator(".hud-details").boundingBox();
    assert.ok(
      strip.x >= 0 && strip.x + strip.width <= 480,
      `${edge} 的图标栏必须在窗口内`,
    );
    assert.ok(
      edge === "right"
        ? details.x + details.width <= strip.x
        : strip.x + strip.width <= details.x,
      `${edge} 的图标栏与详情不能重叠`,
    );
    await page.screenshot({
      path: path.join(root, `artifacts/hud-audit/expanded-${edge}.png`),
    });
  }
  await page.emulateMedia({ colorScheme: "dark" });
  await page.evaluate(() => {
    document.documentElement.dataset.theme = "dark";
  });
  await page.screenshot({
    path: path.join(root, "artifacts/hud-audit/expanded-dark.png"),
  });
  assert.deepEqual(
    errors,
    [],
    "首次识别失败和十个工具的左右停靠不得出现运行时异常",
  );
  console.log("PASS：首次识别失败、10 个工具、左右停靠和亮暗布局。");
} finally {
  await browser?.close();
  await server.close();
}
