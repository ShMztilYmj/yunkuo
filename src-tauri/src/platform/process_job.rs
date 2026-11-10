//! 子进程树清理：把进程放进 Job Object，句柄关闭时整棵树一起结束。
//!
//! `attach` 按进程号重新打开句柄，用于已经接管子进程的异步运行时。

use anyhow::{Context, Result};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use windows::Win32::{
    Foundation::HANDLE,
    System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    },
    System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE},
};

/// 能提供进程号的子进程；std 与 tokio 的实现都只返回启动时的进程号。
pub(crate) trait Spawned {
    fn spawn_id(&self) -> Option<u32>;
}

impl Spawned for std::process::Child {
    fn spawn_id(&self) -> Option<u32> {
        Some(self.id())
    }
}

impl Spawned for tokio::process::Child {
    fn spawn_id(&self) -> Option<u32> {
        self.id()
    }
}

/// 持有即生效：Drop 时关闭句柄，Windows 结束 Job 内的整棵进程树。
pub(crate) struct ProcessJob(OwnedHandle);

impl ProcessJob {
    /// 按进程号重新打开句柄再放进 Job Object。
    pub(crate) fn attach(child: &impl Spawned) -> Result<Self> {
        let pid = child.spawn_id().context("进程已退出")?;
        let handle = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid) }?;
        let owned = unsafe { OwnedHandle::from_raw_handle(handle.0) };
        Self::assign(HANDLE(owned.as_raw_handle()))
    }

    fn assign(process: HANDLE) -> Result<Self> {
        unsafe {
            let raw = CreateJobObjectW(None, None)?;
            let job = Self(OwnedHandle::from_raw_handle(raw.0));
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                raw,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as _,
                std::mem::size_of_val(&limits) as u32,
            )?;
            AssignProcessToJobObject(HANDLE(job.0.as_raw_handle()), process)?;
            Ok(job)
        }
    }
}
