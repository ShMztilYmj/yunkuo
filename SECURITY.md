# 安全策略

## 报告漏洞

请不要开公开 issue。把细节发到 **security@TARGET_DOMAIN**（替换为维护者邮箱），或用 GitHub 的
[Private vulnerability reporting](https://docs.github.com/code-security/security-advisories/guidance-on-reporting-and-writing/privately-reporting-a-security-vulnerability)。

建议包含：受影响版本、复现步骤、预期与实际结果、日志或抓包（请先脱敏）。

我们会在 3 个工作日内确认收到，30 天内给出修复或缓解计划，并在修复发布后致谢（除非你要求匿名）。

## 范围

本仓库是纯本地运行的程序，没有云服务端。重点关注的攻击面：

- 本机 HTTP 服务：安全服务 `127.0.0.1:33215`、AI 入口 `34327`、内核执行器 `34325` 与控制器 `19097` 的鉴权、路径穿越、命令注入与跨源访问（CSRF/DNS rebinding）。
- 系统服务侧车 `yunkuo-service` 的本地提权路径、服务二进制完整性校验、WFP 规则与 TUN 生命周期。
- 凭据存储：AI 账号、身份密钥、订阅链接的落盘加密与内存残留（`platform/secure_storage.rs`、`ai/identity/vault.rs`）。
- 指纹浏览器的参数注入、扩展加载与提权下载路径。
- 任意文件写入/执行、未受控的 `Command` 调用、日志中的明文密钥。

## 不在范围

- 需要用户已开启系统级调试或已取得本机管理员权限后才有意义的攻击。
- 第三方组件的上游问题（mihomo、Fingerprint Chromium、Wintun、Tokscale 等），请直接反馈给上游，同时欢迎抄送我们。
- 绕过第三方服务（Anthropic、OpenAI 等）账号风控或服务条款的问题，不属于本项目的安全边界。
- 依赖项的版本过旧提示（请提 PR 或 Dependabot issue）。

## 支持版本

只对最新发布版本接受报告；更早版本请先升级复测。
