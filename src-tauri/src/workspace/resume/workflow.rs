//! 续跑循环的公共取数与节拍。
//!
//! 两个驱动在「怎么判定一次额度中断、怎么把消息送出去」上必须保持差异
//! （claude 看 transcript 证据 + CLI 管道，codex 看回执 + 队列撤回），
//! 这里只收拢循环本身相同的部分：挑哪条记录、下一跳睡多久、
//! 以及两条共用的安全策略——启动失败只能重试两次、结算后守候作废。

use super::{model::*, store::Store};
use anyhow::Result;

/// 本轮可以尝试的等待记录：未取消、未认领、且已到期。顺序与库内顺序一致。
pub fn due(records: &[Recovery], now: i64) -> impl Iterator<Item = &Recovery> {
    records
        .iter()
        .filter(move |r| r.waiting() && !r.cancel_requested && r.next_check <= now)
}

/// 已认领但还没有定论、且到了核实时刻的记录。重启后不能重发，只能靠证据结算。
pub fn due_in_flight(records: &[Recovery], now: i64) -> impl Iterator<Item = &Recovery> {
    records
        .iter()
        .filter(move |r| r.in_flight() && r.next_check <= now)
}

/// 下一跳间隔：取最近一条待办，夹在 [2, 10] 秒；没有待办时 10 秒。
/// `in_flight` 决定已认领的记录是否也参与排期（codex 需要，claude 由运行表驱动）。
pub fn next_delay(records: &[Recovery], now: i64, in_flight: bool) -> i64 {
    records
        .iter()
        .filter(|r| r.waiting() || (in_flight && r.in_flight()))
        .map(|r| r.next_check - now)
        .min()
        .unwrap_or(10)
        .clamp(2, 10)
}

/// 计划已超过窗口（默认 7 天）：必须重新选择会话，不能拿旧计划去跑新任务。
pub fn expired(record: &Recovery, now: i64, window: i64) -> bool {
    now - record.created_at > window
}

/// 标记该会话的守候已被消费：恢复只做一次，后续空闲不能被同一个守候再次触发。
pub fn consume_watch(store: &Store, record: &Recovery) -> Result<()> {
    if let Some(mut watch) = store.watch(&record.home_key, &record.thread_id)? {
        watch.consumed = true;
        store.put_watch(&watch)?;
    }
    Ok(())
}

/// 进程根本没起来的统一处置：这是唯一可重试的提交失败。
///
/// 前两次把认领退回等待并重新打开守候（用户仍可重选），第三次判死。
/// 已经写进会话的提交不能走这里——那只能核实，不能重发。
pub fn spawn_failed(
    store: &Store,
    record: &mut Recovery,
    retry_reason: &str,
    failed_reason: &str,
    now: i64,
    retry_delay: impl Fn(u32) -> i64,
) -> Result<()> {
    record.spawn_failures += 1;
    if record.spawn_failures < 3 {
        record.claimed_at = None;
        record.set(
            "waiting_quota",
            retry_reason,
            now,
            retry_delay(record.spawn_failures),
        );
        if let Some(mut watch) = store.watch(&record.home_key, &record.thread_id)? {
            watch.consumed = false;
            store.put_watch(&watch)?;
        }
    } else {
        record.set("failed", failed_reason, now, 0);
    }
    store.update(record)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(state: &str, next_check: i64, claimed_at: Option<i64>) -> Recovery {
        Recovery {
            id: uuid::Uuid::new_v4().to_string(),
            thread_id: uuid::Uuid::new_v4().to_string(),
            home_key: "fixture-home".into(),
            turn_id: uuid::Uuid::new_v4().to_string(),
            ordinal: 0,
            account: "a".into(),
            configuration: "c".into(),
            title: "示例".into(),
            project: "demo".into(),
            state: state.into(),
            reason: "等待".into(),
            marker: "m".into(),
            reset_at: None,
            next_check,
            checked_at: None,
            created_at: 100,
            updated_at: 100,
            claimed_at,
            queue_id: None,
            cancel_requested: false,
            spawn_failures: 0,
            quota_confirmed: false,
        }
    }

    #[test]
    fn only_due_unclaimed_records_are_attempted() {
        let until = record("waiting_quota", 130, None);
        let due_now = record("waiting_reset", 100, None);
        let boundary = record("waiting_quota", 100, None);
        let mut cancelled = record("waiting_quota", 90, None);
        cancelled.cancel_requested = true;
        let claimed = record("submission_unknown", 90, Some(95));
        let records = vec![until, due_now, boundary, cancelled, claimed];
        assert_eq!(due(&records, 100).count(), 2, "到期判定包含边界值");
        assert!(due(&records, 99).all(|r| r.state != "waiting_reset"));
    }

    #[test]
    fn in_flight_records_are_verified_on_their_own_schedule() {
        let running = record("running", 100, Some(95));
        let later = record("queued", 200, Some(95));
        let records = vec![running, later];
        assert_eq!(due_in_flight(&records, 100).count(), 1);
        assert_eq!(due(&records, 100).count(), 0, "已认领的记录不进尝试队列");
    }

    #[test]
    fn next_delay_follows_the_earliest_record_and_stays_within_bounds() {
        let records = vec![
            record("waiting_quota", 200, None),
            record("waiting_quota", 104, None),
        ];
        assert_eq!(next_delay(&records, 100, false), 4);
        // 最近的一条已经过期：不能立刻空转，至少睡 2 秒。
        let late = vec![record("waiting_quota", 50, None)];
        assert_eq!(next_delay(&late, 100, false), 2);
        // 最远的一条：不超过 10 秒，保证用户操作后能及时被看见。
        let far = vec![record("waiting_quota", 900, None)];
        assert_eq!(next_delay(&far, 100, false), 10);
        assert_eq!(next_delay(&[], 100, false), 10, "没有待办时按 10 秒");
    }

    #[test]
    fn in_flight_only_counts_when_the_driver_asks_for_it() {
        let records = vec![record("running", 103, Some(100))];
        assert_eq!(next_delay(&records, 100, false), 10);
        assert_eq!(next_delay(&records, 100, true), 3);
    }

    #[test]
    fn plans_older_than_the_window_expire_at_the_boundary() {
        let record = record("waiting_quota", 100, None);
        assert!(!expired(&record, 100 + 7 * 86400, 7 * 86400));
        assert!(expired(&record, 101 + 7 * 86400, 7 * 86400));
    }

    #[test]
    fn spawn_failure_retries_twice_reopens_the_watch_then_stops() {
        let dir = crate::test_support::FixtureDir::new();
        let store = Store::open(dir.path()).unwrap();
        let mut record = record("submitting", 100, Some(100));
        store
            .put_watch(&Watch {
                thread_id: record.thread_id.clone(),
                home_key: record.home_key.clone(),
                enabled: true,
                consumed: true,
                account: record.account.clone(),
                configuration: record.configuration.clone(),
                ..Default::default()
            })
            .unwrap();
        store.insert(&record).unwrap();
        spawn_failed(&store, &mut record, "稍后重试", "已停止", 200, |n| {
            n as i64
        })
        .unwrap();
        assert_eq!(record.state, "waiting_quota");
        assert!(record.claimed_at.is_none());
        assert_eq!(record.next_check, 201);
        assert!(
            !store
                .watch(&record.home_key, &record.thread_id)
                .unwrap()
                .unwrap()
                .consumed
        );
        spawn_failed(&store, &mut record, "稍后重试", "已停止", 300, |n| {
            n as i64
        })
        .unwrap();
        assert_eq!(record.state, "waiting_quota");
        assert_eq!(record.spawn_failures, 2);
        record.claimed_at = Some(300);
        spawn_failed(&store, &mut record, "稍后重试", "已停止", 400, |n| {
            n as i64
        })
        .unwrap();
        assert_eq!(record.state, "failed");
        assert_eq!(record.next_check, 400);
        assert_eq!(store.record(&record.id).unwrap().unwrap().state, "failed");
    }

    #[test]
    fn consuming_a_watch_leaves_other_threads_alone() {
        let dir = crate::test_support::FixtureDir::new();
        let store = Store::open(dir.path()).unwrap();
        let first = record("queued", 100, Some(100));
        let second = record("queued", 100, Some(100));
        for record in [&first, &second] {
            store
                .put_watch(&Watch {
                    thread_id: record.thread_id.clone(),
                    home_key: record.home_key.clone(),
                    enabled: true,
                    ..Default::default()
                })
                .unwrap();
        }
        consume_watch(&store, &first).unwrap();
        assert!(
            store
                .watch(&first.home_key, &first.thread_id)
                .unwrap()
                .unwrap()
                .consumed
        );
        assert!(
            !store
                .watch(&second.home_key, &second.thread_id)
                .unwrap()
                .unwrap()
                .consumed
        );
    }
}
