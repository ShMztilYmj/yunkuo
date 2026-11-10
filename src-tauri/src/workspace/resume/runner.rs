//! 两个驱动共用的守候记录落库逻辑。
//!
//! 各驱动只决定「这是不是一次可续的额度失败」（判定留在各自的 scheduler），
//! 落库、去重与重新守候的字段变更在这里统一，避免两边再次漂移。
use super::{model::*, store::Store};
use anyhow::Result;

/// 一次扫描要写进库里的守候骨架。
pub struct Observation<'a> {
    pub id: String,
    pub thread_id: &'a str,
    pub turn_id: String,
    pub ordinal: i64,
    pub title: &'a str,
    pub project: &'a str,
    pub reason: &'a str,
    pub marker: String,
    pub rearm_reason: &'static str,
    pub quota_confirmed: bool,
    /// 重新守候时是否重建额度快照：claude 需要重新核验真实额度，codex 的额度来自回执。
    pub reset_quota: bool,
}

/// 已存在就只处理"重新守候"，不存在才插入。
pub fn observe(
    store: &Store,
    watch: &Watch,
    at: i64,
    rearm: bool,
    obs: Observation<'_>,
) -> Result<()> {
    if let Some(mut r) = store.record(&obs.id)? {
        if rearm
            && r.claimed_at.is_none()
            && matches!(r.state.as_str(), "cancelled" | "expired" | "superseded")
        {
            r.account = watch.account.clone();
            r.configuration = watch.configuration.clone();
            r.cancel_requested = false;
            r.created_at = at;
            if obs.reset_quota {
                r.marker = obs.marker;
                r.quota_confirmed = obs.quota_confirmed;
                r.reset_at = None;
                r.checked_at = None;
            }
            r.set("waiting_quota", obs.rearm_reason, at, 0);
            store.update(&r)?;
        }
        return Ok(());
    }
    store.insert(&Recovery {
        id: obs.id,
        thread_id: obs.thread_id.into(),
        home_key: watch.home_key.clone(),
        turn_id: obs.turn_id,
        ordinal: obs.ordinal,
        account: watch.account.clone(),
        configuration: watch.configuration.clone(),
        title: obs.title.into(),
        project: obs.project.into(),
        state: "waiting_quota".into(),
        reason: obs.reason.into(),
        marker: obs.marker,
        reset_at: None,
        next_check: at,
        checked_at: None,
        created_at: at,
        updated_at: at,
        claimed_at: None,
        queue_id: None,
        cancel_requested: false,
        spawn_failures: 0,
        quota_confirmed: obs.quota_confirmed,
    })?;
    Ok(())
}
