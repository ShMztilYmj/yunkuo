// Windows 代码签名。tauri.conf.json > bundle.windows.signCommand 对每个待签文件调用本脚本，prepare-service.mjs 也导入它。
// 只签本项目自己的程序：主程序、服务组件、安装包、卸载程序和 NSIS 插件。resources 下的第三方引擎
// 的哈希已固定在主程序或清单中，签名会改变哈希，因此跳过。
// 证书来源二选一：
//   YUNKUO_SIGN_THUMBPRINT：证书存储中代码签名证书的指纹（默认当前用户存储，YUNKUO_SIGN_MACHINE_STORE=1 改用本机存储；硬件令牌/EV 证书也走这里）
//   YUNKUO_SIGN_PFX 与 YUNKUO_SIGN_PFX_PASSWORD：PFX 文件
// YUNKUO_SIGN_TIMESTAMP_URL 可覆盖 RFC 3161 时间戳服务；YUNKUO_SIGNTOOL 可指定 signtool.exe。
// 未配置证书时失败，避免把未签名的包当作发行版；只需本地测试包时使用 `pnpm tauri build --no-sign`。
import { spawnSync } from "node:child_process";
import {
  closeSync,
  existsSync,
  openSync,
  readFileSync,
  readSync,
  readdirSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const resources = path.join(root, "src-tauri", "resources") + path.sep;
const product = JSON.parse(
  readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8"),
).productName;

export function signingConfigured() {
  return Boolean(
    process.env.YUNKUO_SIGN_THUMBPRINT || process.env.YUNKUO_SIGN_PFX,
  );
}

/** PE 安全目录（数据目录第 5 项）非空，即已嵌入 Authenticode 签名。 */
export function hasSignature(file) {
  const fd = openSync(file, "r");
  try {
    const head = Buffer.alloc(4096);
    const read = readSync(fd, head, 0, head.length, 0);
    if (read < 0x40 || head.toString("latin1", 0, 2) !== "MZ") return false;
    const pe = head.readUInt32LE(0x3c);
    if (pe + 26 > read || head.toString("latin1", pe, pe + 4) !== "PE\0\0")
      return false;
    const optional = pe + 24;
    const plus = head.readUInt16LE(optional) === 0x20b;
    const count = optional + (plus ? 108 : 92);
    const security = optional + (plus ? 112 : 96) + 4 * 8;
    if (security + 8 > read || head.readUInt32LE(count) < 5) return false;
    return head.readUInt32LE(security + 4) > 0;
  } finally {
    closeSync(fd);
  }
}

function signtool() {
  const configured =
    process.env.YUNKUO_SIGNTOOL || process.env.TAURI_WINDOWS_SIGNTOOL_PATH;
  if (configured) return configured;
  const kits = path.join(
    process.env["ProgramFiles(x86)"] || "C:\\Program Files (x86)",
    "Windows Kits",
    "10",
    "bin",
  );
  const versions = existsSync(kits)
    ? readdirSync(kits).filter(
        (v) =>
          /^10\.[\d.]+$/.test(v) &&
          existsSync(path.join(kits, v, "x64", "signtool.exe")),
      )
    : [];
  const newest = versions
    .sort((a, b) => {
      const x = a.split(".").map(Number),
        y = b.split(".").map(Number);
      return x.reduce((order, part, i) => order || part - (y[i] ?? 0), 0);
    })
    .at(-1);
  if (!newest)
    throw new Error(
      "找不到 signtool.exe，请安装 Windows SDK 或设置 YUNKUO_SIGNTOOL",
    );
  return path.join(kits, newest, "x64", "signtool.exe");
}

export function signFile(file) {
  if (!signingConfigured()) throw new Error("未配置代码签名证书");
  const args = [
    "sign",
    "/fd",
    "sha256",
    "/tr",
    process.env.YUNKUO_SIGN_TIMESTAMP_URL || "http://timestamp.digicert.com",
    "/td",
    "sha256",
    "/d",
    product,
  ];
  if (process.env.YUNKUO_SIGN_PFX)
    args.push(
      "/f",
      process.env.YUNKUO_SIGN_PFX,
      "/p",
      process.env.YUNKUO_SIGN_PFX_PASSWORD ?? "",
    );
  else {
    args.push("/sha1", process.env.YUNKUO_SIGN_THUMBPRINT);
    if (process.env.YUNKUO_SIGN_MACHINE_STORE === "1") args.push("/sm");
  }
  args.push(file);
  const tool = signtool();
  for (let attempt = 1; ; attempt++) {
    const result = spawnSync(tool, args, {
      encoding: "utf8",
      windowsHide: true,
    });
    if (result.status === 0) return;
    // 输出只含 signtool 的结果，不回显参数，避免泄露 PFX 密码。
    if (attempt === 3)
      throw new Error(
        `签名失败：${path.basename(file)}\n${`${result.stdout ?? ""}${result.stderr ?? ""}`.trim()}`,
      );
    // 时间戳服务偶发超时，稍后重试。
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 3000);
  }
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href
) {
  const file = process.argv[2] && path.resolve(process.argv[2]);
  if (!file || !existsSync(file)) {
    console.error("用法：node scripts/sign-windows.mjs <待签名文件>");
    process.exit(2);
  }
  const name = path.basename(file);
  if (file.toLowerCase().startsWith(resources.toLowerCase())) {
    console.log(`跳过第三方组件（哈希已固定）：${name}`);
  } else if (hasSignature(file)) {
    console.log(`已签名，跳过：${name}`);
  } else if (/^yunkuo-service/i.test(name)) {
    console.error(
      "服务组件尚未签名，而主程序已固定它的哈希；此时签名会让安全服务安装校验失败。请在配置证书后运行 pnpm service:prepare，再重新构建。",
    );
    process.exit(1);
  } else if (!signingConfigured()) {
    console.error(
      "未配置代码签名证书：请设置 YUNKUO_SIGN_THUMBPRINT，或 YUNKUO_SIGN_PFX 与 YUNKUO_SIGN_PFX_PASSWORD。只需本地测试包时使用 pnpm tauri build --no-sign。",
    );
    process.exit(1);
  } else {
    try {
      signFile(file);
      console.log(`已签名：${name}`);
    } catch (error) {
      console.error(error.message);
      process.exit(1);
    }
  }
}
