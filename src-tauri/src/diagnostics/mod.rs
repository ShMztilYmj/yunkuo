//! 检测事实、报告有效性、系统环境检查与修复。
//!
//! 不依赖指纹浏览器内部实现。需要暂停浏览器再修改系统的顺序由应用编排负责。
pub(crate) mod commands;
pub(crate) mod computer;
pub(crate) mod detect;
pub(crate) mod dns_deep;
pub(crate) mod history;
pub(crate) mod report;
pub(crate) mod timezone;
