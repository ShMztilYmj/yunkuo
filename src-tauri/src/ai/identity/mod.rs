//! 账号与本地身份：Codex 账号存储、Claude 本地身份快照。
//!
//! 不同产品空间的账号各自独立保存，同一个邮箱不代表同一个认证主体。

pub(crate) mod account_store;
pub(crate) mod identity_keys;
pub(crate) mod vault;

/// 身份变更消费者的执行钩子，由应用编排装配；AI 不依赖工作台实现。
/// 回调必须同步封锁新续跑并取消活动轮次，才能允许替换身份文件。
pub(crate) struct ExecutionHooks {
    pause_claude: fn(&tauri::AppHandle) -> Result<(), String>,
    pause_codex: fn(&tauri::AppHandle) -> Result<(), String>,
    ensure_environment_closed: fn(&tauri::AppHandle) -> anyhow::Result<()>,
    invalidate_environment: fn(&tauri::AppHandle),
}
impl ExecutionHooks {
    pub(crate) fn new(
        pause_claude: fn(&tauri::AppHandle) -> Result<(), String>,
        pause_codex: fn(&tauri::AppHandle) -> Result<(), String>,
        ensure_environment_closed: fn(&tauri::AppHandle) -> anyhow::Result<()>,
        invalidate_environment: fn(&tauri::AppHandle),
    ) -> Self {
        Self {
            pause_claude,
            pause_codex,
            ensure_environment_closed,
            invalidate_environment,
        }
    }
    pub(crate) fn pause_claude(app: &tauri::AppHandle) -> Result<(), String> {
        use tauri::Manager;
        let hooks = app
            .try_state::<Self>()
            .ok_or_else(|| "身份执行保护未装配，未替换身份资料".to_owned())?;
        (hooks.pause_claude)(app)
    }
    pub(crate) fn pause_codex(app: &tauri::AppHandle) -> Result<(), String> {
        use tauri::Manager;
        let hooks = app
            .try_state::<Self>()
            .ok_or_else(|| "账号执行保护未装配，未替换账号资料".to_owned())?;
        (hooks.pause_codex)(app)
    }

    pub(crate) fn ensure_environment_closed(app: &tauri::AppHandle) -> anyhow::Result<()> {
        use tauri::Manager;
        let hooks = app
            .try_state::<Self>()
            .ok_or_else(|| anyhow::anyhow!("身份环境保护未装配"))?;
        (hooks.ensure_environment_closed)(app)
    }

    pub(crate) fn invalidate_environment(app: &tauri::AppHandle) {
        use tauri::Manager;
        if let Some(hooks) = app.try_state::<Self>() {
            (hooks.invalidate_environment)(app);
        }
    }
}
