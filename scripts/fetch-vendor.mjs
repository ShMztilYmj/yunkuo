// 拉取随包的第三方资源：mihomo 内核、用量引擎、规则库、指纹浏览器。
// 这些文件体积大（合计约 1.3 GB），不进 Git，克隆后按 scripts/vendor.lock.json
// 的固定版本与哈希落地。只依赖 Node 内置模块与 Windows 自带的 tar/Expand-Archive。
//
// 用法：
//   node scripts/fetch-vendor.mjs --check          # 只看现在缺什么，不联网
//   node scripts/fetch-vendor.mjs                  # 下载缺失项（跳过指纹浏览器）
//   node scripts/fetch-vendor.mjs --with-browser   # 连同 181 MB 的指纹浏览器一起下
//   node scripts/fetch-vendor.mjs --only=mihomo,tokscale --force
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFileSync,
  cpSync,
  createWriteStream,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const lock = JSON.parse(
  readFileSync(path.join(root, "scripts/vendor.lock.json"), "utf8"),
);

const argv = process.argv.slice(2);
const flags = new Set(argv.filter((arg) => arg.startsWith("--")));
const onlyArg = argv.find((arg) => arg.startsWith("--only="));
const only = onlyArg
  ? onlyArg
      .slice("--only=".length)
      .split(",")
      .map((id) => id.trim())
      .filter(Boolean)
  : null;
const checkOnly = flags.has("--check");
const force = flags.has("--force");
const withBrowser = flags.has("--with-browser");

const MB = 1024 * 1024;

function formatBytes(bytes) {
  if (bytes >= 1024 * MB) return `${(bytes / 1024 / MB).toFixed(2)} GB`;
  return `${(bytes / MB).toFixed(1)} MB`;
}

function hashOf(file) {
  return createHash("sha256").update(readFileSync(file)).digest("hex");
}

function matches(actual, expected) {
  if (!expected) return true;
  if (!actual) return false;
  return actual.toLowerCase() === expected.toLowerCase();
}

function relative(file) {
  return path.relative(root, file) || ".";
}

function ensureDir(dir) {
  mkdirSync(dir, { recursive: true });
}

async function download(url, file, title) {
  const response = await fetch(url, { redirect: "follow" });
  if (!response.ok) {
    throw new Error(`HTTP ${response.status} ${response.statusText}`);
  }
  const total = Number(response.headers.get("content-length") || 0);
  ensureDir(path.dirname(file));
  const partial = `${file}.part`;
  const stream = createWriteStream(partial);
  let received = 0;
  let lastPrint = 0;
  try {
    for await (const chunk of response.body) {
      received += chunk.length;
      if (!stream.write(chunk)) {
        await new Promise((resolve) => stream.once("drain", resolve));
      }
      const now = Date.now();
      if (now - lastPrint > 1000) {
        lastPrint = now;
        const percent = total
          ? ` ${((received / total) * 100).toFixed(0)}%`
          : "";
        process.stdout.write(
          `\r    ${title} ${formatBytes(received)}${total ? ` / ${formatBytes(total)}` : ""}${percent}   `,
        );
      }
    }
    await new Promise((resolve, reject) => {
      stream.end((error) => (error ? reject(error) : resolve()));
    });
  } catch (error) {
    stream.destroy();
    rmSync(partial, { force: true });
    throw error;
  }
  process.stdout.write("\r");
  rmSync(file, { force: true });
  copyFileSync(partial, file);
  rmSync(partial, { force: true });
  return received;
}

function extractZip(zip, dir) {
  ensureDir(dir);
  const tar = spawnSync("tar.exe", ["-xf", zip, "-C", dir], {
    stdio: "inherit",
  });
  if (!tar.error && tar.status === 0) return;
  // 老系统没有 bsdtar 时退回 PowerShell 的 Expand-Archive。
  const escaped = zip.replace(/'/g, "''");
  const target = dir.replace(/'/g, "''");
  const ps = spawnSync(
    "powershell",
    [
      "-NoProfile",
      "-ExecutionPolicy",
      "Bypass",
      "-Command",
      `Expand-Archive -LiteralPath '${escaped}' -DestinationPath '${target}' -Force`,
    ],
    { stdio: "inherit" },
  );
  if (ps.error || ps.status !== 0) {
    throw new Error("解压失败：tar.exe 与 Expand-Archive 都不可用");
  }
}

function walk(dir) {
  const found = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) found.push(...walk(full));
    else found.push(full);
  }
  return found;
}

function locate(dir, pattern) {
  const matcher = new RegExp(pattern, "i");
  const hit = walk(dir).find((file) => matcher.test(file));
  if (!hit) throw new Error(`压缩包内没有匹配 ${pattern} 的文件`);
  return hit;
}

async function remoteSha256(url) {
  const response = await fetch(url, { redirect: "follow" });
  if (!response.ok) throw new Error(`HTTP ${response.status} 取校验文件失败`);
  const text = await response.text();
  const hex = text.trim().split(/\s+/)[0];
  if (!/^[0-9a-f]{64}$/i.test(hex)) throw new Error("上游校验文件格式不符");
  return hex;
}

function status(item) {
  const dest = path.join(root, item.dest);
  const present = existsSync(dest);
  const isDir = item.kind === "zip-dir";
  const actual = present && !isDir ? hashOf(dest) : null;
  const wanted = item.sha256 || null;
  const verified = isDir ? present : matches(actual, wanted);
  return {
    id: item.id,
    title: item.title,
    dest,
    present,
    actual,
    wanted,
    verified,
    size: present
      ? isDir
        ? walk(dest).reduce((sum, file) => sum + statSync(file).size, 0)
        : statSync(dest).size
      : 0,
  };
}

async function install(item, index, total) {
  const dest = path.join(root, item.dest);
  const staging = path.join(
    os.tmpdir(),
    `yunkuo-vendor-${item.id}-${Date.now()}`,
  );
  process.stdout.write(
    `\n[${index}/${total}] ${item.title}\n    ${item.url}\n`,
  );

  if (item.kind === "file") {
    await download(item.url, dest, item.id);
    if (item.remoteSha256) {
      const expected = await remoteSha256(item.remoteSha256);
      const actual = hashOf(dest);
      if (!matches(actual, expected)) {
        rmSync(dest, { force: true });
        throw new Error(`哈希与上游 .sha256sum 不一致：${actual}`);
      }
      console.log(`    哈希已按上游 .sha256sum 校验通过`);
    }
  } else {
    const zip = path.join(staging, "download.zip");
    await download(item.url, zip, `${item.id}.zip`);
    extractZip(zip, path.join(staging, "unpacked"));
    const hit = locate(path.join(staging, "unpacked"), item.find);
    if (item.kind === "zip-file") {
      ensureDir(path.dirname(dest));
      rmSync(dest, { force: true });
      copyFileSync(hit, dest);
    } else {
      rmSync(dest, { recursive: true, force: true });
      ensureDir(dest);
      cpSync(path.dirname(hit), dest, { recursive: true });
    }
    rmSync(staging, { recursive: true, force: true });
  }

  if (item.sha256) {
    const actual = hashOf(dest);
    if (!matches(actual, item.sha256)) {
      throw new Error(`哈希不匹配：期望 ${item.sha256}，实际 ${actual}`);
    }
  }
  console.log(`    完成 → ${relative(dest)}`);
}

function printTable(rows) {
  const width = rows.reduce((max, row) => Math.max(max, row.id.length), 0);
  for (const row of rows) {
    const mark = row.present && row.verified ? "✔" : row.present ? "!" : "✘";
    const note = !row.present
      ? row.optional
        ? "未安装（可选）"
        : "缺失"
      : row.verified
        ? `就绪 ${formatBytes(row.size)}`
        : `哈希不符（期望 ${row.wanted.slice(0, 12)}…，实际 ${row.actual.slice(0, 12)}…）`;
    console.log(
      `  ${mark} ${row.id.padEnd(width)}  ${note.padEnd(46)}${relative(row.dest)}`,
    );
  }
}

async function main() {
  const items = lock.items.filter((item) => {
    if (only && !only.includes(item.id)) return false;
    if (item.optional && !withBrowser && !only) return false;
    return true;
  });
  if (!items.length) {
    console.error("没有匹配的条目，检查 --only= 的取值");
    process.exitCode = 1;
    return;
  }

  const before = items.map((item) => ({
    ...status(item),
    optional: Boolean(item.optional),
    item,
  }));

  if (checkOnly) {
    console.log("随包资源状态：\n");
    printTable(before);
    const missing = before.filter((row) => !row.verified && !row.optional);
    if (missing.length) {
      console.log(`\n运行 pnpm fetch:vendor 补齐 ${missing.length} 项。`);
    }
    return;
  }

  const pending = before.filter((row) => force || !row.verified);
  if (!pending.length) {
    console.log("随包资源已就绪：\n");
    printTable(before);
    return;
  }

  const skipped = before.filter((row) => !force && row.verified);
  for (const [index, row] of pending.entries()) {
    try {
      await install(row.item, index + 1, pending.length);
    } catch (error) {
      console.error(`    失败：${error.message}`);
      process.exitCode = 1;
    }
  }

  const after = items.map((item) => ({
    ...status(item),
    optional: Boolean(item.optional),
  }));
  console.log("\n结果：\n");
  printTable(after);
  if (skipped.length)
    console.log(`\n${skipped.length} 项已存在且哈希一致，未重复下载。`);
  if (process.exitCode) {
    console.log(
      "\n有资源未就绪。指纹浏览器体积大，确认需要时加 --with-browser；其余可参考上方错误。",
    );
  }
}

await main();
