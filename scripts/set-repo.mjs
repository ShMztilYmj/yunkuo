// 建好 GitHub 仓库后跑一次，把 README 徽章、issue 模板链接、安全邮箱与
// package.json 的 repository 字段指向真实地址。
//
//   node scripts/set-repo.mjs <owner>/<repo> [--email=security@example.com] [--dry-run]
//
// 占位符统一为 OWNER/yunkuo 与 security@TARGET_DOMAIN，脚本可重复执行。
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const argv = process.argv.slice(2);
const dryRun = argv.includes("--dry-run");
const emailArg = argv.find((arg) => arg.startsWith("--email="));
const email = emailArg ? emailArg.slice("--email=".length) : null;
const slug = argv.find((arg) => !arg.startsWith("--"));

if (!slug || !/^[\w.-]+\/[\w.-]+$/.test(slug)) {
  console.error(
    "用法：node scripts/set-repo.mjs <owner>/<repo> [--email=…] [--dry-run]",
  );
  process.exit(1);
}

const targets = [
  "README.md",
  ".github/ISSUE_TEMPLATE/config.yml",
  "SECURITY.md",
  "CONTRIBUTING.md",
];

const placeholders = [
  ["OWNER/REPO", slug],
  ["OWNER/yunkuo", slug],
];
if (email) placeholders.push(["security@TARGET_DOMAIN", email]);

let changed = 0;
for (const relative of targets) {
  const file = path.join(root, relative);
  if (!existsSync(file)) continue;
  const before = readFileSync(file, "utf8");
  let after = before;
  for (const [from, to] of placeholders) after = after.split(from).join(to);
  if (after === before) continue;
  changed += 1;
  console.log(`  ${relative}`);
  if (!dryRun) writeFileSync(file, after);
}

// package.json 的 repository 字段：没有就补，有就改。
const manifestPath = path.join(root, "package.json");
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
const url = `https://github.com/${slug}.git`;
if (manifest.repository?.url !== url) {
  manifest.repository = { type: "git", url };
  console.log("  package.json（repository 字段）");
  changed += 1;
  if (!dryRun)
    writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
}

if (!changed) {
  console.log("没有需要替换的占位符，仓库地址已经是最新的。");
} else if (dryRun) {
  console.log(`\n以上 ${changed} 处将被改写（--dry-run 未落盘）。`);
} else {
  console.log(
    `\n已更新 ${changed} 处。提交后再推送：git add -A && git commit -m "chore: 指向 GitHub 仓库"`,
  );
}
