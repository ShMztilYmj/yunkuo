//! 命令错误的统一出口。
//!
//! 内部实现仍可继续返回 `String`（历史原因，且大量错误来自 anyhow 上下文）；
//! 命令边界把错误收敛成 `{ code, message }`：界面继续展示 `message`，
//! 需要重试、降级或分支处理时按 `code` 判断，不必再解析中文文案。

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppError {
    pub code: &'static str,
    pub message: String,
}

impl AppError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// 内部实现已经把上下文拼进 message，这里只补一个可编程的错误码。
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("internal", message)
    }

    /// 配置被拒绝：字段非法、状态不允许、需要走专用面板等。
    pub fn config(message: impl Into<String>) -> Self {
        Self::new("config.rejected", message)
    }

    /// 运行环境未就绪：内核、服务、TUN 组件或前置检查失败。
    pub fn runtime(message: impl Into<String>) -> Self {
        Self::new("runtime.failed", message)
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("AppError", 2)?;
        state.serialize_field("code", self.code)?;
        state.serialize_field("message", &self.message)?;
        state.end()
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for AppError {}

impl From<String> for AppError {
    fn from(message: String) -> Self {
        Self::internal(message)
    }
}

impl From<&str> for AppError {
    fn from(message: &str) -> Self {
        Self::internal(message)
    }
}

impl From<anyhow::Error> for AppError {
    fn from(error: anyhow::Error) -> Self {
        Self::internal(format!("{error:#}"))
    }
}

/// 让还在返回 `String` 的内部函数可以继续用 `?` 调用已迁移的命令。
impl From<AppError> for String {
    fn from(error: AppError) -> Self {
        error.message
    }
}

#[cfg(test)]
mod tests {
    use super::AppError;

    #[test]
    fn 序列化为代码与文案() {
        let value = serde_json::to_value(AppError::new("core.restart_failed", "内核未响应"))
            .expect("序列化失败");
        assert_eq!(value["code"], "core.restart_failed");
        assert_eq!(value["message"], "内核未响应");
    }

    #[test]
    fn 字符串错误归类为内部错误() {
        let error: AppError = String::from("服务未安装").into();
        assert_eq!(error.code(), "internal");
        assert_eq!(String::from(error), "服务未安装");
    }
}
