//! WFP 出站白名单。非动态会话 + 持久过滤器：服务/GUI/内核退出不解除保护。
//! 仅本模块自己的 provider/sublayer 会被更新或删除，不修改 Windows 防火墙默认策略。
use crate::privacy::Endpoint;
use anyhow::{Result, bail};
use std::path::Path;
use windows::Win32::Foundation::{
    FWP_E_ALREADY_EXISTS, FWP_E_PROVIDER_NOT_FOUND, FWP_E_SUBLAYER_NOT_FOUND, HANDLE,
};
use windows::Win32::NetworkManagement::WindowsFilteringPlatform::*;
use windows::core::{GUID, PCWSTR, PWSTR};

const PROVIDER: GUID = GUID::from_u128(0x87dad16e_8b07_4f95_a6b7_0f9b11ec584d);
const SUBLAYER: GUID = GUID::from_u128(0x7c6c731e_ea22_43c3_b504_3b9fae9bc784);

fn check(code: u32, operation: &str) -> Result<()> {
    if code != 0 {
        bail!("{operation} 失败（WFP {code:#x}）");
    }
    Ok(())
}

pub struct NetworkLock {
    handle: HANDLE,
    provider: GUID,
    sublayer: GUID,
}

impl Drop for NetworkLock {
    fn drop(&mut self) {
        unsafe {
            FwpmEngineClose0(self.handle);
        }
    }
}

impl NetworkLock {
    pub fn open() -> Result<Self> {
        Self::with_keys(PROVIDER, SUBLAYER)
    }

    fn with_keys(provider: GUID, sublayer: GUID) -> Result<Self> {
        let mut handle = HANDLE::default();
        // RPC_C_AUTHN_WINNT=10。刻意不用 FWPM_SESSION_FLAG_DYNAMIC。
        check(
            unsafe { FwpmEngineOpen0(PCWSTR::null(), 10, None, None, &mut handle) },
            "连接网络过滤引擎",
        )?;
        Ok(Self {
            handle,
            provider,
            sublayer,
        })
    }

    fn ensure_owner(&self) -> Result<()> {
        let mut label: Vec<u16> = "Claude One Click 网络保护\0".encode_utf16().collect();
        let display = FWPM_DISPLAY_DATA0 {
            name: PWSTR(label.as_mut_ptr()),
            ..Default::default()
        };
        let provider = FWPM_PROVIDER0 {
            providerKey: self.provider,
            displayData: display,
            flags: FWPM_PROVIDER_FLAG_PERSISTENT,
            ..Default::default()
        };
        let code = unsafe { FwpmProviderAdd0(self.handle, &provider, None) };
        if code != FWP_E_ALREADY_EXISTS.0 as u32 {
            check(code, "创建网络保护提供程序")?;
        }
        let mut provider_key = self.provider;
        let sublayer = FWPM_SUBLAYER0 {
            subLayerKey: self.sublayer,
            providerKey: &mut provider_key,
            displayData: display,
            flags: FWPM_SUBLAYER_FLAG_PERSISTENT,
            weight: 0x8000,
            ..Default::default()
        };
        let code = unsafe { FwpmSubLayerAdd0(self.handle, &sublayer, None) };
        if code != FWP_E_ALREADY_EXISTS.0 as u32 {
            check(code, "创建网络保护子层")?;
        }
        Ok(())
    }

    fn owned_ids(&self) -> Result<Vec<u64>> {
        let mut enumeration = HANDLE::default();
        // None 枚举全部层；带有空 layerKey 的模板会被 WFP 拒绝。
        check(
            unsafe { FwpmFilterCreateEnumHandle0(self.handle, None, &mut enumeration) },
            "读取保护规则",
        )?;
        let result = (|| {
            let mut ids = Vec::new();
            loop {
                let mut entries = std::ptr::null_mut();
                let mut count = 0;
                check(
                    unsafe {
                        FwpmFilterEnum0(self.handle, enumeration, 256, &mut entries, &mut count)
                    },
                    "枚举保护规则",
                )?;
                if count > 0 {
                    for entry in unsafe { std::slice::from_raw_parts(entries, count as usize) } {
                        let filter = unsafe { &**entry };
                        if filter.subLayerKey == self.sublayer {
                            ids.push(filter.filterId);
                        }
                    }
                }
                if !entries.is_null() {
                    unsafe {
                        FwpmFreeMemory0(&mut entries as *mut _ as *mut *mut std::ffi::c_void);
                    }
                }
                if count < 256 {
                    break;
                }
            }
            Ok(ids)
        })();
        unsafe {
            FwpmFilterDestroyEnumHandle0(self.handle, enumeration);
        }
        result
    }

    pub fn installed(&self) -> Result<bool> {
        Ok(!self.owned_ids()?.is_empty())
    }

    pub fn rule_count(&self) -> Result<usize> {
        Ok(self.owned_ids()?.len())
    }

    fn transaction(&self, work: impl FnOnce() -> Result<()>) -> Result<()> {
        check(
            unsafe { FwpmTransactionBegin0(self.handle, 0) },
            "开始保护规则事务",
        )?;
        let result = work().and_then(|_| {
            check(
                unsafe { FwpmTransactionCommit0(self.handle) },
                "提交保护规则",
            )
        });
        if result.is_err() {
            unsafe {
                FwpmTransactionAbort0(self.handle);
            }
        }
        result
    }

    fn delete_filters(&self) -> Result<()> {
        for id in self.owned_ids()? {
            check(
                unsafe { FwpmFilterDeleteById0(self.handle, id) },
                "删除本软件保护规则",
            )?;
        }
        Ok(())
    }

    pub fn release(&self) -> Result<()> {
        self.transaction(|| self.delete_filters())
    }

    /// 卸载用：规则、子层和提供程序在同一事务中删除，不留下空的持久化 WFP 对象。
    pub fn remove(&self) -> Result<()> {
        self.transaction(|| {
            self.delete_filters()?;
            let code = unsafe { FwpmSubLayerDeleteByKey0(self.handle, &self.sublayer) };
            if code != FWP_E_SUBLAYER_NOT_FOUND.0 as u32 {
                check(code, "删除网络保护子层")?;
            }
            let code = unsafe { FwpmProviderDeleteByKey0(self.handle, &self.provider) };
            if code != FWP_E_PROVIDER_NOT_FOUND.0 as u32 {
                check(code, "删除网络保护提供程序")?;
            }
            Ok(())
        })
    }

    /// 更新在一个事务中完成；失败时旧规则保留。scope 仅用于限制集成测试到测试程序。
    pub fn apply(&self, kernel: &Path, endpoints: &[Endpoint], tunnel: Option<u64>) -> Result<()> {
        self.apply_scoped(kernel, endpoints, tunnel, None)
    }

    fn apply_scoped(
        &self,
        kernel: &Path,
        endpoints: &[Endpoint],
        tunnel: Option<u64>,
        scope: Option<&Path>,
    ) -> Result<()> {
        self.ensure_owner()?;
        let app = AppId::new(kernel)?;
        let scoped_app = scope.map(AppId::new).transpose()?;
        let resolver = if scope.is_none() {
            Some(AppId::new(&std::env::current_exe()?)?)
        } else {
            None
        };
        self.transaction(|| {
            self.delete_filters()?;
            for layer in [
                FWPM_LAYER_ALE_AUTH_CONNECT_V4,
                FWPM_LAYER_ALE_AUTH_CONNECT_V6,
            ] {
                let mut base = scoped_app
                    .as_ref()
                    .map(|a| vec![a.condition()])
                    .unwrap_or_default();
                self.add(layer, FWP_ACTION_BLOCK, 1, &mut base, true)?;
                let mut loopback = base.clone();
                loopback.push(condition(
                    FWPM_CONDITION_FLAGS,
                    FWP_MATCH_FLAGS_ALL_SET,
                    FWP_UINT32,
                    FWP_CONDITION_VALUE0_0 {
                        uint32: FWP_CONDITION_FLAG_IS_LOOPBACK,
                    },
                ));
                self.add(layer, FWP_ACTION_PERMIT, 15, &mut loopback, true)?;
            }
            if let Some(mut luid) = tunnel {
                let mut conditions = scoped_app
                    .as_ref()
                    .map(|a| vec![a.condition()])
                    .unwrap_or_default();
                conditions.push(condition(
                    FWPM_CONDITION_IP_LOCAL_INTERFACE,
                    FWP_MATCH_EQUAL,
                    FWP_UINT64,
                    FWP_CONDITION_VALUE0_0 { uint64: &mut luid },
                ));
                // TUN 放行不跨系统重启持久化，避免陈旧网卡 LUID 被复用。
                self.add(
                    FWPM_LAYER_ALE_AUTH_CONNECT_V4,
                    FWP_ACTION_PERMIT,
                    14,
                    &mut conditions,
                    false,
                )?;
            }
            for endpoint in endpoints {
                for protocol in [6u8, 17u8] {
                    let mut conditions = vec![
                        app.condition(),
                        condition(
                            FWPM_CONDITION_IP_REMOTE_ADDRESS,
                            FWP_MATCH_EQUAL,
                            FWP_UINT32,
                            FWP_CONDITION_VALUE0_0 {
                                uint32: u32::from(endpoint.ip),
                            },
                        ),
                        condition(
                            FWPM_CONDITION_IP_REMOTE_PORT,
                            FWP_MATCH_EQUAL,
                            FWP_UINT16,
                            FWP_CONDITION_VALUE0_0 {
                                uint16: endpoint.port,
                            },
                        ),
                        condition(
                            FWPM_CONDITION_IP_PROTOCOL,
                            FWP_MATCH_EQUAL,
                            FWP_UINT8,
                            FWP_CONDITION_VALUE0_0 { uint8: protocol },
                        ),
                    ];
                    self.add(
                        FWPM_LAYER_ALE_AUTH_CONNECT_V4,
                        FWP_ACTION_PERMIT,
                        14,
                        &mut conditions,
                        true,
                    )?;
                }
            }
            if let Some(resolver) = &resolver {
                // 服务只可向两个固定 IP 的 HTTPS DNS 发起引导解析；不放行系统 DNS。
                for ip in crate::privacy::BOOTSTRAP_IPS {
                    let mut conditions = vec![
                        resolver.condition(),
                        condition(
                            FWPM_CONDITION_IP_REMOTE_ADDRESS,
                            FWP_MATCH_EQUAL,
                            FWP_UINT32,
                            FWP_CONDITION_VALUE0_0 {
                                uint32: u32::from(ip.parse::<std::net::Ipv4Addr>()?),
                            },
                        ),
                        condition(
                            FWPM_CONDITION_IP_REMOTE_PORT,
                            FWP_MATCH_EQUAL,
                            FWP_UINT16,
                            FWP_CONDITION_VALUE0_0 { uint16: 443 },
                        ),
                        condition(
                            FWPM_CONDITION_IP_PROTOCOL,
                            FWP_MATCH_EQUAL,
                            FWP_UINT8,
                            FWP_CONDITION_VALUE0_0 { uint8: 6 },
                        ),
                    ];
                    self.add(
                        FWPM_LAYER_ALE_AUTH_CONNECT_V4,
                        FWP_ACTION_PERMIT,
                        14,
                        &mut conditions,
                        true,
                    )?;
                }
            }
            Ok(())
        })
    }

    fn add(
        &self,
        layer: GUID,
        action: FWP_ACTION_TYPE,
        weight: u8,
        conditions: &mut [FWPM_FILTER_CONDITION0],
        persistent: bool,
    ) -> Result<()> {
        let mut provider = self.provider;
        let mut name: Vec<u16> = "Claude OC 受控出口\0".encode_utf16().collect();
        let filter = FWPM_FILTER0 {
            filterKey: GUID::from_u128(uuid::Uuid::new_v4().as_u128()),
            displayData: FWPM_DISPLAY_DATA0 {
                name: PWSTR(name.as_mut_ptr()),
                ..Default::default()
            },
            flags: if persistent {
                FWPM_FILTER_FLAG_PERSISTENT
            } else {
                FWPM_FILTER_FLAGS(0)
            },
            providerKey: &mut provider,
            layerKey: layer,
            subLayerKey: self.sublayer,
            weight: FWP_VALUE0 {
                r#type: FWP_UINT8,
                Anonymous: FWP_VALUE0_0 { uint8: weight },
            },
            numFilterConditions: conditions.len() as u32,
            filterCondition: conditions.as_mut_ptr(),
            action: FWPM_ACTION0 {
                r#type: action,
                ..Default::default()
            },
            ..Default::default()
        };
        check(
            unsafe { FwpmFilterAdd0(self.handle, &filter, None, None) },
            "安装网络保护规则",
        )
    }
}

fn condition(
    key: GUID,
    matching: FWP_MATCH_TYPE,
    kind: FWP_DATA_TYPE,
    value: FWP_CONDITION_VALUE0_0,
) -> FWPM_FILTER_CONDITION0 {
    FWPM_FILTER_CONDITION0 {
        fieldKey: key,
        matchType: matching,
        conditionValue: FWP_CONDITION_VALUE0 {
            r#type: kind,
            Anonymous: value,
        },
    }
}

struct AppId(*mut FWP_BYTE_BLOB);
impl AppId {
    fn new(path: &Path) -> Result<Self> {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut blob = std::ptr::null_mut();
        check(
            unsafe { FwpmGetAppIdFromFileName0(PCWSTR(wide.as_ptr()), &mut blob) },
            "确认受保护程序",
        )?;
        Ok(Self(blob))
    }
    fn condition(&self) -> FWPM_FILTER_CONDITION0 {
        condition(
            FWPM_CONDITION_ALE_APP_ID,
            FWP_MATCH_EQUAL,
            FWP_BYTE_BLOB_TYPE,
            FWP_CONDITION_VALUE0_0 { byteBlob: self.0 },
        )
    }
}
impl Drop for AppId {
    fn drop(&mut self) {
        unsafe {
            FwpmFreeMemory0(&mut self.0 as *mut _ as *mut *mut std::ffi::c_void);
        }
    }
}

pub fn tunnel_luid() -> Option<u64> {
    use windows::Win32::NetworkManagement::IpHelper::{
        ConvertInterfaceAliasToLuid, GetIfEntry2, MIB_IF_ROW2,
    };
    use windows::Win32::NetworkManagement::Ndis::{IfOperStatusUp, NET_LUID_LH};
    let alias: Vec<u16> = crate::privacy::TUN_NAME
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut luid = NET_LUID_LH::default();
    if unsafe { ConvertInterfaceAliasToLuid(PCWSTR(alias.as_ptr()), &mut luid) }.0 != 0 {
        return None;
    }
    let mut row = MIB_IF_ROW2 {
        InterfaceLuid: luid,
        ..Default::default()
    };
    if unsafe { GetIfEntry2(&mut row) }.0 != 0 || row.OperStatus != IfOperStatusUp {
        return None;
    }
    Some(unsafe { luid.Value })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "需管理员；规则仅匹配本测试程序，不影响其他程序"]
    fn scoped_filters_block_direct_and_survive_engine_close() {
        use std::net::{SocketAddr, TcpListener, TcpStream};
        use std::time::Duration;
        fn dns_roundtrip() -> bool {
            let socket = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
            socket
                .set_read_timeout(Some(Duration::from_millis(700)))
                .unwrap();
            // example.com A 查询；send_to 成功只代表进入本机缓冲，不能证明 WFP 已放行。
            let query = [
                0x43, 0x4f, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 7, b'e', b'x', b'a', b'm', b'p', b'l',
                b'e', 3, b'c', b'o', b'm', 0, 0, 1, 0, 1,
            ];
            if socket.send_to(&query, "1.1.1.1:53").is_err() {
                return false;
            }
            let mut response = [0u8; 512];
            socket
                .recv_from(&mut response)
                .is_ok_and(|(len, _)| len >= 12 && response[0..2] == query[0..2])
        }
        let exe = std::env::current_exe().unwrap();
        let public: SocketAddr = "1.1.1.1:443".parse().unwrap();
        assert!(
            TcpStream::connect_timeout(&public, Duration::from_secs(3)).is_ok(),
            "测试前需确认 IPv4 外网可达，避免把网络故障误当作成功拦截"
        );
        let udp_reachable = dns_roundtrip();
        let provider = GUID::from_u128(uuid::Uuid::new_v4().as_u128());
        let sublayer = GUID::from_u128(uuid::Uuid::new_v4().as_u128());
        let lock = NetworkLock::with_keys(provider, sublayer).unwrap();
        lock.apply_scoped(&exe, &[], None, Some(&exe)).unwrap();
        assert_eq!(lock.rule_count().unwrap(), 4);
        drop(lock);
        let lock = NetworkLock::with_keys(provider, sublayer).unwrap();
        let result = std::panic::catch_unwind(|| {
            assert_eq!(lock.rule_count().unwrap(), 4);
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            assert!(
                TcpStream::connect_timeout(&listener.local_addr().unwrap(), Duration::from_secs(2))
                    .is_ok()
            );
            for destination in ["1.1.1.1:443", "[2606:4700:4700::1111]:443"] {
                let address: SocketAddr = destination.parse().unwrap();
                assert!(TcpStream::connect_timeout(&address, Duration::from_secs(2)).is_err());
            }
            if udp_reachable {
                assert!(!dns_roundtrip(), "普通 UDP DNS 不能完成往返");
            } else {
                eprintln!("当前网络 UDP DNS 基线不可达，本次仅核验 TCP 与 WFP IPv4/IPv6 规则");
            }
            let allowed = Endpoint {
                ip: "1.1.1.1".parse().unwrap(),
                port: 443,
            };
            lock.apply_scoped(&exe, &[allowed], None, Some(&exe))
                .unwrap();
            assert!(
                TcpStream::connect_timeout(&public, Duration::from_secs(3)).is_ok(),
                "白名单中的程序、IP、端口组合应放行"
            );
            assert!(
                TcpStream::connect_timeout(&"1.1.1.1:80".parse().unwrap(), Duration::from_secs(2))
                    .is_err(),
                "同 IP 的其他端口不能绕过白名单"
            );
            assert!(
                TcpStream::connect_timeout(&"9.9.9.9:443".parse().unwrap(), Duration::from_secs(2))
                    .is_err(),
                "其他 IP 的相同端口不能绕过白名单"
            );
            let failed = lock.transaction(|| {
                lock.delete_filters()?;
                bail!("模拟更新失败")
            });
            assert!(failed.is_err());
            assert_eq!(lock.rule_count().unwrap(), 6, "事务失败时旧规则必须保留");
            assert!(
                TcpStream::connect_timeout(&"9.9.9.9:443".parse().unwrap(), Duration::from_secs(2))
                    .is_err()
            );
            lock.apply_scoped(&exe, &[], None, Some(&exe)).unwrap();
            assert!(TcpStream::connect_timeout(&public, Duration::from_secs(2)).is_err());
        });
        lock.remove().unwrap();
        assert!(!lock.installed().unwrap());
        lock.remove().expect("重复移除应视为已完成");
        assert!(
            TcpStream::connect_timeout(&public, Duration::from_secs(3)).is_ok(),
            "解除后恢复外网"
        );
        if let Err(error) = result {
            std::panic::resume_unwind(error);
        }
    }
}
