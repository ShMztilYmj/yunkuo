import { createHash } from "node:crypto";
import { copyFileSync, existsSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { checkEmbeddedFrontend } from "./check-embedded-frontend.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const config = JSON.parse(
  readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8"),
);
const tokenEngine = JSON.parse(
  readFileSync(
    path.join(root, "src-tauri/resources/tokscale/manifest.json"),
    "utf8",
  ),
);
const tokenBinary = path.join(
  root,
  "src-tauri/resources/tokscale/tokscale.exe",
);
if (
  !existsSync(tokenBinary) ||
  createHash("sha256").update(readFileSync(tokenBinary)).digest("hex") !==
    tokenEngine.platforms["win32-x64"].sha256
) {
  throw new Error(
    "统一用量引擎缺失或校验失败，请运行 pnpm agent-engine:prepare",
  );
}

if (config.bundle.resources.some((entry) => /firefox/i.test(entry))) {
  throw new Error("统一浏览器安装包不能再次分发旧 Firefox 资源");
}
if (
  !existsSync(
    path.join(root, "src-tauri/resources/fingerprint-chromium/chrome.exe"),
  )
) {
  throw new Error("指纹 Chromium 引擎缺失，请先运行 pnpm browser:prepare");
}
const extensionManifest = path.join(
  root,
  "src-tauri/resources/extensions/webrtc-control/manifest.json",
);
if (!existsSync(extensionManifest)) {
  throw new Error("指纹浏览器预置扩展缺失，请检查 resources/extensions");
}
// 清单里的 key 决定扩展 ID。没有它，ID 由安装路径推导，换一个免安装目录、换盘或
// 换机器都会变，已经固定到工具栏的按钮随即缩回扩展菜单。
if (!JSON.parse(readFileSync(extensionManifest, "utf8")).key) {
  throw new Error("预置扩展清单缺少 key，工具栏按钮无法固定");
}
const triple = process.env.TAURI_ENV_TARGET_TRIPLE || "x86_64-pc-windows-msvc";
if (triple !== "x86_64-pc-windows-msvc")
  throw new Error("本项目安装包仅支持 Windows x64");
const target = path.resolve(
  root,
  process.env.CARGO_TARGET_DIR || "src-tauri/target",
);
const profile = process.env.TAURI_ENV_DEBUG === "true" ? "debug" : "release";
const candidates = [
  path.join(target, profile),
  path.join(target, triple, profile),
]
  .filter((dir) => existsSync(path.join(dir, "yunkuo.exe")))
  .sort(
    (a, b) =>
      statSync(path.join(b, "yunkuo.exe")).mtimeMs -
      statSync(path.join(a, "yunkuo.exe")).mtimeMs,
  );
if (!candidates.length) throw new Error("请先构建主程序");
const built = candidates[0];
checkEmbeddedFrontend(path.join(built, "yunkuo.exe"));
const sidecar = path.join(
  root,
  "src-tauri/binaries",
  `yunkuo-service-${triple}.exe`,
);
const serviceHash = createHash("sha256")
  .update(readFileSync(sidecar))
  .digest("hex")
  .toUpperCase();
if (!readFileSync(sidecar).includes(Buffer.from("mihomo"))) {
  throw new Error(
    "服务组件尚未迁移到 mihomo，请先重新运行 pnpm service:prepare",
  );
}
if (
  !readFileSync(path.join(built, "yunkuo.exe")).includes(
    Buffer.from(serviceHash),
  )
) {
  throw new Error(
    "主程序与服务组件的校验值不一致，请复制最新服务 sidecar 后重新构建主程序",
  );
}
// Tauri 会同时打包 Cargo 额外 binary 和 externalBin；保证同名文件不会覆盖成另一版本。
copyFileSync(sidecar, path.join(built, "yunkuo-service.exe"));
console.log("服务打包组件及主程序校验值一致");

const mihomo = path.join(
  root,
  "src-tauri/binaries/云阔mihomo-x86_64-pc-windows-msvc.exe",
);
if (
  createHash("sha256")
    .update(readFileSync(mihomo))
    .digest("hex")
    .toUpperCase() !==
  "04F8D7FC2B314771E1ECBF15951D59F0E1CB7914503C88CFB6972FF6A824E314"
)
  throw Error("mihomo 内核哈希不匹配");
copyFileSync(mihomo, path.join(built, "云阔mihomo.exe"));
