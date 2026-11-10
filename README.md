# 云阔

[![CI](https://github.com/OWNER/yunkuo/actions/workflows/ci.yml/badge.svg)](https://github.com/OWNER/yunkuo/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

Windows 桌面应用，使用 React、Vite 和 Tauri 2。仅支持 Windows 10 1809+ / 11 x64。

本机一体化的代理与环境工作台：内核托管与 TUN、指纹浏览器、AI 工具账号与用量、系统环境检测与修复，
全部在本机进程内完成，不依赖任何云端服务。

参与开发前请先读 [CONTRIBUTING.md](CONTRIBUTING.md)——随包的 Fingerprint Chromium、mihomo 内核与规则库不入库，
用 `pnpm fetch:vendor` 按固定版本拉取。安全漏洞请按 [SECURITY.md](SECURITY.md) 私密上报，不要开公开 issue。

## 快速开始

```powershell
pnpm install --frozen-lockfile
pnpm fetch:vendor            # mihomo 内核 + 用量引擎 + 规则库（约 90 MB）
pnpm tauri dev               # 需要指纹浏览器功能时再加 --with-browser
pnpm verify                  # 提交前最低门槛
```

打包安装包前先确认 `pnpm fetch:vendor:check` 全部就绪，再执行 `pnpm tauri build`。

主窗口最小 **1140 × 700**（`src-tauri/tauri.conf.json` 的 `minWidth` / `minHeight`）：低于这个宽度时，能力库页面的表格（最小 850 px）加上侧边栏（218 px）和页边距就会横向滚动，所以窗口不允许再缩。窗口尺寸按逻辑像素计算，125% 缩放下相当于约 1430 物理像素宽。

混合代理端口沿用 Clash / Codex `.env` 生态惯例的 7890，其余固定本地端口统一落在高位：

| 用途                | 端口          |
| ------------------- | ------------- |
| HTTP/SOCKS 混合代理 | 7890          |
| 内核控制器          | 19097         |
| 本地 DNS            | 11053         |
| 开发页面 / 热更新   | 11420 / 11421 |
| 安全服务            | 33215         |
| AI 模型入口（本机） | 34327         |
| AI 内核执行器（内部） | 34325       |

AI 相关的两个端口分工：`34327` 是工具写入配置后指向的唯一入口，由应用自己处理同协议请求；`34325` 是随包的 Magpie 执行器（已裁剪掉内嵌面板与插件包），只接收应用转发过来的跨协议请求，二者都只监听 `127.0.0.1`。

启动时只迁移历史默认端口：混合代理 17897 或 7897 → 7890，控制器 9097 → 19097，空值同样回落到当前默认值；用户自定义端口（包括 8080 这类低位自定义值）保持原值。迁移后若两个端口相同，控制器端口顺延一位。写回的改动会保留 `config.bak`。设置页接受 1–65535，但不能与控制器端口相同。订阅中的本地 DNS 监听统一使用 11053。安全服务协议升级为 4.3，旧版本需在设置中安装或修复服务。预览、协作等临时服务继续由系统分配端口。

## 内核会话

路由方式（全局 / 规则 / 直连）由控制器 `PATCH /configs` 原地改写：不重建内核进程，现有连接保持，落盘配置照常更新，下次启动与崩溃恢复仍按当前路由。日志等级同理。

订阅更新、切换订阅或应用覆写时，只要变化只在节点、分组、provider 与规则内，就通过控制器 `PUT /configs` 原地重载：进程、TUN 与已建立连接都保留，界面在「订阅管理」页标注「原地热更新」。严格保护不走这条路——WFP 放行名单按当前节点端点冻结，换节点必须重建会话后重新冻结；网络方案切换、TUN 开关、混合端口与控制器端口、IPv6 与监听范围、DNS 落点或宿主变化也仍会重建会话。

受控覆写存放在数据目录的 `proxy-overlay.yaml`，只接受 `rules`（前插规则）、`proxy-groups`（追加分组）与 `dns.nameserver-policy`（补充解析策略）；订阅更新不会覆盖它，AI 域名与系统解析器不能在其中改写。严格保护下覆写会被拒绝。宿主是粘性的：首次开启 TUN 会把会话切到服务托管；之后关闭 TUN 只重建服务内的内核会话，不搬回应用进程，只有重新启动应用或显式释放后才回到应用内核。宿主变化同样换 PID，已建立的连接随之断开。内核完整性校验按文件指纹在进程内缓存，同一份内核不会在每次切换时重复读取 58 MB。概览页的「重启内核」始终强制换进程。

## 开发与构建

在项目根目录使用 Windows 自带的 PowerShell 5.1 即可（脚本不依赖 PowerShell 7）：

```powershell
pnpm install --frozen-lockfile
pnpm tauri dev
```

生成可独立启动的免安装目录：

```powershell
pnpm build:portable
```

脚本会先重建系统服务侧车（`pnpm service:prepare`），再构建主程序并校验“主程序内嵌的服务哈希 = 包内 sidecar 哈希”，最后复制资源生成新目录。顺序不能颠倒：`build.rs` 在编译早期就把侧车哈希写进主程序，侧车晚一步更新会产出不一致的包。

成品位于项目 `release` 目录，每次构建创建一个新目录。请保留主程序旁的两个服务程序、`resources` 和许可证文件。

生成安装包：

```powershell
pnpm tauri build
```

正式安装包沿用项目的证书签名配置。仅在本机测试未签名安装包时使用 `pnpm tauri build --no-sign`。

`pnpm build` 只生成网页资源。普通 `cargo build --release` 只编译 Rust，且未启用 Tauri 正式模式，生成的 exe 仍会连接 `http://localhost:11420`；开发服务器未运行时会显示 `ERR_CONNECTION_REFUSED`。桌面成品应通过上述 Tauri 构建入口生成，让前端页面内嵌到主程序中。

检查某个 exe 是否内嵌本次构建的前端入口：

```powershell
node scripts/check-embedded-frontend.mjs '完整的exe路径'
```

`scripts/build-portable.ps1 -SkipBuild` 可在已完成正式构建后重新组装免安装目录，仍会校验前端和组件。

## 验证

```powershell
pnpm verify          # 脚本契约 + 类型检查 + Rust 单元测试
pnpm test            # 只跑 Rust 单元测试
pnpm check:scripts   # 确认 package.json 的每个脚本都指向真实文件
pnpm test:ui-resources
pnpm test:hud       # 悬浮条范围切换、异常恢复、零在线工具与左右停靠
pnpm test:packaged   # 对最新免安装包做界面端到端检查
pnpm measure:ui -- -AppProcessId 12345 -Seconds 15 -Output artifacts/ui-performance/sample.json
```

`pnpm verify` 是提交前的最低门槛，CI（`.github/workflows/ci.yml`）执行同一组命令，另有一个手动触发的打包冒烟流程。

Rust 用例默认串行（`--test-threads=1`）：代理内核与协作用例会占用真实端口、拉起真实进程，并行跑会互相抢占导致偶发失败。需要真实 CLI 登录的用例已标记 `#[ignore]`，用 `pnpm test:live` 在准备好的机器上单独跑。

`test:ui-resources` 使用独立的无头浏览器，验证头像视频的释放与回退、托盘首次打开的事件订阅顺序和隐藏时停止刷新。`test:packaged` 用 Windows UI Automation 驱动真实打包窗口：核对概览页的“重启内核”入口、处理中状态和最终结果，并确认关闭悬浮条后窗口被销毁；它需要可交互桌面，只能在带桌面的 Windows 会话运行。采样命令只读取指定进程及其子进程的 CPU 和私有常驻内存，CPU 百分比按整机逻辑处理器数量归一化。

待机、完成状态使用静态头像；工作和等待确认状态在可见时播放动效。悬浮条关闭后释放原生窗口，重新启用时按需创建。托盘快捷面板在启动几秒后预建并隐藏复用，不再等到第一次右键才建 webview——WebView2 运行时被后台静默升级后，运行期新建 webview 会直接失败（HRESULT 0x80010108）。主窗口、托盘和悬浮条各自加载所需前端代码。

## 许可

本仓库源码采用 [MIT](LICENSE)。随包分发的第三方组件各自保留原许可：mihomo 为 GPL-3.0 且以独立进程运行，Fingerprint Chromium 为 BSD-3-Clause，另有 Wintun、Tokscale、Agent HUD、CCBaga 等，完整清单与版本哈希见 [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) 和 `licenses/`，随包资源的固定版本与 SHA-256 见 [scripts/vendor.lock.json](scripts/vendor.lock.json)。

二次分发时请连同 `licenses/`、`THIRD-PARTY-NOTICES.md` 与内核二进制一并提供，并保留 mihomo 的对应源码获取方式（https://github.com/MetaCubeX/mihomo）。

## 致谢

功能与交互参考了 CCBaga、codex-auto-resume-windows、Agent HUD、Tokscale、Fingerprint Chromium、mihomo 等上游项目，详见 [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)。

## 免责声明

- 本项目用于在本机管理网络代理、开发环境与自有账号的使用情况，**不提供**任何节点、订阅、账号，也不包含绕过计费或风控的功能。
- 使用本项目需自行遵守所在地法律法规，以及各上游服务（Anthropic、OpenAI、Cursor、DeepSeek 等）的服务条款；账号权益、风控与合规风险由使用者承担。
- 指纹浏览器、代理接管、TUN 等功能请仅用于自有环境、本地调试或已获授权的测试。
- 作者与上述公司无隶属、赞助或背书关系；因使用本项目产生的任何直接或间接损失，作者不承担责任。

## 商标

「云阔 / Yunkuo」是本项目的名称。Claude、Codex、Cursor、Antigravity、OpenCode、Kimi、Grok、DeepSeek、Windows 等名称与标识归各自所有者所有，
本项目仅在说明兼容性或引用上游资源时使用，不表示任何关联或背书。


