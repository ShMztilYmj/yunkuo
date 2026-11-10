// 校验 package.json 里的脚本都指向真实存在的文件，避免再次出现“脚本还在、目标已丢”的死命令。
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
let pkg;
try {
  pkg = JSON.parse(
    readFileSync(path.join(root, "package.json"), "utf8").replace(
      /^\uFEFF/,
      "",
    ),
  );
} catch (error) {
  console.error(`无法读取 package.json：${error.message}`);
  process.exit(1);
}
const scripts = pkg.scripts ?? {};

// 只认仓库内的相对路径；命令名、参数和 --flag 不参与检查。
const filePattern =
  /(?:^|\s)((?:\.\/)?(?:scripts|tests|src|src-tauri)\/[^\s"'`]+)/g;
const pnpmPattern = /\bpnpm\s+(?:run\s+)?([\w:.-]+)/g;
const problems = [];

for (const [name, command] of Object.entries(scripts)) {
  for (const match of command.matchAll(filePattern)) {
    const target = match[1].replace(/^\.\//, "").replace(/[;,)]$/, "");
    if (!existsSync(path.join(root, target))) {
      problems.push(`${name} → 缺少文件 ${target}`);
    }
  }
  for (const match of command.matchAll(pnpmPattern)) {
    const referenced = match[1];
    if (referenced === "run" || referenced === "exec" || referenced === "dlx")
      continue;
    if (!scripts[referenced]) {
      problems.push(`${name} → 引用了不存在的脚本 ${referenced}`);
    }
  }
}

if (problems.length) {
  console.error(`脚本契约检查失败（${problems.length} 项）：`);
  for (const problem of problems) console.error(`  - ${problem}`);
  process.exitCode = 1;
} else {
  console.log(
    `脚本契约检查通过：${Object.keys(scripts).length} 个脚本，引用的文件与子脚本均存在。`,
  );
}

// 测试夹具由 Rust 用例按相对路径引用；缺文件时现象是“子进程连接已退出”，
// 很难顺着日志定位，所以在这里一并校验。
const rustFiles = [];
const walk = (directory) => {
  for (const entry of readdirSync(directory)) {
    const full = path.join(directory, entry);
    if (statSync(full).isDirectory()) walk(full);
    else if (entry.endsWith(".rs")) rustFiles.push(full);
  }
};
walk(path.join(root, "src-tauri/src"));

const missingFixtures = new Set();
for (const file of rustFiles) {
  for (const match of readFileSync(file, "utf8").matchAll(
    /scripts\/fixtures\/([\w.-]+)/g,
  )) {
    if (!existsSync(path.join(root, "scripts/fixtures", match[1]))) {
      missingFixtures.add(
        `${path.relative(root, file)} → scripts/fixtures/${match[1]}`,
      );
    }
  }
}
if (missingFixtures.size) {
  console.error(`测试夹具缺失（${missingFixtures.size} 项）：`);
  for (const problem of missingFixtures) console.error(`  - ${problem}`);
  process.exitCode = 1;
}
