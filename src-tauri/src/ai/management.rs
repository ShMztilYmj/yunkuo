//! AI 管理的公共入口：存储就绪检查、操作互斥与输入校验。
//!
//! 工具启动、能力库同步与用量扫描共用这一层，避免各模块各写一份互斥与校验。
use anyhow::Result;

#[derive(Default)]
pub struct State {
    pub(crate) operation: tokio::sync::Mutex<()>,
}

/// 资料目录还没恢复正常时，任何写入都会制造第二个真相，所以统一在这里拦下。
pub(crate) fn require_ready(root: &std::path::Path) -> Result<()> {
    crate::platform::state::require_storage_ready(root)
}
