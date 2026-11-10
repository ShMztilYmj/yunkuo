# 参与贡献

感谢你愿意参与。云阔是一个只能运行在 Windows 上的桌面应用（Tauri 2 + React + Rust），
仓库不含任何云服务端代码，所有逻辑都在本机进程与随包服务里。

## 环境要求

| 组件 | 版本 | 说明 |
| --- | --- | --- |
| Windows | 10 1809+ / 11，x64 | 依赖 WFP、Wintun、WebView2，没有 macOS/Linux 构建 |
| Node.js | 24 | 与 `.github/workflows/ci.yml` 一致 |
| pnpm | 12.11.2 | `corepack enable` 或 `npm i -g pnpm@12.11.2` |
| Rust | stable（edition 2024） | `rustup default stable`，需 MSVC 工具链 |
| WebView2 | 系统自带 | 缺省由安装包内嵌的 bootstrapper 安装 |
| PowerShell | 5.1 | 构建脚本按 5.1 编写，不依赖 PowerShell 7 |

## 准备随包资源（仓库里没有）

为控制仓库体积，三项第三方资源不入 Git，需要本地放置后才能完整运行与打包：

| 路径 | 内容 | 来源与固定版本 |
| --- | --- | --- |
| `src-tauri/resources/fingerprint-chromium/` | 约 425 MB，需含 `chrome.exe` | https://github.com/adryfish/fingerprint-chromium，SHA-256 校验见 THIRD-PARTY-NOTICES.md |
| `src-tauri/binaries/云阔mihomo-x86_64-pc-windows-msvc.exe` | 约 58 MB | https://github.com/MetaCubeX/mihomo v1.19.32，SHA-256 `04F8D7FC…824E314`，见 THIRD-PARTY-NOTICES.md |
| `src-tauri/resources/geodata`、`resources/tokscale` | 规则库与用量引擎 | 版本与哈希见 THIRD-PARTY-NOTICES.md、`scripts/native-geodata.json`、`scripts/native-build-tools.json` |

服务侧车 `src-tauri/binaries/yunkuo-service-*.exe` 不需要手工放，`pnpm service:prepare` 会编译并复制。

只跑前端和单元测试时，上述资源缺失不影响；执行 `pnpm tauri dev`、打包或涉及指纹浏览器/内核的用例时才需要。

## 常用命令

```powershell
pnpm install --frozen-lockfile
pnpm tauri dev          # 开发，前端热更新在 11420，Tauri 在 11421
pnpm verify             # 提交前最低门槛：脚本契约 + 类型检查 + 前端测试 + Rust 测试
pnpm test               # 只跑 Rust 单元测试（串行）
pnpm test:frontend      # 只跑 tests/*.test.mjs
pnpm format             # cargo fmt + prettier --write
```

`pnpm verify` 失败不要提 PR；CI 执行同一组命令。Rust 用例默认串行，会占用真实端口并拉起真实进程。

## 代码地图

| 目录 | 内容 |
| --- | --- |
| `src/app/` | 应用外壳、路由、设置页、标题栏、托盘与悬浮条前端 |
| `src/features/{proxy,ai,diagnostics,fingerprint-browser}/` | 四个业务模块的页面与组件 |
| `src/shared/ui/` | 通用组件（对话框、提示、错误边界） |
| `src-tauri/src/app/` | 命令注册、启动编排、托盘、迁移 |
| `src-tauri/src/proxy/` | 内核生命周期、控制器 API、TUN/WFP、订阅覆盖 |
| `src-tauri/src/ai/` | 账号、额度、用量引擎、Agent 监控 |
| `src-tauri/src/service_host/`、`src/bin/yunkuo-service.rs` | 本机服务侧车（127.0.0.1:33215 等） |
| `src-tauri/crates/` | 独立 crate：`protection`、`mihomo-runtime`、`native-*` |

## 提交规范

- 提交信息用中文或英文均可，建议 `模块: 做了什么`，例如 `proxy: 修复 TUN 关闭后残留路由`。
- 一个 PR 只做一件事；行为变更请补对应测试（`tests/*.test.mjs` 或 Rust `#[cfg(test)]`）。
- 提交前跑 `pnpm format` 与 `pnpm verify`；CI 会检查 `cargo fmt --check` 与 prettier。
- 不要提交构建产物、`release/` 目录、签名证书、个人配置文件。

## 分支与发布

- `main` 保持可构建；功能分支命名 `feat/…`、`fix/…`、`docs/…`。
- 版本号在 `package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json` 三处同步。
- 发布由维护者打 `vX.Y.Z` 标签并附构建产物说明，细节见 README「开发与构建」。

## 行为准则

参与即表示同意保持友善、就事论事。人身攻击、骚扰、泄露他人账号或密钥的内容会被移除并封禁。
