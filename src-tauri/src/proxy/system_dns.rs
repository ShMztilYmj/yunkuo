//! 系统解析器身份探测。TXT 不会被 Fake-IP 的 A 记录映射替换。
use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::sync::Mutex;

const GOOGLE: &str = "o-o.myaddr.l.google.com";
const AKAMAI: &str = "whoami.ds.akahelp.net";
static QUERY_LOCK: Mutex<()> = Mutex::new(());

pub fn public_ip(value: &str) -> Option<String> {
    let ip: IpAddr = value.parse().ok()?;
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        _ => ip,
    };
    let public = match ip {
        IpAddr::V4(v4) => {
            let [a, b, _, _] = v4.octets();
            !v4.is_private()
                && !v4.is_loopback()
                && !v4.is_link_local()
                && !v4.is_documentation()
                && a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 198 && (b == 18 || b == 19))
        }
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            (0x2000..0x4000).contains(&segments[0])
                && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
        }
    };
    public.then(|| ip.to_string())
}

fn resolver_from_txt(host: &str, chunks: &[String]) -> Option<String> {
    let text = chunks.join(" ");
    let fields: Vec<_> = text
        .split_whitespace()
        .map(|v| v.trim_matches('"'))
        .collect();
    let address = match (host, fields.as_slice()) {
        (GOOGLE, [ip]) => *ip,
        (AKAMAI, [kind, ip]) if kind.eq_ignore_ascii_case("ns") => *ip,
        _ => return None,
    };
    public_ip(address)
}

#[cfg(windows)]
fn txt_records(host: &str) -> Result<Vec<Vec<String>>, String> {
    use std::ffi::{CStr, CString};
    use windows::Win32::NetworkManagement::Dns::{
        DNS_QUERY_BYPASS_CACHE, DNS_RECORDA, DNS_TYPE_TEXT, DnsFree, DnsFreeRecordList,
        DnsQuery_UTF8,
    };
    use windows::core::PCSTR;
    struct Records(*mut DNS_RECORDA);
    impl Drop for Records {
        fn drop(&mut self) {
            // DnsQuery 返回的整个链表由 DNS API 分配，只释放一次。
            unsafe {
                DnsFree(Some(self.0.cast()), DnsFreeRecordList);
            }
        }
    }
    let name = CString::new(host).map_err(|_| "DNS 身份域名无效".to_string())?;
    let mut records = Records(std::ptr::null_mut());
    // 不指定服务器，保留系统 TUN/DNS 路径；跳过本机缓存，避免切换节点后读到旧身份。
    let status = unsafe {
        DnsQuery_UTF8(
            PCSTR(name.as_ptr().cast()),
            DNS_TYPE_TEXT,
            DNS_QUERY_BYPASS_CACHE,
            None,
            &mut records.0,
            None,
        )
    };
    if status.0 != 0 {
        return Err(format!("DNS 查询失败（代码 {}）", status.0));
    }
    let mut rows = Vec::new();
    let mut current = records.0;
    for _ in 0..64 {
        if current.is_null() {
            break;
        }
        // 记录类型、变长 TXT 字符串数组与其生命周期均由 Windows DNS API 保证。
        unsafe {
            let record = &*current;
            if record.wType == DNS_TYPE_TEXT.0 {
                let txt = &record.Data.TXT;
                if (1..=64).contains(&txt.dwStringCount) {
                    let strings = std::slice::from_raw_parts(
                        txt.pStringArray.as_ptr(),
                        txt.dwStringCount as usize,
                    );
                    let chunks = strings
                        .iter()
                        .filter(|s| !s.is_null())
                        .map(|s| CStr::from_ptr(s.0.cast()).to_string_lossy().into_owned())
                        .collect();
                    rows.push(chunks);
                }
            }
            current = record.pNext;
        }
    }
    Ok(rows)
}

#[cfg(not(windows))]
fn txt_records(_host: &str) -> Result<Vec<Vec<String>>, String> {
    Err("系统 TXT 身份探测当前仅支持 Windows".into())
}

pub fn probe(rounds: u32) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    // 前一轮超时后仍在系统解析时，不堆积不可取消的阻塞查询。
    let _guard = QUERY_LOCK
        .try_lock()
        .map_err(|_| "上一轮系统 DNS 查询尚未结束，请稍后重试".to_string())?;
    let mut resolvers = BTreeMap::<String, BTreeSet<String>>::new();
    let mut failures = Vec::new();
    for host in [GOOGLE, AKAMAI] {
        for _ in 0..rounds.clamp(1, 3) {
            match txt_records(host) {
                Ok(rows) => {
                    let ips: Vec<_> = rows
                        .iter()
                        .filter_map(|row| resolver_from_txt(host, row))
                        .collect();
                    if ips.is_empty() {
                        failures.push(format!(
                            "{host}：未返回公网解析器身份（已排除 ECS、内网及合成地址）"
                        ));
                        break;
                    }
                    for ip in ips {
                        resolvers
                            .entry(ip)
                            .or_default()
                            .insert(format!("系统 DNS · {host} (TXT)"));
                    }
                    // 同一身份服务的重复查询不能证明完整路径覆盖，取得结果后停止重试。
                    break;
                }
                Err(error) => failures.push(format!("{host}：{error}")),
            }
        }
    }
    if resolvers.is_empty() {
        return Err(format!(
            "系统 DNS TXT 探测未取得解析器：{}",
            failures.join("；")
        ));
    }
    Ok(resolvers)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identity_txt_excludes_ecs_and_synthetic_addresses() {
        let strings = |values: &[&str]| values.iter().map(|v| v.to_string()).collect::<Vec<_>>();
        assert_eq!(
            resolver_from_txt(GOOGLE, &strings(&["8.8.8.8"])),
            Some("8.8.8.8".into())
        );
        assert_eq!(
            resolver_from_txt(AKAMAI, &strings(&["ns", "1.1.1.1"])),
            Some("1.1.1.1".into())
        );
        for values in [
            &["ecs", "9.9.9.0/24"][..],
            &["ip", "9.9.9.9"],
            &["ns", "198.18.0.84"],
        ] {
            assert_eq!(resolver_from_txt(AKAMAI, &strings(values)), None);
        }
        assert_eq!(
            resolver_from_txt(GOOGLE, &strings(&["edns0-client-subnet 9.9.9.0/24"])),
            None
        );
    }
    #[test]
    fn excludes_non_public_dns_addresses() {
        for ip in [
            "198.18.0.84",
            "198.19.1.1",
            "192.168.1.1",
            "100.64.0.1",
            "169.254.1.1",
            "0.0.0.0",
            "224.0.0.1",
            "999.1.1.1",
            "::1",
            "fc00::1",
            "2001:db8::1",
        ] {
            assert_eq!(public_ip(ip), None, "{ip}");
        }
        assert_eq!(
            public_ip("2606:4700:4700::1111"),
            Some("2606:4700:4700::1111".into())
        );
    }
}
