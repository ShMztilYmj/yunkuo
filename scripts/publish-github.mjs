// 一条命令把当前仓库发布到 GitHub：建仓库（可选）→ 配置 origin → 推 main → 推标签。
//
//   $env:GITHUB_TOKEN = "github_pat_xxx"      # 只需 Contents/Workflows 写权限 + 建仓权限
//   node scripts/publish-github.mjs <owner>/<repo> [--private] [--tag=v0.2.14] [--dry-run]
//
// 令牌只用于本次请求，不写进 .git/config，也不会打印出来。
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const argv = process.argv.slice(2);
const dryRun = argv.includes("--dry-run");
const isPrivate = argv.includes("--private");
const tagArg = argv.find((arg) => arg.startsWith("--tag="));
const tag = tagArg ? tagArg.slice("--tag=".length) : null;
const slug = argv.find((arg) => !arg.startsWith("--"));

if (!slug || !/^[\w.-]+\/[\w.-]+$/.test(slug)) {
  console.error(
    "用法：node scripts/publish-github.mjs <owner>/<repo> [--private] [--tag=v0.2.14]",
  );
  process.exit(1);
}

const token = process.env.GITHUB_TOKEN || process.env.GH_TOKEN;
if (!token) {
  console.error(
    "缺少令牌。先设置环境变量 GITHUB_TOKEN（Fine-grained 需要 Contents: Read and write、Workflows: Read and write、Administration: Read and write）。",
  );
  process.exit(1);
}

const [owner, name] = slug.split("/");
const apiBase = "https://api.github.com";

function mask(text) {
  return String(text).split(token).join("***");
}

function run(command, args, label, options = {}) {
  const result = spawnSync(command, args, {
    cwd: root,
    encoding: "utf8",
    ...options,
  });
  if (result.error)
    throw new Error(`${label} 无法执行：${result.error.message}`);
  if (result.status !== 0) {
    throw new Error(`${label} 失败：\n${mask(result.stderr || result.stdout)}`);
  }
  return mask(result.stdout || "");
}

async function api(pathname, init = {}) {
  const response = await fetch(`${apiBase}${pathname}`, {
    ...init,
    headers: {
      Accept: "application/vnd.github+json",
      Authorization: `Bearer ${token}`,
      "X-GitHub-Api-Version": "2022-11-28",
      "User-Agent": "yunkuo-publish",
      ...(init.body ? { "Content-Type": "application/json" } : {}),
      ...init.headers,
    },
  });
  const text = await response.text();
  const body = text ? JSON.parse(text) : null;
  return { status: response.status, body, ok: response.ok };
}

async function main() {
  const me = await api("/user");
  if (!me.ok) {
    throw new Error(
      `令牌无效或权限不足（HTTP ${me.status}）：${me.body?.message ?? ""}`,
    );
  }
  console.log(`令牌身份：${me.body.login}`);

  const existing = await api(`/repos/${slug}`);
  let created = false;
  if (existing.ok) {
    console.log(`仓库已存在：${existing.body.html_url}`);
  } else if (existing.status !== 404) {
    throw new Error(`查询仓库失败：HTTP ${existing.status}`);
  } else if (dryRun) {
    console.log(`将创建仓库 ${slug}（${isPrivate ? "私有" : "公开"}）`);
  } else {
    const payload = {
      name,
      private: isPrivate,
      description:
        "云阔 · Windows 原生代理、环境检测与 AI 工作台（Tauri 2 + React）",
      has_issues: true,
      has_wiki: false,
      has_projects: false,
    };
    const createdRes = await api(
      me.body.login === owner ? "/user/repos" : `/orgs/${owner}/repos`,
      { method: "POST", body: JSON.stringify(payload) },
    );
    if (!createdRes.ok) {
      throw new Error(
        `创建仓库失败（HTTP ${createdRes.status}）：${createdRes.body?.message ?? ""}`,
      );
    }
    created = true;
    console.log(`已创建仓库：${createdRes.body.html_url}`);
  }

  // 提交尚未落盘时先拦住，避免推上去的是旧内容。
  const dirty = run("git", ["status", "--porcelain"], "git status").trim();
  if (dirty) {
    console.error("工作区有未提交的改动：\n" + dirty);
    console.error("先 git add -A && git commit，再重新执行本脚本。");
    process.exitCode = 1;
    return;
  }

  const remoteUrl = `https://github.com/${slug}.git`;
  const remotes = run("git", ["remote"], "git remote").split("\n");
  if (!remotes.includes("origin")) {
    run("git", ["remote", "add", "origin", remoteUrl], "git remote add");
    console.log(`已添加 origin：${remoteUrl}`);
  } else {
    run(
      "git",
      ["remote", "set-url", "origin", remoteUrl],
      "git remote set-url",
    );
  }

  if (tag) {
    const tags = run("git", ["tag", "--list", tag], "git tag --list").trim();
    if (!tags) run("git", ["tag", "-a", tag, "-m", `云阔 ${tag}`], "git tag");
  }

  if (dryRun) {
    console.log("\n--dry-run：跳过推送。");
    return;
  }

  // 令牌只出现在这一次 push 的参数里，不落到 .git/config。
  const pushUrl = `https://x-access-token:${token}@github.com/${slug}.git`;
  console.log("推送 main…");
  run("git", ["push", pushUrl, "HEAD:refs/heads/main"], "git push", {
    stdio: ["ignore", "inherit", "pipe"],
  });
  if (tag) {
    console.log(`推送标签 ${tag}…`);
    run("git", ["push", pushUrl, `refs/tags/${tag}`], "git push tag", {
      stdio: ["ignore", "inherit", "pipe"],
    });
  }

  console.log(`\n完成：https://github.com/${slug}`);
  if (created) {
    console.log(
      "下一步：到 Settings 打开 secret scanning / push protection，替换 README 占位符。",
    );
  }
}

try {
  await main();
} catch (error) {
  console.error(`\n失败：${mask(error.message)}`);
  process.exit(1);
}
