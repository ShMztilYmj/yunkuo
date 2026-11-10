//! 代理业务：订阅、内核、路由、DNS 与服务客户端。
//! 生命周期只在 `engine` 解释，宿主差异只在 `host` 解释；
//! 跨业务网络模式与配置应用由 app 编排。
pub(crate) mod api;
mod controller;
pub(crate) mod core;
mod engine;
pub(crate) mod explain;
mod host;
pub(crate) mod ipmap;
pub(crate) mod network_mode;
pub(crate) mod network_preflight;
pub(crate) mod overlay;
pub(crate) mod policy;
pub(crate) mod profile;
pub(crate) mod service;
pub(crate) mod service_install;
pub(crate) mod sysproxy;

mod windows_proxy;
