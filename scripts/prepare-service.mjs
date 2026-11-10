// 构建系统服务侧车并复制成 Tauri externalBin 约定的三元组文件名。
// 顺序要求：必须在构建主程序之前运行；build.rs 会把该文件的 SHA256 内嵌到主程序，
// 用来在安装/修复服务时校验复制过去的二进制。
import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  statSync,
} from "node:fs";
import { createHash } from "node:crypto";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { setFileDescription } from "./pe-version.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const manifest = path.join(root, "src-tauri/Cargo.toml");
const triple = process.env.TAURI_ENV_TARGET_TRIPLE || "x86_64-pc-windows-msvc";
const targetDir = process.env.CARGO_TARGET_DIR
  ? path.resolve(root, process.env.CARGO_TARGET_DIR)
  : path.join(root, "src-tauri/target");
const built = path.join(targetDir, "release", "yunkuo-service.exe");
const sidecar = path.join(
  root,
  "src-tauri/binaries",
  `yunkuo-service-${triple}.exe`,
);

console.log("构建系统服务侧车：yunkuo-service");
const build = spawnSync(
  "cargo",
  [
    "build",
    "--release",
    "--bin",
    "yunkuo-service",
    "--manifest-path",
    manifest,
  ],
  { stdio: "inherit" },
);
if (build.error) throw build.error;
if (build.status !== 0) {
  throw new Error(`服务侧车构建失败（退出码 ${build.status}）`);
}
if (!existsSync(built)) {
  throw new Error(`没有找到构建产物：${built}`);
}
if (!readFileSync(built).includes(Buffer.from("mihomo"))) {
  throw new Error("服务侧车尚未迁移到 mihomo，拒绝作为打包组件");
}

mkdirSync(path.dirname(sidecar), { recursive: true });
// 任务管理器的“名称”列取版本资源里的 FileDescription，不是文件名：
// 服务组件显示为 云阔TUN，文件名保持不变（服务注册路径与哈希校验都依赖它）。
setFileDescription(built, "云阔TUN");
copyFileSync(built, sidecar);
const hash = createHash("sha256")
  .update(readFileSync(sidecar))
  .digest("hex")
  .toUpperCase();
const size = (statSync(sidecar).size / 1024 / 1024).toFixed(2);
console.log(`已复制：${path.relative(root, sidecar)}（${size} MB）`);
console.log(`SHA256：${hash}`);
console.log(
  "下一步：重新构建主程序（pnpm build:portable），让它内嵌这个哈希。",
);
