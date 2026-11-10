//! 断点续跑。
//!
//! Claude Code 与 Codex 共用同一套数据模型、存储与秒级时间戳，两个调度器各自保留
//! 与自家 CLI 协议的差异。数据库文件名 `codex-resume.sqlite` 属于已安装用户的存量数据，
//! 不论模块怎么搬都不能改。

pub(crate) mod claude;
pub(crate) mod codex;
pub(crate) mod model;
pub(crate) mod runner;
pub(crate) mod store;
pub(crate) mod workflow;

/// 秒级时间戳，两个续跑调度器共用。
pub(crate) fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// 只放行本模块自己构造的短中文消息。
///
/// 两边的黑名单原本不一致（claude 挡 `Bearer`、codex 挡 `token`），
/// 于是同一类路径/凭据泄漏总有一侧漏网；这里取并集并统一长度上限。
pub(crate) fn public_error(error: &anyhow::Error, fallback: &'static str) -> String {
    let text = error.to_string();
    let safe = text.chars().count() <= 100
        && text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
        && !["Bearer", "token", "password", "sk-", ":\\"]
            .iter()
            .any(|needle| text.contains(needle));
    if safe { text } else { fallback.into() }
}
