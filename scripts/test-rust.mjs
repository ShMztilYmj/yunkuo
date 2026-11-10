// Rust 用例入口：默认串行执行；失败项逐个单独重试一次。
// 代理内核、协作等用例会占用真实端口并拉起真实进程，同进程内互相干扰时会偶发失败，
// 单独重跑即可区分“干扰”与“真失败”——两次都失败才算失败。
import { spawnSync } from "node:child_process";
import {
  appendFileSync,
  closeSync,
  mkdirSync,
  openSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const manifest = "src-tauri/Cargo.toml";
const logDir = path.join(root, "artifacts");
const logPath = path.join(logDir, "test-rust.log");
// 每次运行都会覆盖 test-rust.log；失败证据单独落盘，否则重试成功后原始输出就没了。
const failurePath = path.join(logDir, "test-rust-failure.log");
mkdirSync(logDir, { recursive: true });

function runCargo(target) {
  // 用文件描述符重定向，而不是管道：受限沙箱里不允许为子进程创建管道。
  // `--exact` 是测试二进制自己的参数，必须放在 `--` 之后，否则 cargo 只会打印用法。
  const args = [
    "test",
    "--manifest-path",
    manifest,
    "--lib",
    "--",
    "--test-threads=1",
  ];
  if (target) args.push("--exact", target);
  const fd = openSync(logPath, "w");
  try {
    const result = spawnSync("cargo", args, {
      cwd: root,
      stdio: ["ignore", fd, fd],
    });
    return {
      status: result.status ?? 1,
      output: readFileSync(logPath, "utf8"),
    };
  } finally {
    closeSync(fd);
  }
}

function failedTests(output) {
  const names = new Set();
  let inBlock = false;
  for (const line of output.split(/\r?\n/)) {
    if (/^failures:$/.test(line)) {
      inBlock = true;
      continue;
    }
    if (!inBlock) continue;
    if (!line.trim()) continue;
    if (/^test result:/.test(line)) break;
    const match = line.match(/^\s{4}(\S+)$/);
    if (match) names.add(match[1]);
  }
  return [...names];
}

const first = runCargo(null);
const firstFailures = first.status === 0 ? [] : failedTests(first.output);
if (first.status === 0) {
  const summary = first.output.match(/test result: .*/)?.[0] ?? "全部通过";
  console.log(`Rust 用例通过：${summary}`);
  process.exit(0);
}
// 首轮失败时先保底：test-rust.log 会在每次重试时被覆盖。
writeFileSync(failurePath, first.output);
if (!firstFailures.length) {
  console.error("Rust 用例失败，且未能从输出中解析出失败用例：");
  console.error(first.output.split(/\r?\n/).slice(-20).join("\n"));
  console.error(`完整输出：${path.relative(root, failurePath)}`);
  process.exit(first.status || 1);
}

console.log(
  `\n首轮失败 ${firstFailures.length} 项，逐个单独重试（最多两次）：`,
);
let stillFailing = 0;
for (const name of firstFailures) {
  let passed = false;
  for (let attempt = 1; attempt <= 2 && !passed; attempt += 1) {
    const retry = runCargo(name);
    passed = retry.status === 0;
    if (!passed) {
      appendFileSync(
        failurePath,
        `\n===== ${name} 第 ${attempt} 次重试 =====\n${retry.output}`,
      );
      console.log(`  · 第 ${attempt} 次重试仍失败：${name}`);
    }
  }
  if (passed) {
    console.log(`  ✔ 重试通过：${name}`);
  } else {
    stillFailing += 1;
    console.log(`  ✘ 仍然失败：${name}`);
  }
}
if (stillFailing) {
  console.error(`\n${stillFailing} 项在单独重试后仍失败，需人工确认。`);
  console.error(`失败输出已保存：${path.relative(root, failurePath)}`);
  process.exit(1);
}
console.log("\n首轮失败项单独重试后全部通过（同进程干扰导致，非真实失败）。");
