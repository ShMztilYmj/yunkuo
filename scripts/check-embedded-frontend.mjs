import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

// Tauri 的页面内容可以压缩，资源路径仍作为 EmbeddedAssets 的键存放在 exe 中。
// 用本次构建的入口文件名校验，可拒绝开发模式 exe 以及包含旧前端的 exe。
export function checkEmbeddedFrontend(
  executable,
  frontend = path.join(root, "dist"),
) {
  const html = readFileSync(path.join(frontend, "index.html"), "utf8");
  const assets = [
    ...html.matchAll(/(?:src|href)=["']\/?(assets\/[^"']+\.(?:js|css))["']/g),
  ].map((match) => match[1]);
  if (!assets.some((asset) => asset.endsWith(".js"))) {
    throw new Error(
      "dist/index.html 缺少编译后的 JavaScript 入口，请先构建前端。",
    );
  }
  const binary = readFileSync(executable);
  const missing = assets.filter(
    (asset) => !binary.includes(Buffer.from(asset)),
  );
  if (missing.length) {
    throw new Error(
      `主程序未内嵌本次构建的前端资源：${missing.join("、")}。请使用 pnpm build:portable，或 pnpm tauri build；仅运行 cargo build --release 会继续依赖 localhost:11420。`,
    );
  }
  console.log(`前端内嵌校验通过：${assets.length} 个入口资源，${executable}`);
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href
) {
  try {
    if (!process.argv[2])
      throw new Error(
        "用法：node scripts/check-embedded-frontend.mjs <exe路径>",
      );
    checkEmbeddedFrontend(path.resolve(process.argv[2]));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
