//! 指纹浏览器：配置、隔离资料目录、会话与默认浏览器关联。
//!
//! 启动资格由消费者定义的 LaunchGate 注入，浏览器不读取检测模块的内部状态。
pub(crate) mod bookmarks;
pub(crate) mod browser;
pub(crate) mod defaultbrowser;
pub(crate) mod pinned;
pub(crate) mod selfcheck;
