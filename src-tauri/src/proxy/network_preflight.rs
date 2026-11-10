use anyhow::{Context, Result, bail};
use std::net::{IpAddr, Ipv4Addr};

const TUN_IPV4: Ipv4Addr = Ipv4Addr::new(198, 18, 0, 1);

fn conflicting_interface<'a>(
    interfaces: impl Iterator<Item = (&'a str, IpAddr)>,
) -> Option<&'a str> {
    interfaces
        .filter(|(name, address)| {
            *address == IpAddr::V4(TUN_IPV4) && !name.eq_ignore_ascii_case(crate::privacy::TUN_NAME)
        })
        .map(|(name, _)| name)
        .next()
}

/// 只读预检；不关闭其他客户端，不删除网卡，不改动路由。
pub fn ensure_tun_available() -> Result<()> {
    // FlClash 的地址可为 28.0.0.1、172.19.0.1 等，不能只判断固定 IP。
    if let Some(name) = active_foreign_tun()? {
        bail!(
            "无法启用 TUN：网卡「{name}」仍在运行。请先关闭该软件的 TUN，再开启云阔 TUN；原连接保持不变。"
        );
    }
    let interfaces = if_addrs::get_if_addrs().context("无法核对 TUN 地址占用，请稍后重试")?;
    if let Some(name) = conflicting_interface(
        interfaces
            .iter()
            .map(|item| (item.name.as_str(), item.ip())),
    ) {
        bail!(
            "无法启用 TUN：网卡「{name}」正在使用地址 {TUN_IPV4}。请先在该代理软件中关闭 TUN，再回到本软件启用；本次未修改原配置。"
        );
    }
    Ok(())
}

fn active_foreign_tun() -> Result<Option<String>> {
    use windows::Win32::NetworkManagement::{
        IpHelper::{FreeMibTable, GetIfTable2},
        Ndis::IfOperStatusUp,
    };
    let mut table = std::ptr::null_mut();
    let result = unsafe { GetIfTable2(&mut table) };
    if result.0 != 0 {
        bail!("无法读取虚拟网卡状态（{}）", result.0);
    }
    if table.is_null() {
        bail!("虚拟网卡状态为空");
    }
    let found = unsafe {
        let rows =
            std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
        let found = rows
            .iter()
            .filter(|row| row.OperStatus == IfOperStatusUp)
            .find_map(|row| {
                let decode = |value: &[u16]| {
                    String::from_utf16_lossy(
                        &value[..value.iter().position(|v| *v == 0).unwrap_or(value.len())],
                    )
                };
                let alias = decode(&row.Alias);
                let description = decode(&row.Description);
                (is_proxy_tunnel(&alias, &description)
                    && !alias.eq_ignore_ascii_case(crate::privacy::TUN_NAME))
                .then_some(alias)
            });
        FreeMibTable(table.cast());
        found
    };
    Ok(found)
}

/// Hyper-V、VMware 等虚拟以太网不等于代理隧道，不能一概阻止 TUN。
fn is_proxy_tunnel(alias: &str, description: &str) -> bool {
    let alias = alias.to_ascii_lowercase();
    let description = description.to_ascii_lowercase();
    [
        "wintun",
        "wireguard",
        "meta tunnel",
        "sing-box",
        "singbox",
        "clash",
        "mihomo",
        "hiddify",
        "nekoray",
        "v2ray",
    ]
    .iter()
    .any(|name| alias.contains(name) || description.contains(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreign_tun_is_rejected_before_core_or_proxy_changes() {
        let foreign = [("FlClash", IpAddr::V4(TUN_IPV4))];
        assert_eq!(
            conflicting_interface(foreign.iter().map(|(name, address)| (*name, *address))),
            Some("FlClash")
        );
    }

    #[test]
    fn own_adapter_and_unrelated_addresses_are_allowed() {
        let interfaces = [
            ("云阔TUN", IpAddr::V4(TUN_IPV4)),
            ("WLAN", "192.168.1.2".parse().unwrap()),
        ];
        assert!(
            conflicting_interface(interfaces.iter().map(|(name, address)| (*name, *address)))
                .is_none()
        );
    }
    #[test]
    fn virtual_ethernet_is_not_a_proxy_tunnel() {
        assert!(!is_proxy_tunnel(
            "vEthernet (Default Switch)",
            "Hyper-V Virtual Ethernet Adapter"
        ));
        assert!(!is_proxy_tunnel(
            "VMware Network Adapter VMnet8",
            "VMware Virtual Ethernet Adapter"
        ));
        assert!(is_proxy_tunnel("FlClash", "Meta Tunnel"));
        assert!(is_proxy_tunnel("VPN", "WireGuard Tunnel"));
    }
}
