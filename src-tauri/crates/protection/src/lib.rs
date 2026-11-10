//! 云阔的网络保护内核：存储机制、故障日志、隐私策略、WFP 出站白名单与安全服务协议。
//!
//! 主程序与 `yunkuo-service` 共用这里的实现。服务程序过去用 `#[path]` 直接包含主程序的
//! 九个源文件，因此两边的代码只能靠人工保持一致；现在改为依赖本 crate，既不链接 Tauri，
//! 也能让服务和界面走同一份策略代码。
//!
//! 依赖边界：本 crate 只允许依赖基础库（serde / reqwest / windows）与标准库，不得引入
//! tauri、tokio 或 `team`，否则服务组件会连带链接主程序，体积基线就守不住了。

pub mod config;
pub mod faults;
pub mod http;
pub mod network_lock;
pub mod privacy;
pub mod privacy_service;
pub mod service_protocol;
/// 测试用的临时目录夹具。主程序的 `#[cfg(test)]` 模块也通过 `crate::test_support` 复用它，
/// 两边只有这一份实现；发布构建里没有调用点，链接时会被丢掉。
#[doc(hidden)]
pub mod test_support;
