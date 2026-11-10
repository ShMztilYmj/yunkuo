#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    std::process::exit(yunkuo_lib::run_ui_performance_probe());
}
