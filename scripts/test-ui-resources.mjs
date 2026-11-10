import assert from "node:assert/strict";
import { chromium } from "playwright-core";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const server = await createServer({
  root,
  server: { host: "127.0.0.1", port: 15420, strictPort: true },
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
  const page = await browser.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("http://127.0.0.1:15420/tests/fixtures/agent-bot.html");
  await page.waitForSelector('[data-bot-state="idle"]');
  await page.waitForTimeout(300);
  assert.equal(
    await page.locator("video").count(),
    0,
    "待机头像不得创建视频解码器",
  );
  await page.evaluate(() => window.fixture.render("working"));
  await page.waitForFunction(
    () => document.querySelector("video")?.paused === false,
  );
  await page.evaluate(() => window.fixture.visible(false));
  await page.waitForFunction(() => !document.querySelector("video"));
  assert.equal(
    await page
      .locator("img")
      .evaluate((img) => getComputedStyle(img).visibility),
    "visible",
  );
  await page.evaluate(() => window.fixture.visible(true));
  await page.waitForFunction(
    () => document.querySelector("video")?.paused === false,
  );
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.waitForFunction(() => !document.querySelector("video"));
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.waitForFunction(
    () => document.querySelector("video")?.paused === false,
  );
  await page.locator(".agent-bot").evaluate((element) => {
    element.style.marginTop = "2000px";
  });
  await page.waitForFunction(() => !document.querySelector("video"));
  await page.locator(".agent-bot").evaluate((element) => {
    element.style.marginTop = "0";
  });
  await page.waitForFunction(() => !!document.querySelector("video"));
  await page
    .locator("video")
    .evaluate((video) => video.dispatchEvent(new Event("error")));
  await page.waitForFunction(() => !document.querySelector("video"));
  await page.evaluate(() => window.fixture.render("asking"));
  await page.waitForFunction(() => !!document.querySelector("video"));
  await page.evaluate(() => window.fixture.render("done"));
  await page.waitForFunction(() => !document.querySelector("video"));
  assert.deepEqual(errors, [], "组件不能产生运行时异常");
  console.log(
    "PASS：待机/完成使用静态图，工作动效正常；隐藏、移出视口、减少动态效果和解码失败均释放视频并显示回退图。",
  );
  await page.goto("http://127.0.0.1:15420/tests/fixtures/tray.html");
  await page.waitForFunction(() =>
    window.fixture?.calls.includes("tray_ready"),
  );
  assert.equal(
    await page.evaluate(() => window.fixture.readyBeforeListeners()),
    false,
    "首次打开前必须完成事件订阅",
  );
  await page.waitForFunction(() => window.fixture.visible());
  await page.waitForTimeout(100);
  await page.evaluate(() => window.fixture.emit("tray-visibility", false));
  const hiddenReads = await page.evaluate(
    () => window.fixture.calls.filter((call) => call === "core_status").length,
  );
  await page.evaluate(async () => {
    await window.fixture.emit("core-event");
    await window.fixture.emit("config-changed");
  });
  await page.waitForTimeout(100);
  assert.equal(
    await page.evaluate(
      () =>
        window.fixture.calls.filter((call) => call === "core_status").length,
    ),
    hiddenReads,
    "隐藏托盘不得响应后台事件重复查询",
  );
  await page.evaluate(() => window.fixture.emit("tray-opened"));
  await page.waitForFunction(
    (before) =>
      window.fixture.calls.filter((call) => call === "core_status").length >
      before,
    hiddenReads,
  );
  assert.deepEqual(errors, [], "托盘不能产生运行时异常");
  console.log(
    "PASS：托盘冷启动先订阅再握手，首次打开事件正常；隐藏时停止刷新，重新打开后恢复。",
  );
} finally {
  await browser?.close();
  await server.close();
}
