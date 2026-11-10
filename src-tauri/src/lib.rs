//! 云阔：五大业务模块，应用编排与通用平台机制。
mod ai;
mod app;
mod config;
mod diagnostics;
mod error;
mod fingerprint_browser;
mod platform;
mod proxy;
mod workspace;

pub(crate) use error::AppError;

pub use app::desktop::{
    crash_journal_probe, diagnose_claude_resume, diagnose_codex_resume, initialize_fault_reporting,
    install_agent_monitor, run, uninstall_cleanup, verify_agent_usage_engine,
    verify_managed_states,
};
pub(crate) use app::desktop::{data_dir, show_main, update_tray};
#[cfg(feature = "performance-probes")]
pub use app::performance_probe::run as run_ui_performance_probe;
#[cfg(test)]
pub(crate) use protection::test_support;
pub(crate) use protection::{faults, http, privacy, service_protocol};
pub(crate) use proxy::network_mode;
