//! One-time recovery of the former runtime's durable DNS snapshot.
use anyhow::{Result, bail};
use std::{
    fs,
    net::IpAddr,
    os::windows::{fs::MetadataExt, process::CommandExt},
    path::Path,
    process::Command,
};

fn restore_script(backup: &str) -> Result<String> {
    let (ipv4, ipv6) = backup
        .split_once("---IPV6---")
        .ok_or_else(|| anyhow::anyhow!("旧内核 DNS 恢复记录不完整"))?;
    let mut rows = Vec::new();
    for (section, v6) in [(ipv4, false), (ipv6, true)] {
        for line in section.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let (alias, servers) = line
                .split_once('|')
                .ok_or_else(|| anyhow::anyhow!("旧内核 DNS 恢复记录格式无效"))?;
            if alias.is_empty() || alias.len() > 256 || alias.chars().any(char::is_control) {
                bail!("DNS 网卡名称无效");
            }
            let ips: Vec<IpAddr> = servers
                .split(',')
                .map(|s| s.trim().parse())
                .collect::<std::result::Result<_, _>>()?;
            if ips.is_empty() || ips.len() > 16 || ips.iter().any(|ip| ip.is_ipv6() != v6) {
                bail!("DNS 服务器记录无效");
            }
            if ips.iter().all(IpAddr::is_loopback) {
                continue;
            }
            rows.push((alias.replace('\'', "''"), ips, v6));
        }
    }
    // A recovery journal proves only these recorded interfaces were touched.
    // Restore one address family only when it still holds the old loopback DNS;
    // a different current resolver belongs to a later user/client change.
    let mut script = String::from("$ErrorActionPreference='Stop';\n");
    for (alias, ips, v6) in rows {
        let ips = ips
            .iter()
            .map(|ip| format!("'{ip}'"))
            .collect::<Vec<_>>()
            .join(",");
        let family = if v6 { "IPv6" } else { "IPv4" };
        script.push_str(&format!(
            "$recorded = @(Get-DnsClientServerAddress -AddressFamily {family} | Where-Object {{ $_.InterfaceAlias -eq '{alias}' }}); foreach ($adapter in $recorded) {{ if (@($adapter.ServerAddresses).Count -gt 0 -and @($adapter.ServerAddresses | Where-Object {{ $_ -ne '127.0.0.1' -and $_ -ne '::1' }}).Count -eq 0) {{ Set-DnsClientServerAddress -InterfaceIndex $adapter.InterfaceIndex -ServerAddresses @({ips}) -ErrorAction Stop }} }};\n"
        ));
    }
    script.push_str("Clear-DnsClientCache -ErrorAction Stop;");
    Ok(script)
}
pub fn recover_dns(directory: &Path) -> Result<()> {
    let journal = directory.join("native-dns-recovery.txt");
    let metadata = match fs::symlink_metadata(&journal) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    if metadata.file_attributes() & 0x400 != 0
        || !metadata.is_file()
        || metadata.len() > 1024 * 1024
    {
        bail!("旧内核 DNS 恢复记录不安全");
    }
    let script = restore_script(&fs::read_to_string(&journal)?)?;
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let root = std::path::PathBuf::from(root);
    let output = Command::new(root.join("System32/WindowsPowerShell/v1.0/powershell.exe"))
        .env(
            "PSModulePath",
            root.join("System32/WindowsPowerShell/v1.0/Modules"),
        )
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x0800_0000)
        .output()?;
    if !output.status.success() {
        bail!("旧内核 DNS 恢复失败，记录已保留，请在设置中修复服务后重试");
    }
    fs::remove_file(journal)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_dns_journal_is_validated_before_any_system_change() {
        let script =
            restore_script("Ethernet's|8.8.8.8\n---IPV6---\nWi-Fi|2606:4700:4700::1111").unwrap();
        assert!(script.contains("Ethernet''s"));
        for invalid in [
            "Ethernet|8.8.8.8",
            "Ethernet|bad\n---IPV6---",
            "Ethernet|::1\n---IPV6---",
            "---IPV6---\nWi-Fi|1.1.1.1",
        ] {
            assert!(restore_script(invalid).is_err());
        }
        assert!(
            !restore_script("Ethernet|127.0.0.1\n---IPV6---\nWi-Fi|::1")
                .unwrap()
                .contains("-ServerAddresses @('127")
        );
    }
}
