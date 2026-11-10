//! 系统代理租约：保存实际代理，只恢复仍由本会话占有的设置。
use crate::config::AppConfig;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
pub use sysproxy::Sysproxy;
use tauri::{AppHandle, Manager};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub(super) struct Settings {
    enable: bool,
    server: String,
    bypass: String,
}
impl Settings {
    fn target(cfg: &AppConfig) -> Self {
        Self {
            enable: true,
            server: format!("127.0.0.1:{}", cfg.mixed_port),
            bypass: cfg.proxy_bypass.clone(),
        }
    }
    fn endpoint(&self, other: &Self) -> bool {
        self.enable && other.enable && self.server.eq_ignore_ascii_case(&other.server)
    }
    fn same(&self, other: &Self) -> bool {
        self.enable == other.enable
            && self.server.eq_ignore_ascii_case(&other.server)
            && self.bypass == other.bypass
    }
}
pub(super) trait Store {
    fn read(&self) -> Result<Settings>;
    fn write(&mut self, value: &Settings) -> Result<()>;
}
pub(super) struct Windows;
impl Store for Windows {
    fn read(&self) -> Result<Settings> {
        let (enable, server, bypass) = super::windows_proxy::read()?;
        Ok(Settings {
            enable,
            server,
            bypass,
        })
    }
    fn write(&mut self, value: &Settings) -> Result<()> {
        let previous = self.read()?;
        if let Err(error) = super::windows_proxy::write(value.enable, &value.server, &value.bypass)
        {
            let restoration =
                super::windows_proxy::write(previous.enable, &previous.server, &previous.bypass);
            return Err(match restoration {
                Ok(()) => error,
                Err(restore) => {
                    anyhow::anyhow!("系统代理写入失败：{error:#}；原值恢复失败：{restore:#}")
                }
            });
        }
        Ok(())
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Record {
    previous: Option<Settings>,
    owned: Option<Settings>,
    /// 写入前记录目标，崩溃时同时保留旧端口和待切换端口的恢复权。
    #[serde(default)]
    pending: Option<Settings>,
}
impl Record {
    fn owns(&self, settings: &Settings) -> bool {
        self.owned
            .iter()
            .chain(self.pending.iter())
            .any(|owned| owned.same(settings))
    }
}
pub(super) struct Lease<S = Windows> {
    store: S,
    record: Record,
    path: Option<PathBuf>,
}
impl Default for Lease {
    fn default() -> Self {
        Self {
            store: Windows,
            record: Record::default(),
            path: None,
        }
    }
}
impl<S: Store> Lease<S> {
    fn open(&mut self, directory: &Path) -> Result<()> {
        if self.path.is_none() {
            let path = directory.join("proxy-lease.json");
            crate::platform::secure_storage::safe(&path)?;
            if path.is_file() {
                let bytes = std::fs::read(&path)?;
                if bytes.len() > 64 * 1024 {
                    bail!("系统代理恢复记录过大，未改动系统设置");
                }
                self.record = serde_json::from_slice(&bytes)?;
            }
            self.path = Some(path);
        }
        Ok(())
    }
    fn persist_record(&self, record: &Record) -> Result<()> {
        if let Some(path) = &self.path {
            crate::config::atomic_write(path, &serde_json::to_vec(record)?)?;
        }
        Ok(())
    }
    fn acquire(&mut self, desired: Settings) -> Result<()> {
        let current = self.store.read()?;
        let mut next = self.record.clone();
        if !self.record.owns(&current) || next.previous.is_none() {
            // 他软件重新设置代理后再接管，退出应恢复那个新值。
            next.previous = Some(
                if !self.record.owns(&current) && current.endpoint(&desired) {
                    Settings {
                        enable: false,
                        ..current.clone()
                    }
                } else {
                    current.clone()
                },
            );
        }
        next.pending = Some(desired.clone());
        self.persist_record(&next)?;
        self.record = next;
        self.store.write(&desired)?;
        if !self.store.read()?.same(&desired) {
            bail!("系统代理写入后核验失败");
        }
        let committed = Record {
            previous: self.record.previous.clone(),
            owned: Some(desired),
            pending: None,
        };
        self.persist_record(&committed)?;
        self.record = committed;
        Ok(())
    }
    fn release(&mut self) -> Result<()> {
        let current = self.store.read()?;
        if self.record.owns(&current) {
            let previous = self.record.previous.clone().unwrap_or_default();
            self.store.write(&previous)?;
            if !self.store.read()?.same(&previous) {
                bail!("原系统代理未恢复，内核保持运行");
            }
        }
        // 他软件已改写设置时，只结束租约，不覆盖对方的新值。
        let released = Record::default();
        self.persist_record(&released)?;
        self.record = released;
        Ok(())
    }
}
pub fn get_sysproxy() -> Result<Sysproxy> {
    let value = Windows.read()?;
    let address = value.server.parse::<std::net::SocketAddr>().ok();
    Ok(Sysproxy {
        enable: value.enable,
        host: address.map(|v| v.ip().to_string()).unwrap_or_default(),
        port: address.map(|v| v.port()).unwrap_or(0),
        bypass: value.bypass,
    })
}
pub fn is_ours(cfg: &AppConfig) -> bool {
    Windows
        .read()
        .is_ok_and(|p| p.endpoint(&Settings::target(cfg)))
}
/// 清理旧版本遗留的空端口；不动仍有程序监听的其他代理。
pub fn recover_stale(app: &AppHandle, cfg: &AppConfig) -> Result<()> {
    let state = app.state::<super::core::CoreState>();
    let directory = state.data_dir.lock().clone();
    let mut lease = state.proxy.lock();
    lease.open(&directory)?;
    let current = lease.store.read()?;
    let Ok(address) = current.server.parse::<std::net::SocketAddr>() else {
        return Ok(());
    };
    if !current.enable
        || !address.ip().is_loopback()
        || proxy_runtime::tcp_listener_owner(address)?.is_some()
    {
        return Ok(());
    }
    if lease.record.owns(&current) {
        return lease.release();
    }
    // 只有当前配置指定的旧版本空端口才能在缺少租约时恢复。
    if current.endpoint(&Settings::target(cfg))
        && lease.record.owned.is_none()
        && lease.record.pending.is_none()
    {
        let mut previous = current.clone();
        previous.enable = false;
        let record = Record {
            previous: Some(previous),
            owned: Some(current),
            pending: None,
        };
        lease.persist_record(&record)?;
        lease.record = record;
        lease.release()?;
    }
    Ok(())
}
pub fn release_saved(directory: &Path) -> Result<()> {
    let mut lease = Lease::default();
    lease.open(directory)?;
    lease.release()
}
pub fn apply(app: &AppHandle, cfg: &AppConfig) -> Result<()> {
    let state = app.state::<super::core::CoreState>();
    let directory = state.data_dir.lock().clone();
    let mut lease = state.proxy.lock();
    lease.open(&directory)?;
    if cfg.enable_system_proxy {
        lease.acquire(Settings::target(cfg))
    } else {
        lease.release()
    }
}
pub fn release(app: &AppHandle) -> Result<()> {
    let state = app.state::<super::core::CoreState>();
    let directory = state.data_dir.lock().clone();
    let mut lease = state.proxy.lock();
    lease.open(&directory)?;
    lease.release()
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Memory {
        current: Settings,
        fail: bool,
    }
    impl Store for Memory {
        fn read(&self) -> Result<Settings> {
            Ok(self.current.clone())
        }
        fn write(&mut self, value: &Settings) -> Result<()> {
            if self.fail {
                bail!("write rejected");
            }
            self.current = value.clone();
            Ok(())
        }
    }
    fn proxy(port: u16) -> Settings {
        Settings {
            enable: true,
            server: format!("127.0.0.1:{port}"),
            bypass: "localhost".into(),
        }
    }
    fn fixture() -> Lease<Memory> {
        Lease {
            store: Memory {
                current: proxy(7890),
                fail: false,
            },
            record: Record::default(),
            path: None,
        }
    }
    #[test]
    fn takeover_switch_and_release_restores_original_proxy() {
        let mut p = fixture();
        p.acquire(proxy(7897)).unwrap();
        p.acquire(proxy(7898)).unwrap();
        p.release().unwrap();
        assert_eq!(p.store.current, proxy(7890));
        p.release().unwrap();
        assert_eq!(p.store.current, proxy(7890));
    }
    #[test]
    fn exit_never_overwrites_another_clients_proxy() {
        let mut p = fixture();
        p.acquire(proxy(7897)).unwrap();
        p.store.current = proxy(12345);
        p.release().unwrap();
        assert_eq!(p.store.current, proxy(12345));
    }
    #[test]
    fn restoration_failure_retains_ownership_for_retry() {
        let mut p = fixture();
        p.acquire(proxy(7897)).unwrap();
        p.store.fail = true;
        assert!(p.release().is_err());
        assert!(p.record.owned.is_some());
        p.store.fail = false;
        p.release().unwrap();
        assert_eq!(p.store.current, proxy(7890));
    }
    #[test]
    fn recovery_survives_process_restart() {
        let root = crate::test_support::FixtureDir::new();
        let mut p = fixture();
        p.open(root.path()).unwrap();
        p.acquire(proxy(7897)).unwrap();
        let mut cold = fixture();
        cold.store.current = proxy(7897);
        cold.open(root.path()).unwrap();
        cold.release().unwrap();
        assert_eq!(cold.store.current, proxy(7890));
    }
    #[test]
    fn failed_port_change_retains_old_proxy_ownership() {
        let mut p = fixture();
        p.acquire(proxy(7897)).unwrap();
        p.store.fail = true;
        assert!(p.acquire(proxy(7898)).is_err());
        assert!(p.record.owns(&proxy(7897)));
        p.store.fail = false;
        p.release().unwrap();
        assert_eq!(p.store.current, proxy(7890));
    }
    #[test]
    fn renewed_takeover_restores_the_other_clients_latest_proxy() {
        let mut p = fixture();
        p.acquire(proxy(7897)).unwrap();
        p.store.current = proxy(12345);
        p.acquire(proxy(7897)).unwrap();
        p.release().unwrap();
        assert_eq!(p.store.current, proxy(12345));
    }
}
