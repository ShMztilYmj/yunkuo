//! 通用平台机制：文件、受保护存储、归档、进程、系统时间、通知与状态锁。

pub(crate) mod archive_codec;
pub(crate) mod backup_crypto;
pub(crate) mod digest;
pub(crate) mod files;
pub(crate) mod notifications;
pub(crate) mod process_job;
pub(crate) mod startup;
pub(crate) mod state;
pub(crate) mod system_time;

pub(crate) mod secure_storage;
