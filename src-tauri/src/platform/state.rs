//! 跨模块共享的 Tauri 状态与路径。
//!
//! `ConfigUpdateMutex` 串行化所有会改写 config.yaml 的操作：命令、托盘、维护和账号页都要
//! 拿同一把锁，否则两条写入路径会互相覆盖。

use tauri::Manager;

/// 串行化配置读写的全局锁（Tauri 按类型区分托管状态）。
pub(crate) type ConfigUpdateMutex = tokio::sync::Mutex<()>;

/// 浏览器会话与系统环境变更共享串行化机制。它与配置锁是不同的托管类型。
#[derive(Default)]
pub(crate) struct EnvironmentOperation(tokio::sync::Mutex<()>);

impl EnvironmentOperation {
    pub(crate) async fn lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.0.lock().await
    }
}

/// 应用数据目录。所有配置文件、订阅和诊断目录都在它下面。
pub(crate) fn data_dir(app: &tauri::AppHandle) -> std::path::PathBuf {
    app.path().app_data_dir().expect("无法获取数据目录")
}

/// 共享存储恢复检查点尚未清理时，所有写入入口保持关闭。
pub(crate) fn require_storage_ready(root: &std::path::Path) -> anyhow::Result<()> {
    if root.join("maintenance-pending.coc").exists() {
        anyhow::bail!("配置恢复尚未完成，请重启应用完成自动回退后再操作");
    }
    Ok(())
}
