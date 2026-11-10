# CCBaga 功能整合

## Codex 自动续跑

参考并改写 songyb111-gachon/codex-auto-resume-windows 的实时额度判定、只读会话识别、Windows Restart Manager 加载探测、定向消息队列及持久化去重机制。固定参考提交 `506ef16eaca0245629946161969a5bd1a48bbf7e`，来源：https://github.com/songyb111-gachon/codex-auto-resume-windows 。Copyright (c) 2026 songyb111-gachon，MIT，完整许可见 `licenses/Codex-Auto-Resume-MIT.txt`。本项目在 Rust/Tauri 与 React 中集成，不分发上游独立 GUI 或 Python 运行时。

上游：https://github.com/liseami/CCBaga

参考版本：30a171f5191fbb65dc24e4b1a0392d998f7c5719。

身份归档认证流 `src-tauri/src/environment/codec.rs` 与账号键白名单 `identity_keys.rs` 来自或改编自 CCBaga，Copyright (c) 2026 CCBaga contributors，MIT；完整许可见 `licenses/CCBaga-MIT.txt`。浏览器参数和环境核验流程参考该项目，并针对本项目的 Windows/原生代理/WFP 架构实现。

可选 Fingerprint Chromium 引擎来自 https://github.com/adryfish/fingerprint-chromium ，使用固定版本和 SHA-256 校验；BSD-3-Clause 许可见 `licenses/Fingerprint-Chromium-BSD.txt`，浏览器自身的 Chromium/其他第三方声明随其目录保留。

指纹浏览器随包预置扩展 WebRTC Control 0.3.5（Chrome 应用商店 item id `fjkmabmdepjfammlpliljpnbhleegehm`，来源 https://mybrowseraddon.com/webrtc-control.html ），原样分发、未修改，位于 `src-tauri/resources/extensions/webrtc-control`，启动时以 `--load-extension` 装载。版权归原作者所有，按其自身条款使用；如需移除，删除该目录并同步 `tauri.conf.json` 的 resources 条目即可。

本软件的 GPL-3.0 及其他第三方许可保持各自适用范围。0.2.11 恢复包含 mihomo v1.19.32，作为独立进程运行；来源 https://github.com/MetaCubeX/mihomo ，采用 GPL-3.0，完整许可见 `licenses/Mihomo-GPL-3.0.txt`。固定 SHA-256：`04F8D7FC2B314771E1ECBF15951D59F0E1CB7914503C88CFB6972FF6A824E314`。0.2.1 起不再分发旧 Firefox 引擎；源码目录或旧安装中的历史 Firefox 文件仍受原许可约束。

## Agent HUD 统计与会话设计

上游：https://github.com/jazzenchen/agent-hud-open

参考版本：7ddd27e9e4d75bb627f2e8fb7498b9166238dc1b。Apache-2.0，完整许可见 `licenses/AgentHUD-Apache-2.0.txt`。本项目参考其本地优先的统计口径、额度快照有效性与明确事件驱动的会话生命周期规则，以 Rust/React 实现 Windows 版本，主要位于 `src-tauri/src/aimon/activity.rs`、`src-tauri/src/ai_monitor.rs` 与 `src/services/aiStats.ts`。未移植其 Swift 窗口系统或 Clawd 精灵图。

## Agent Bot 动效图标

`public/agent-bots/` 内置 Claude Code、Codex、Cursor、Antigravity、OpenCode、Kimi Code、Grok Build、ZCode、DeepSeek Harness 九款角色及唤醒、工作、等待确认、完成四种状态。角色几何与动作时间轴为本项目重新绘制，动作参考上述 Agent HUD 演示；唤醒开场的品牌标志使用该版本随附的 LobeHub 图标资源（部分调整配色），Copyright (c) 2023 LobeHub，MIT，完整许可见 `licenses/LobeHub-Icons-MIT.txt`。品牌标志与名称仍归各自所有者。本软件使用预渲染 WebM 和静态 SVG，不包含 GSAP 运行时代码。

Windows 原生完成通知通过 Tauri notification 插件（MIT / Apache-2.0）提供。

## 本地 SQLite 用量读取

新增 rusqlite 0.40.2（MIT，完整许可见 `licenses/Rusqlite-MIT.txt`）及 bundled SQLite（public domain）用于只读 OpenCode / ZCode 本地数据库；未移植这些产品的界面或会话内容。来源：https://github.com/rusqlite/rusqlite 、https://www.sqlite.org/copyright.html 。


## Tokscale / Token Monitor 用量引擎

- 原项目：https://github.com/junhoyeo/tokscale
- 随附分支：https://github.com/Javis603/tokscale
- Token Monitor 参考：https://github.com/Javis603/token-monitor
- 固定源码：324d51362201b746fc35c2f2a3793c971f01d5aa（4.20.0）
- 固定 release：token-monitor-324d5136，Windows x64 SHA-256 887113d6adeb4aa15bb298e80d316a1dc1c961d4aa469cec64533ac606b9d173。
- MIT，Copyright (c) 2025 Junho Yeo，完整文本见 licenses/Tokscale-MIT.txt。
- 作为独立随附可执行文件读取标准 JSON 报告，统一九款 Agent 的来源归属、Token、会话数及每日趋势，并查询已登录供应商的额度。不调用 submit、GitHub 登录、模型对话或社交上传功能。
- Claude 自动续跑的实时额度适配另行参考同一固定提交的 `crates/tokscale-cli/src/commands/usage/claude.rs`，在 Rust/Tauri 后台读取 Claude 官方 OAuth usage 响应，不调用其他供应商；完整 MIT 许可同上。会话调度、审批中止及界面集成为本项目实现。


## Chrono-TZ

Rust chrono-tz 0.10.4 用于按 tokscale 已固定的 IANA 统计时区计算会话活跃日，防止时区切换造成日期错配。项目：https://github.com/chronotope/chrono-tz 。许可全文见 licenses/Chrono-TZ.txt。


## 原生代理与 Windows TUN

历史实验代理协议库使用 https://github.com/meow-rs/meow-rs 的固定 0.26.1 版本（MIT），历史实验依赖由其独立 Cargo.lock 固定；0.2.11 的主程序和服务均不链接 meow。`src-tauri/crates/native-listener` 与 `src-tauri/crates/native-dns` 分别基于同版本 meow-listener / meow-dns 修改，保留包身份和原始 MIT 许可；修改范围见各目录 ORIGIN.md。本软件实现自己的原生生命周期、内存控制入口、TCP DNS 传输、TUN TCP DNS 接管及 Windows DNS 持久恢复，不声称从零编写上游协议。完整许可见 `licenses/Meow-rs-MIT.txt`。

历史原生实验的 Windows TUN 内嵌官方 Wintun 0.14.1，来源 https://www.wintun.net/ ，固定下载及 SHA-256 见 `scripts/native-build-tools.json`，许可见 `licenses/Wintun.txt`。

Country.mmdb 与 ASN.mmdb 使用 P3TERX/GeoLite.mmdb 的 2026.10.22 发布（https://github.com/P3TERX/GeoLite.mmdb/releases/tag/2026.10.22），固定哈希见 `scripts/native-geodata.json`。包含 MaxMind 的 GeoLite2 数据（Database and Contents Copyright MaxMind, Inc.），保留来源与数据库元信息；上游列出的条款为 https://www.maxmind.com/en/geolite2/eula 和 https://creativecommons.org/licenses/by-sa/4.0/ ，参见 https://github.com/P3TERX/GeoLite.mmdb#license 。原有 geosite.dat 继续沿用其上游声明。
