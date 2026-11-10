//! Service-owned mihomo runtime; PID identifies the supervised child process.
//! Kept outside src/bin so the bundler does not discover a spurious service.exe.
use std::{io, process::ExitStatus};

/// Check stop while holding the same lock the shutdown path uses to take the
/// owner. A start completing after shutdown was requested cannot publish late.
pub(super) fn publish_if_running<T>(
    owner: &std::sync::Mutex<Option<T>>,
    stop: &std::sync::atomic::AtomicBool,
    value: T,
) -> Result<(), T> {
    let Ok(mut owner) = owner.lock() else {
        return Err(value);
    };
    if stop.load(std::sync::atomic::Ordering::SeqCst) || owner.is_some() {
        return Err(value);
    }
    *owner = Some(value);
    Ok(())
}
pub(super) struct ManagedCore {
    runtime: proxy_runtime::Runtime,
    session_id: Option<String>,
}
impl ManagedCore {
    #[cfg(test)]
    pub(super) fn new(runtime: proxy_runtime::Runtime) -> Self {
        Self {
            runtime,
            session_id: None,
        }
    }
    pub(super) fn with_session(
        runtime: proxy_runtime::Runtime,
        session_id: Option<String>,
    ) -> Self {
        Self {
            runtime,
            session_id,
        }
    }
    pub(super) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }
    pub(super) fn snapshot(&self) -> proxy_runtime::Snapshot {
        self.runtime.snapshot()
    }
    pub(super) fn id(&self) -> u32 {
        self.runtime.snapshot().pid
    }
    pub(super) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        use std::os::windows::process::ExitStatusExt;
        Ok(match self.runtime.snapshot().phase {
            proxy_runtime::Phase::Stopped => Some(ExitStatus::from_raw(0)),
            proxy_runtime::Phase::Failed => Some(ExitStatus::from_raw(1)),
            _ => None,
        })
    }
    pub(super) fn kill(&mut self) -> io::Result<()> {
        self.wait()
    }
    pub(super) fn wait(&mut self) -> io::Result<()> {
        block_on(self.runtime.stop())
            .map(|_| ())
            .map_err(io::Error::other)
    }
    pub(super) fn supports_strict_protection(&self) -> bool {
        let snapshot = self.runtime.snapshot();
        snapshot.tun_ready && snapshot.execution_policy == proxy_runtime::ExecutionPolicy::Protected
    }
    pub(super) fn engine(&self) -> &'static str {
        proxy_runtime::ENGINE_ID
    }
}
pub(super) fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("mihomo service control runtime")
        .block_on(future)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_publication_rechecks_stop_after_waiting_for_owner_lock() {
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        };
        let owner = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let held = owner.lock().unwrap();
        let worker_owner = owner.clone();
        let worker_stop = stop.clone();
        let worker =
            std::thread::spawn(move || publish_if_running(&worker_owner, &worker_stop, 42));
        stop.store(true, Ordering::SeqCst);
        drop(held);
        assert_eq!(worker.join().unwrap(), Err(42));
        assert!(owner.lock().unwrap().is_none());
    }

    #[test]
    fn publication_never_replaces_an_owned_runtime() {
        let owner = std::sync::Mutex::new(Some(1));
        assert_eq!(
            publish_if_running(&owner, &std::sync::atomic::AtomicBool::new(false), 2),
            Err(2)
        );
        assert_eq!(*owner.lock().unwrap(), Some(1));
    }
    #[test]
    fn service_owned_mihomo_is_child_process_and_waits_for_cleanup() {
        let directory = protection::test_support::FixtureDir::new();
        let yaml = "listeners:\n- name: local\n  type: mixed\n  listen: 127.0.0.1\n  port: 0\nexternal-controller: 127.0.0.1:0\nsecret: fixture-control-secret-32-bytes\ndns: {enable: false}\nrules: ['MATCH,DIRECT']\n";
        let runtime = block_on(proxy_runtime::Runtime::start(proxy_runtime::StartOptions {
            yaml: yaml.into(),
            work_dir: directory.path().into(),
        }))
        .unwrap();
        let listener = runtime.snapshot().listeners[0];
        let mut managed = ManagedCore::new(runtime);
        assert_ne!(managed.id(), std::process::id());
        assert_eq!(managed.engine(), proxy_runtime::ENGINE_ID);
        assert!(!managed.supports_strict_protection());
        assert!(managed.try_wait().unwrap().is_none());
        managed.kill().unwrap();
        managed.wait().unwrap();
        assert!(managed.try_wait().unwrap().is_some());
        assert!(std::net::TcpListener::bind(listener).is_ok());
    }
}
