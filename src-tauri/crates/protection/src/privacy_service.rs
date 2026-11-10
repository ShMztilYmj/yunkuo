//! 服务持有的网络保护生命周期。持续检查只能收紧；重新放行必须显式重新启用保护。
use crate::{
    network_lock::{NetworkLock, tunnel_luid},
    privacy::{self, Endpoint},
    service_protocol,
};
use anyhow::{Result, anyhow, bail};
use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

#[derive(Default)]
struct State {
    armed: bool,
    healthy: bool,
    endpoints: Vec<Endpoint>,
    tunnel: Option<u64>,
    error: Option<String>,
    /// 最近一次逐项复核结果。保护运行期间每秒刷新，阻断后保留便于排障。
    checks: Vec<Check>,
    checked_at: Option<i64>,
}

/// 严格保护的单项体检结论。服务端决定顺序与文案，前端只负责按顺序展示。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub key: &'static str,
    pub title: &'static str,
    pub ok: bool,
    pub detail: String,
}

impl Check {
    fn pass(key: &'static str, title: &'static str, detail: impl Into<String>) -> Self {
        Self {
            key,
            title,
            ok: true,
            detail: detail.into(),
        }
    }

    fn fail(key: &'static str, title: &'static str, detail: impl Into<String>) -> Self {
        Self {
            key,
            title,
            ok: false,
            detail: detail.into(),
        }
    }
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| Mutex::new(State::default()));

type BootstrapResolver = fn(&str) -> Result<Vec<std::net::Ipv4Addr>>;
static BOOTSTRAP_RESOLVER: std::sync::OnceLock<BootstrapResolver> = std::sync::OnceLock::new();

/// The service supplies encrypted bootstrap DNS. WFP permits only its fixed
/// HTTPS resolver endpoints while other direct connections stay blocked.
pub fn install_bootstrap_resolver(resolver: BootstrapResolver) {
    let _ = BOOTSTRAP_RESOLVER.set(resolver);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub armed: bool,
    pub healthy: bool,
    pub tunnel_on: bool,
    pub error: Option<String>,
    /// 最近一次体检的分项结果；从未复核时为空。
    pub checks: Vec<Check>,
    /// 最近一次体检的 Unix 毫秒时间；从未复核时为 None。
    pub checked_at: Option<i64>,
}

pub fn initialize() -> Result<()> {
    let lock = NetworkLock::open()?;
    if lock.installed()? {
        // 上次服务退出后仍在保护；重启时先撤掉旧网卡放行，不能自动解除保护。
        let mut state = STATE
            .lock()
            .map_err(|_| anyhow!("网络保护状态锁失效，保护规则保留"))?;
        state.armed = true;
        lock.apply(&service_protocol::kernel_path(), &[], None)?;
        state.error = Some("保护规则已保留，请重新启用网络保护以恢复代理联网".into());
    }
    Ok(())
}

pub fn status() -> Result<Status> {
    let mut state = STATE
        .lock()
        .map_err(|_| anyhow!("网络保护状态锁失效，保护规则保留"))?;
    let lock = NetworkLock::open()?;
    let expected = 6 + state.endpoints.len() * 2 + usize::from(state.tunnel.is_some());
    let intact = state.armed && lock.rule_count()? == expected;
    if state.armed && !intact {
        seal_locked(&mut state, "保护规则不完整，已重新阻断，请重新启用保护")?;
    }
    Ok(Status {
        armed: state.armed,
        healthy: state.healthy && intact,
        tunnel_on: state.tunnel.is_some() && tunnel_luid() == state.tunnel,
        error: state.error.clone(),
        checks: state.checks.clone(),
        checked_at: state.checked_at,
    })
}

fn seal_locked(state: &mut State, reason: &str) -> Result<()> {
    if !state.armed {
        return Ok(());
    }
    NetworkLock::open()?.apply(&service_protocol::kernel_path(), &[], None)?;
    state.healthy = false;
    state.tunnel = None;
    state.endpoints.clear();
    state.error = Some(reason.into());
    Ok(())
}

pub fn seal(reason: &str) -> Result<()> {
    let mut state = STATE
        .lock()
        .map_err(|_| anyhow!("网络保护状态锁失效，保护规则保留"))?;
    seal_locked(&mut state, reason)
}

pub fn release() -> Result<()> {
    let mut state = STATE
        .lock()
        .map_err(|_| anyhow!("网络保护状态锁失效，保护规则保留"))?;
    NetworkLock::open()?.release()?;
    *state = State::default();
    Ok(())
}

fn config() -> Result<serde_yaml::Value> {
    privacy::validate(&std::fs::read_to_string(
        service_protocol::runtime_dir().join("config.yaml"),
    )?)
}

fn controller(value: &serde_yaml::Value, path: &str) -> Result<serde_json::Value> {
    static CLIENT: once_cell::sync::OnceCell<reqwest::blocking::Client> =
        once_cell::sync::OnceCell::new();
    let client =
        CLIENT.get_or_try_init(|| crate::http::direct_blocking(Duration::from_millis(500)))?;
    let address = value["external-controller"]
        .as_str()
        .ok_or_else(|| anyhow!("控制器地址缺失"))?;
    let port = address
        .strip_prefix("127.0.0.1:")
        .and_then(|v| v.parse::<u16>().ok())
        .filter(|v| *v > 0)
        .ok_or_else(|| anyhow!("保护仅接受本地控制器"))?;
    Ok(client
        .get(format!("http://127.0.0.1:{port}{path}"))
        .bearer_auth(value["secret"].as_str().unwrap_or(""))
        .send()?
        .error_for_status()?
        .json()?)
}

fn now_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn on_off(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "开启",
        Some(false) => "关闭",
        None => "未知",
    }
}

/// 逐项执行严格保护的实时校验并保留结果，任何一项失败都不影响其余项出结论。
/// 顺序固定为「内核身份 → TUN 与路由 → 出口节点 → 系统代理」。
fn live_checks(value: &serde_yaml::Value) -> Vec<Check> {
    let mut checks = Vec::with_capacity(4);
    checks.push(match controller(value, "/version") {
        Ok(version) if version["meta"].as_bool() == Some(true) => {
            Check::pass("engine", "内核身份", "mihomo 服务 · 严格保护模式")
        }
        Ok(_) => Check::fail(
            "engine",
            "内核身份",
            "严格保护要求 mihomo 服务，请更新服务并重新应用方案",
        ),
        Err(_) => Check::fail(
            "engine",
            "内核身份",
            "无法连接 mihomo 控制器，请确认严格运行时已启动",
        ),
    });
    checks.push(match controller(value, "/configs") {
        Ok(live) => {
            let mode = live["mode"].as_str().unwrap_or("未知");
            let tun = live["tun"]["enable"].as_bool();
            let ipv6 = live["ipv6"].as_bool();
            let device = live["tun"]["device"]
                .as_str()
                .filter(|device| !device.is_empty())
                .unwrap_or("TUN");
            if mode == "global" && tun == Some(true) && ipv6 == Some(false) {
                Check::pass(
                    "tunnel",
                    "TUN 与路由",
                    format!("{device} · 全局模式 · IPv6 关闭"),
                )
            } else {
                Check::fail(
                    "tunnel",
                    "TUN 与路由",
                    format!(
                        "TUN、全局模式或 IPv6 阻断未实际生效（{mode} · TUN {} · IPv6 {}）",
                        on_off(tun),
                        on_off(ipv6)
                    ),
                )
            }
        }
        Err(_) => Check::fail("tunnel", "TUN 与路由", "无法读取内核运行配置"),
    });
    checks.push(match controller(value, "/proxies") {
        Ok(live) => match privacy::selected_proxy(&live["proxies"]) {
            Some(name) => Check::pass("node", "出口节点", format!("全局代理 · {name}")),
            None => Check::fail(
                "node",
                "出口节点",
                "全局代理未选择有效节点，禁止 DIRECT 或未知代理链",
            ),
        },
        Err(_) => Check::fail("node", "出口节点", "无法读取全局代理选择"),
    });
    let port = value["mixed-port"].as_u64().unwrap_or(0) as u16;
    checks.push(if owner_proxy_on(port) {
        Check::pass(
            "sysproxy",
            "系统代理",
            format!("127.0.0.1:{port} 已由当前用户指向本软件"),
        )
    } else {
        Check::fail(
            "sysproxy",
            "系统代理",
            "当前用户的系统代理未开启或未指向本软件",
        )
    });
    checks
}

fn owner_proxy_on(port: u16) -> bool {
    use windows::Win32::System::Registry::{
        HKEY_USERS, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
    };
    use windows::core::PCWSTR;
    let Ok(sid) = std::fs::read_to_string(
        std::path::Path::new(service_protocol::PROGRAM_DIR).join("owner_sid.txt"),
    ) else {
        return false;
    };
    let sid = sid.trim();
    if !sid.starts_with("S-1-")
        || !sid
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b'S' || b == b'-')
    {
        return false;
    }
    let key: Vec<u16> =
        format!("{sid}\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings\0")
            .encode_utf16()
            .collect();
    let enabled: Vec<u16> = "ProxyEnable\0".encode_utf16().collect();
    let server: Vec<u16> = "ProxyServer\0".encode_utf16().collect();
    let mut on = 0u32;
    let mut length = 4u32;
    if unsafe {
        RegGetValueW(
            HKEY_USERS,
            PCWSTR(key.as_ptr()),
            PCWSTR(enabled.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut on as *mut _ as _),
            Some(&mut length),
        )
    }
    .0 != 0
        || on != 1
    {
        return false;
    }
    let mut text = [0u16; 512];
    length = std::mem::size_of_val(&text) as u32;
    if unsafe {
        RegGetValueW(
            HKEY_USERS,
            PCWSTR(key.as_ptr()),
            PCWSTR(server.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(text.as_mut_ptr() as _),
            Some(&mut length),
        )
    }
    .0 != 0
    {
        return false;
    }
    let text =
        String::from_utf16_lossy(&text[..text.iter().position(|v| *v == 0).unwrap_or(text.len())]);
    let expected = format!("127.0.0.1:{port}");
    text == expected
        || (!text.is_empty()
            && text.split(';').filter(|s| !s.is_empty()).all(|s| {
                s.split_once('=')
                    .is_some_and(|(_, address)| address == expected)
            }))
}

fn endpoints(value: &serde_yaml::Value) -> Result<Vec<Endpoint>> {
    let mut servers = privacy::node_servers(value)?;
    if let Some(providers) = value["proxy-providers"].as_mapping() {
        for (_, provider) in providers {
            let path = provider["path"]
                .as_str()
                .filter(|p| p.starts_with("providers/") && !p.contains("..") && !p.contains('\\'))
                .ok_or_else(|| anyhow!("provider 缓存路径不安全"))?;
            let data: serde_yaml::Value = serde_yaml::from_str(
                &std::fs::read_to_string(service_protocol::runtime_dir().join("mihomo").join(path))
                    .map_err(|_| anyhow!("节点订阅缓存尚未就绪，请等待下载后重试"))?,
            )?;
            servers.extend(privacy::node_servers(&data)?);
        }
    }
    if servers.is_empty() {
        bail!("没有可约束到固定地址的代理节点");
    }
    if servers.len() > 512 {
        bail!("节点超过严格保护上限 512，请精简订阅");
    }
    let started = std::time::Instant::now();
    let mut endpoints = BTreeSet::new();
    for (host, port) in servers {
        if started.elapsed() > Duration::from_secs(8) {
            bail!("节点解析准备超时，请精简订阅后重试；保护阻断保留");
        }
        let ips = if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
            vec![ip]
        } else {
            if host.parse::<std::net::Ipv6Addr>().is_ok() {
                bail!("严格保护暂不支持仅 IPv6 地址的代理节点");
            }
            let ips = BOOTSTRAP_RESOLVER
                .get()
                .ok_or_else(|| anyhow!("服务引导解析器尚未安装"))?(&host)?;
            if ips.is_empty() {
                bail!("节点没有可用的 IPv4 地址");
            }
            ips
        };
        for ip in ips {
            if ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_multicast()
            {
                bail!("严格保护不接受内网或保留服务器地址");
            }
            endpoints.insert(Endpoint { ip, port });
        }
    }
    for ip in privacy::BOOTSTRAP_IPS {
        endpoints.insert(Endpoint {
            ip: ip.parse()?,
            port: 443,
        });
    }
    Ok(endpoints.into_iter().collect())
}

pub fn arm() -> Result<Status> {
    let mut state = STATE
        .lock()
        .map_err(|_| anyhow!("网络保护状态锁失效，保护规则保留"))?;
    let lock = NetworkLock::open()?;
    // 先阻断再解析和核验，任何失败都保留阻断；旧连接也由 WFP 重新授权。
    lock.apply(&service_protocol::verify_kernel()?, &[], None)?;
    state.armed = true;
    state.healthy = false;
    state.tunnel = None;
    state.endpoints.clear();
    let result = (|| {
        let value = config()?;
        // 限制解析准备时间，超时保持阻断；最多仅解析已有节点。
        let endpoints = endpoints(&value)?;
        let tunnel = tunnel_luid().ok_or_else(|| anyhow!("专用 TUN 网卡未启动"))?;
        let checks = live_checks(&value);
        // 先落盘体检结果：失败时界面仍能逐项说明原因，而不是只剩一条结论。
        state.checks = checks;
        state.checked_at = Some(now_millis());
        if let Some(failed) = state.checks.iter().find(|check| !check.ok) {
            bail!("{}", failed.detail);
        }
        lock.apply(&service_protocol::kernel_path(), &endpoints, Some(tunnel))?;
        state.endpoints = endpoints;
        state.tunnel = Some(tunnel);
        state.healthy = true;
        state.error = None;
        Ok(())
    })();
    if let Err(error) = result {
        state.error = Some(format!("{error:#}"));
        return Err(error);
    }
    drop(state);
    status()
}

pub fn watch() {
    // 状态锁失效时不得凭旧状态重新放行，也不得让监视线程重复 panic。
    let Ok(mut state) = STATE.lock() else {
        let _ = NetworkLock::open()
            .and_then(|lock| lock.apply(&service_protocol::kernel_path(), &[], None));
        return;
    };
    if !state.armed {
        return;
    }
    let result = (|| {
        let lock = NetworkLock::open()?;
        if lock.rule_count()? != 6 + state.endpoints.len() * 2 + usize::from(state.tunnel.is_some())
        {
            bail!("网络保护规则已变化");
        }
        if state.healthy {
            if tunnel_luid() != state.tunnel {
                bail!("TUN 网卡消失或发生变化");
            }
            let value = config()?;
            let checks = live_checks(&value);
            // 复核结果先写入状态：即使随后因某项失败而阻断，界面仍能看到是哪一项。
            state.checks = checks;
            state.checked_at = Some(now_millis());
            if let Some(failed) = state.checks.iter().find(|check| !check.ok) {
                bail!("{}", failed.detail);
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        let _ = seal_locked(&mut state, &format!("网络状态变化，已阻断：{error:#}"));
    }
}

#[cfg(test)]
mod performance_tests {
    use super::*;

    #[test]
    fn status_exposes_checkup_with_camel_case_fields() {
        let status = Status {
            armed: true,
            healthy: true,
            tunnel_on: true,
            error: None,
            checks: vec![Check::pass("engine", "内核身份", "mihomo 服务")],
            checked_at: Some(1_700_000_000_000),
        };
        let json = serde_json::to_value(status).unwrap();
        assert_eq!(json["tunnelOn"], true);
        assert_eq!(json["checkedAt"], 1_700_000_000_000i64);
        assert_eq!(json["checks"][0]["key"], "engine");
        assert_eq!(json["checks"][0]["ok"], true);
    }

    #[test]
    fn live_checks_report_each_gate_in_order() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let worker = std::thread::spawn(move || {
            for _ in 0..3 {
                let request = server
                    .recv_timeout(Duration::from_secs(3))
                    .unwrap()
                    .expect("local checkup request");
                let body = match request.url() {
                    "/version" => r#"{"meta":true}"#,
                    "/configs" => {
                        r#"{"mode":"global","ipv6":false,"tun":{"enable":true,"device":"云阔TUN"}}"#
                    }
                    "/proxies" => {
                        r#"{"proxies":{"GLOBAL":{"now":"node"},"node":{"type":"Trojan"}}}"#
                    }
                    _ => r#"{}"#,
                };
                request
                    .respond(tiny_http::Response::from_string(body))
                    .unwrap();
            }
        });
        let value = serde_yaml::to_value(serde_json::json!({
            "external-controller": address.to_string(),
            "secret": "fixture-secret",
            "mixed-port": 7890
        }))
        .unwrap();
        let checks = live_checks(&value);
        assert_eq!(
            checks.iter().map(|check| check.key).collect::<Vec<_>>(),
            ["engine", "tunnel", "node", "sysproxy"]
        );
        assert!(checks[0].ok, "mihomo meta flag must pass");
        assert!(checks[1].ok, "global TUN without IPv6 must pass");
        assert!(checks[2].ok, "a real Trojan node must pass");
        assert!(checks[1].detail.contains("云阔TUN"));
        assert!(checks[2].detail.contains("node"));
        worker.join().unwrap();
    }

    #[test]
    fn health_checks_reuse_the_local_connection() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let worker = std::thread::spawn(move || {
            let mut peers = Vec::new();
            for _ in 0..2 {
                let request = server
                    .recv_timeout(Duration::from_secs(3))
                    .unwrap()
                    .expect("local health request");
                assert_eq!(request.url(), "/configs");
                assert!(
                    request
                        .headers()
                        .iter()
                        .any(|h| h.field.equiv("Authorization")
                            && h.value.as_str() == "Bearer fixture-secret")
                );
                peers.push(*request.remote_addr().unwrap());
                request
                    .respond(tiny_http::Response::from_string(r#"{"mode":"global"}"#))
                    .unwrap();
            }
            peers
        });
        let value = serde_yaml::to_value(serde_json::json!({"external-controller":address.to_string(), "secret":"fixture-secret"})).unwrap();
        for _ in 0..2 {
            assert_eq!(controller(&value, "/configs").unwrap()["mode"], "global");
        }
        let peers = worker.join().unwrap();
        assert_eq!(
            peers[0], peers[1],
            "health probes should share their HTTP connection"
        );
    }
}
