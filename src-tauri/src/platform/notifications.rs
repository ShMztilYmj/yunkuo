use parking_lot::Mutex;
use serde::Serialize;
use std::collections::VecDeque;
use tauri::{Emitter, Manager, State};

/// 提示等级：消息中心据此分组、配色和排序。
///
/// 这个队列收的是“需要人看一眼”的上报，所以 `report` 默认按错误记；
/// 兜底、历史记录这类“已经降级但仍可用”的上报由调用点显式点名 `Warn`，
/// 纯告知（例如兼容目录回退）点名 `Info`。等级不靠措辞猜测，
/// 免得真故障被字符串匹配悄悄降级。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Error,
    Warn,
    Info,
}

#[derive(Clone, Serialize)]
pub struct Notice {
    id: u64,
    level: Level,
    message: String,
}

#[derive(Default)]
pub struct Errors {
    next_id: u64,
    pending: VecDeque<Notice>,
}
pub type ErrorState = Mutex<Errors>;

impl Errors {
    fn push(&mut self, level: Level, message: String) {
        self.next_id += 1;
        self.pending.push_back(Notice {
            id: self.next_id,
            level,
            message,
        });
        while self.pending.len() > 20 {
            self.pending.pop_front();
        }
    }
}

/// 默认入口：没点名等级的上报按错误记，与改造前的行为一致。
pub fn report(app: &tauri::AppHandle, message: String) {
    report_as(app, Level::Error, message);
}

/// 显式定级：兜底、历史记录、兼容回退这类上报不该和真故障混在一起。
pub fn report_as(app: &tauri::AppHandle, level: Level, message: String) {
    app.state::<ErrorState>()
        .lock()
        .push(level, message.clone());
    let _ = app.emit("core-error", message);
}

#[tauri::command]
pub fn get_core_errors(state: State<'_, ErrorState>) -> Vec<Notice> {
    state.lock().pending.iter().cloned().collect()
}

#[tauri::command]
pub fn dismiss_core_error(state: State<'_, ErrorState>, id: u64) {
    state.lock().pending.retain(|error| error.id != id);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn errors_remain_until_acknowledged_and_queue_is_bounded() {
        let mut errors = Errors::default();
        for i in 0..25 {
            errors.push(Level::Error, format!("fixture-{i}"));
        }
        assert_eq!(errors.pending.len(), 20);
        let id = errors.pending.back().unwrap().id;
        assert_eq!(errors.pending.back().unwrap().message, "fixture-24");
        assert_eq!(errors.pending.back().unwrap().level, Level::Error);
        errors.pending.retain(|notice| notice.id != id);
        assert_eq!(errors.pending.len(), 19);
        assert!(!errors.pending.iter().any(|notice| notice.id == id));
    }

    #[test]
    fn notices_keep_the_level_they_were_reported_with() {
        let mut errors = Errors::default();
        errors.push(Level::Error, "代理配置未应用：保存失败".into());
        errors.push(Level::Warn, "上次程序未正常结束".into());
        errors.push(Level::Info, "WebView2 已回退到兼容数据目录".into());
        let levels: Vec<_> = errors.pending.iter().map(|notice| notice.level).collect();
        assert_eq!(levels, vec![Level::Error, Level::Warn, Level::Info]);
        let serialized: Vec<_> = errors
            .pending
            .iter()
            .map(|notice| serde_json::to_value(notice).unwrap()["level"].clone())
            .collect();
        assert_eq!(serialized, vec!["error", "warn", "info"]);
    }
}
