use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[allow(dead_code, reason = "共享协议模块：受保护目录仅供安全服务使用")]
pub const PROGRAM_DIR: &str = r"C:\ProgramData\Yunkuo";
pub const SERVICE_VERSION: &str = "4.3";
pub const MAX_REQUEST_BYTES: usize = 10 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRequest {
    pub config: String,
    pub execution_policy: RuntimePolicy,
}

/// Required protocol field: old clients cannot silently start an unguarded strict runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimePolicy {
    Standard,
    Protected,
}

/// Old authenticated protections remain explicitly releasable during upgrades.
pub fn protection_action_supported(version: &str, action: &str) -> bool {
    match action {
        "arm" => version == SERVICE_VERSION,
        "release" => [SERVICE_VERSION, "4.2", "4.1", "4.0", "3.1", "3.0"].contains(&version),
        _ => false,
    }
}

#[allow(dead_code, reason = "共享协议模块：运行目录仅供安全服务使用")]
pub fn runtime_dir() -> PathBuf {
    PathBuf::from(PROGRAM_DIR).join("runtime")
}
#[allow(dead_code, reason = "共享协议模块：固定内核路径仅供安全服务使用")]
pub fn kernel_path() -> PathBuf {
    PathBuf::from(PROGRAM_DIR).join("云阔mihomo.exe")
}

fn unsafe_path(value: &str) -> bool {
    let ipv6 = value.parse::<std::net::Ipv6Addr>().is_ok()
        || value.split_once('/').is_some_and(|(address, prefix)| {
            address.parse::<std::net::Ipv6Addr>().is_ok()
                && prefix.parse::<u8>().is_ok_and(|bits| bits <= 128)
        });
    if ipv6 {
        return false;
    }
    let bytes = value.as_bytes();
    // Windows 盘符必须是 ASCII 字母；::1 等 IPv6 地址的第二个字符也可能是冒号。
    let windows_drive = bytes.len() > 1 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    Path::new(value).is_absolute()
        || windows_drive
        || value.starts_with("\\\\")
        || value.split(['/', '\\']).any(|v| v == "..")
}

fn check_values(value: &serde_yaml::Value) -> Result<()> {
    match value {
        serde_yaml::Value::String(v) if unsafe_path(v) => bail!("服务配置不能引用外部文件路径"),
        serde_yaml::Value::Sequence(values) => {
            for value in values {
                check_values(value)?;
            }
        }
        serde_yaml::Value::Mapping(values) => {
            for (_, value) in values {
                check_values(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// SYSTEM 内核只使用服务自己的工作目录，不接受客户端的程序/目录路径。
pub fn sanitized_config(config: &str) -> Result<String> {
    if config.len() > MAX_REQUEST_BYTES {
        bail!("配置过大");
    }
    let mut value: serde_yaml::Value =
        serde_yaml::from_str(config).map_err(|_| anyhow!("服务配置不是有效 YAML"))?;
    let map = value
        .as_mapping_mut()
        .ok_or_else(|| anyhow!("服务配置必须为映射"))?;
    for key in [
        "external-ui",
        "external-ui-url",
        "log-file",
        "external-controller-unix",
        "external-controller-pipe",
        "listeners",
    ] {
        map.remove(serde_yaml::Value::String(key.into()));
    }
    for kind in ["proxy-providers", "rule-providers"] {
        if let Some(providers) = map
            .get_mut(serde_yaml::Value::String(kind.into()))
            .and_then(|v| v.as_mapping_mut())
        {
            for (index, (_, provider)) in providers.iter_mut().enumerate() {
                let provider = provider
                    .as_mapping_mut()
                    .ok_or_else(|| anyhow!("服务中的 provider 必须为映射"))?;
                let provider_type = provider
                    .get(serde_yaml::Value::String("type".into()))
                    .and_then(|v| v.as_str())
                    .unwrap_or("http");
                if provider_type == "file" {
                    bail!(
                        "服务模式不接受外部本地 provider 文件，请使用在线 provider 或普通代理模式"
                    );
                }
                provider.insert(
                    "path".into(),
                    format!("providers/{kind}-{index}.yaml").into(),
                );
                if let Some(url) = provider
                    .get(serde_yaml::Value::String("url".into()))
                    .and_then(|v| v.as_str())
                {
                    if !url.starts_with("https://") && !url.starts_with("http://") {
                        bail!("provider 只允许 HTTP(S) 地址");
                    }
                }
            }
        }
    }
    check_values(&value)?;
    Ok(serde_yaml::to_string(&value)?)
}

#[allow(
    dead_code,
    reason = "共享协议模块：SYSTEM 内核完整性检查由安全服务执行"
)]
pub fn verify_kernel() -> Result<PathBuf> {
    use std::os::windows::fs::MetadataExt;
    let root = PathBuf::from(PROGRAM_DIR);
    let kernel = kernel_path();
    for path in [&root, &kernel] {
        if std::fs::symlink_metadata(path)?.file_attributes() & 0x400 != 0 {
            bail!("服务程序目录不能是重解析点");
        }
    }
    if kernel.canonicalize()?.parent() != Some(root.canonicalize()?.as_path()) {
        bail!("内核路径超出受保护目录");
    }
    let current = std::env::current_exe()?.canonicalize()?;
    if current.parent() != Some(root.canonicalize()?.as_path())
        || current.file_name() != Some(std::ffi::OsStr::new("yunkuo-service.exe"))
    {
        bail!("网络保护只能由已安装的安全服务操作");
    }
    let digest = ring::digest::digest(&ring::digest::SHA256, &std::fs::read(&kernel)?);
    let hash: String = digest.as_ref().iter().map(|b| format!("{b:02X}")).collect();
    if hash != "04F8D7FC2B314771E1ECBF15951D59F0E1CB7914503C88CFB6972FF6A824E314" {
        bail!("mihomo 内核完整性校验失败");
    }
    Ok(kernel)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_executable_parameters_are_rejected() {
        assert!(
            serde_json::from_str::<StartRequest>(
                r#"{"mihomo_path":"bad.exe","data_dir":"x","config_path":"x"}"#
            )
            .is_err()
        );
    }
    #[test]
    fn execution_policy_is_required_and_only_known_policies_are_accepted() {
        assert!(serde_json::from_str::<StartRequest>(r#"{"config":"mode: global"}"#).is_err());
        assert!(
            serde_json::from_str::<StartRequest>(
                r#"{"config":"mode: global","execution_policy":"daily"}"#
            )
            .is_err()
        );
        let request: StartRequest =
            serde_json::from_str(r#"{"config":"mode: global","execution_policy":"protected"}"#)
                .unwrap();
        assert_eq!(request.execution_policy, RuntimePolicy::Protected);
    }
    #[test]
    fn old_services_only_support_explicit_recovery_release() {
        for version in ["4.2", "4.1", "4.0", "3.1", "3.0"] {
            assert!(protection_action_supported(version, "release"));
            assert!(!protection_action_supported(version, "arm"));
        }
        assert!(protection_action_supported(SERVICE_VERSION, "arm"));
        assert!(!protection_action_supported("unknown", "release"));
        assert!(!protection_action_supported(SERVICE_VERSION, "../start"));
    }
    #[test]
    fn protected_runtime_never_accepts_external_paths() {
        assert!(sanitized_config("proxies:\n- private-key-path: C:\\secret\\key\n").is_err());
        assert!(sanitized_config("proxies:\n- private-key-path: ../../secret\n").is_err());
        let safe = sanitized_config("proxies: []\nproxy-providers:\n  online:\n    type: http\n    url: https://example.test/profile\n    path: ../../other.yaml\n").unwrap();
        assert!(safe.contains("providers/proxy-providers-0.yaml"));
        assert!(!safe.contains("../../other"));
    }

    #[test]
    fn ipv6_addresses_are_not_windows_drive_paths() {
        let config = "skip-proxy: ['::1', '::', '::/0', 'a::1', 'a::/16']\nproxies:\n- name: ipv6-node\n  type: socks5\n  server: '2001:db8::1'\n  port: 1080\n";
        let once = sanitized_config(config).expect("IPv6 must not be rejected as an external path");
        let twice = sanitized_config(&once).expect("the service validates GUI output again");
        let value: serde_yaml::Value = serde_yaml::from_str(&twice).unwrap();
        assert_eq!(value["skip-proxy"][0].as_str(), Some("::1"));
        assert_eq!(value["proxies"][0]["server"].as_str(), Some("2001:db8::1"));
    }

    #[test]
    fn windows_drive_relative_unc_and_parent_paths_remain_rejected() {
        for path in [
            r"C:\secret\key",
            r"c:secret\key",
            r"Z:/secret/key",
            r"\\server\share\key",
            "../secret/key",
            "safe/../secret/key",
            "a::/../secret/key",
        ] {
            let config = format!("proxies:\n- private-key-path: '{path}'\n");
            assert!(
                sanitized_config(&config).is_err(),
                "external paths must remain rejected"
            );
        }
    }
}
