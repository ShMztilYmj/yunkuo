// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // 卸载钩子在删除文件前调用；不初始化窗口，主程序界面损坏时也能运行。
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--uninstall-cleanup")) {
        std::process::exit(yunkuo_lib::uninstall_cleanup(
            std::env::args_os().nth(2).map(Into::into),
        ));
    }
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("--diagnose-claude-resume"))
    {
        if let Some(output) = std::env::args_os().nth(2) {
            yunkuo_lib::diagnose_claude_resume(output.into());
        }
        return;
    }
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("--diagnose-codex-resume"))
    {
        if let Some(output) = std::env::args_os().nth(2) {
            yunkuo_lib::diagnose_codex_resume(output.into());
        }
        return;
    }
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("--self-test-agent-engine"))
    {
        if yunkuo_lib::verify_agent_usage_engine().is_err() {
            std::process::exit(1);
        }
        return;
    }
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("--install-agent-monitor"))
    {
        if yunkuo_lib::install_agent_monitor().is_err() {
            std::process::exit(1);
        }
        return;
    }
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("--self-test-crash-journal"))
    {
        if let Some(root) = std::env::args_os().nth(2) {
            yunkuo_lib::crash_journal_probe(root.into());
        }
        std::process::exit(2);
    }
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("--self-test-state-registration"))
    {
        yunkuo_lib::verify_managed_states();
        return;
    }
    yunkuo_lib::initialize_fault_reporting();
    yunkuo_lib::run()
}
