//! Observe the kernel process that actually owns each Windows TCP listener.
use anyhow::{Result, bail};
use std::{
    mem::size_of,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
};
use windows::Win32::{
    Foundation::CloseHandle,
    NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID,
        TCP_TABLE_OWNER_PID_LISTENER,
    },
    Networking::WinSock::{AF_INET, AF_INET6},
    System::Threading::{
        OpenProcess, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
        QueryFullProcessImageNameW, TerminateProcess,
    },
};
use windows::core::PWSTR;

/// 某进程的可执行文件；进程已退出或权限不足时返回 None。
pub fn process_image(pid: u32) -> Option<PathBuf> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = vec![0u16; 1024];
        let mut size = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_FORMAT(0),
            PWSTR(buffer.as_mut_ptr()),
            &mut size,
        );
        let _ = CloseHandle(process);
        if result.is_err() || size == 0 {
            return None;
        }
        Some(PathBuf::from(String::from_utf16_lossy(
            &buffer[..size as usize],
        )))
    }
}

/// 结束指定进程；用于回收上一轮遗留的自家内核。
pub fn terminate(pid: u32) -> bool {
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_TERMINATE, false, pid) else {
            return false;
        };
        let result = TerminateProcess(process, 1);
        let _ = CloseHandle(process);
        result.is_ok()
    }
}

/// 端口是否被本软件的内核占用。按可执行文件名识别，
/// 这样应用托管和服务托管（ProgramData 下的副本）都算自家进程。
pub fn listener_owned_by_engine(address: SocketAddr, engine: &Path) -> bool {
    let Some(engine_name) = engine.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Ok(Some(pid)) = tcp_listener_owner(address) else {
        return false;
    };
    pid != 0
        && process_image(pid).is_some_and(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(engine_name))
        })
}

fn rows<T: Copy>(family: u32) -> Result<Vec<T>> {
    let mut size = 0u32;
    unsafe {
        GetExtendedTcpTable(
            None,
            &mut size,
            false,
            family,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };
    for _ in 0..3 {
        if size < 4 || size > 32 * 1024 * 1024 {
            bail!("TCP 监听表大小无效");
        }
        let mut storage = vec![0u32; (size as usize).div_ceil(4)];
        let result = unsafe {
            GetExtendedTcpTable(
                Some(storage.as_mut_ptr().cast()),
                &mut size,
                false,
                family,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        if result == 122 {
            continue;
        }
        if result != 0 {
            bail!("读取 TCP 监听进程失败（{result}）");
        }
        let count = storage[0] as usize;
        if count > (size as usize - 4) / size_of::<T>() {
            bail!("TCP 监听表格式无效");
        }
        return Ok((0..count)
            .map(|index| unsafe {
                storage
                    .as_ptr()
                    .cast::<u8>()
                    .add(4 + index * size_of::<T>())
                    .cast::<T>()
                    .read_unaligned()
            })
            .collect());
    }
    bail!("TCP 监听表持续变化，请重试")
}
pub fn tcp_listener_owner(address: SocketAddr) -> Result<Option<u32>> {
    Ok(tcp_listener_owners(&[address])?[0])
}

/// Read each listener table at most once for a complete ownership check.
/// Results keep the input order; no process or table snapshot is cached.
pub fn tcp_listener_owners(addresses: &[SocketAddr]) -> Result<Vec<Option<u32>>> {
    if addresses.is_empty() {
        return Ok(Vec::new());
    }
    let ipv4 = rows::<MIB_TCPROW_OWNER_PID>(AF_INET.0 as u32)?;
    let mut owners: Vec<_> = addresses
        .iter()
        .map(|address| {
            ipv4.iter().find_map(|row| {
                let ip = Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes());
                (u16::from_be(row.dwLocalPort as u16) == address.port()
                    && (ip.is_unspecified() || address.ip() == std::net::IpAddr::V4(ip)))
                .then_some(row.dwOwningPid)
            })
        })
        .collect();
    if owners.iter().any(Option::is_none) {
        let ipv6 = rows::<MIB_TCP6ROW_OWNER_PID>(AF_INET6.0 as u32)?;
        for (address, owner) in addresses.iter().zip(&mut owners) {
            if owner.is_none() {
                *owner = ipv6.iter().find_map(|row| {
                    let ip = std::net::Ipv6Addr::from(row.ucLocalAddr);
                    (u16::from_be(row.dwLocalPort as u16) == address.port()
                        && (ip.is_unspecified() || address.ip() == std::net::IpAddr::V6(ip)))
                    .then_some(row.dwOwningPid)
                });
            }
        }
    }
    Ok(owners)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finds_actual_loopback_listener_owner() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        assert_eq!(
            tcp_listener_owner(address).unwrap(),
            Some(std::process::id())
        );
        drop(listener);
        assert_eq!(tcp_listener_owner(address).unwrap(), None);
    }
}
