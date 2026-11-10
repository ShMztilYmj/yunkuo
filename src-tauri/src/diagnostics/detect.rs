use crate::config::AppConfig;
use crate::proxy::core;
use serde::Serialize;
use std::time::{Duration, Instant};

pub(crate) const REPORT_LIFETIME: Duration = Duration::from_secs(300);

/// 仅在当前应用会话保留已完成的浏览器检测；启动入口不能自行提交评分。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WebScanState {
    score: Option<i32>,
    revision: u64,
    recorded_at: Option<Instant>,
    basis: Option<super::report::EnvironmentBasis>,
    pending: Option<(super::report::EnvironmentBasis, Instant, Option<String>)>,
}

impl WebScanState {
    pub fn record(&mut self, score: Option<i32>) -> Result<(), String> {
        self.revision = self.revision.wrapping_add(1);
        self.score = None;
        self.recorded_at = None;
        self.basis = None;
        self.pending = None;
        if let Some(score) = score {
            if !(0..=100).contains(&score) {
                return Err("环境评分必须在 0–100 之间，请重新检测".into());
            }
            self.score = Some(score);
            self.recorded_at = Some(Instant::now());
        }
        Ok(())
    }

    pub fn score(&self) -> Option<i32> {
        self.recorded_at
            .filter(|at| at.elapsed() <= REPORT_LIFETIME)
            .and(self.score)
    }

    pub(crate) fn record_for(
        &mut self,
        score: Option<i32>,
        basis: super::report::EnvironmentBasis,
    ) -> Result<(), String> {
        if score.is_none() {
            self.record(None)?;
            self.pending = Some((basis, Instant::now(), None));
            return Ok(());
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|(_, _, ticket)| ticket.is_some())
        {
            return Err("当前检测轮次需要原始票据，请等待本轮完成".into());
        }
        let current = self
            .pending
            .as_ref()
            .is_some_and(|(start, at, _)| start == &basis && at.elapsed() <= REPORT_LIFETIME);
        self.record(score)?;
        if !current {
            self.record(None)?;
            return Err("检测期间环境已变化或检测轮次已过期，请重新检测".into());
        }
        self.basis = Some(basis);
        Ok(())
    }

    pub(crate) fn begin_scan(&mut self, basis: super::report::EnvironmentBasis) -> String {
        let ticket = uuid::Uuid::new_v4().simple().to_string();
        // None 不会产生校验错误。
        let _ = self.record(None);
        self.pending = Some((basis, Instant::now(), Some(ticket.clone())));
        ticket
    }

    pub(crate) fn finish_scan(
        &mut self,
        ticket: &str,
        score: i32,
        basis: super::report::EnvironmentBasis,
    ) -> Result<(), String> {
        let Some((started_basis, started, Some(current))) = self.pending.as_ref() else {
            return Err("检测轮次已取消或完成，请重新检测".into());
        };
        if current != ticket {
            // 迟到请求只丢弃自身结果，不能撤销较新轮次。
            return Err("检测结果不属于当前轮次，已丢弃".into());
        }
        if started_basis != &basis || started.elapsed() > REPORT_LIFETIME {
            self.record(None)?;
            return Err("检测期间环境已变化或轮次已过期，请重新检测".into());
        }
        self.record(Some(score))?;
        self.basis = Some(basis);
        Ok(())
    }

    pub(crate) fn valid_for(&self, basis: &super::report::EnvironmentBasis) -> bool {
        self.score().is_some() && self.basis.as_ref() == Some(basis)
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn age_seconds(&self) -> Option<u64> {
        self.recorded_at.map(|at| at.elapsed().as_secs())
    }
}

pub type WebScanMutex = parking_lot::Mutex<WebScanState>;

/// 后端原生检测结果
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NativeDetect {
    pub core_running: bool,
    /// 系统代理是否已指向本应用端口
    pub sysproxy_on: bool,
    pub sysproxy_host: Option<String>,
    pub sysproxy_port: Option<u16>,
    /// TUN 是否开启
    pub tun_on: bool,
    pub global_on: bool,
    pub proxy_node_selected: bool,
    pub privacy_armed: bool,
    pub privacy_healthy: bool,
    pub privacy_error: Option<String>,
    /// 出口 IP 信息
    pub exit_ip: Option<String>,
    pub exit_country: Option<String>,
    pub exit_country_name: Option<String>,
    pub country_allowed: Option<bool>,
    /// None 表示未检测（例如关闭了连通性要求）。
    pub claude_reachable: Option<bool>,
    pub claude_status: Option<u16>,
    pub claude_error: Option<String>,
    /// 当前系统时区
    pub current_tz: String,
    pub current_tz_iana: String,
    pub exit_timezone: Option<String>,
    pub timezone_matches: Option<bool>,
    /// 是否命中 Claude 封禁时区（Asia/Shanghai / Asia/Urumqi）
    pub tz_is_cn: bool,
}

/// 检测页和浏览器启动共用的本机网络事实，不触发公网归属地或站点探测。
#[derive(Default)]
pub(crate) struct ProxyNetwork {
    pub sysproxy_on: bool,
    pub sysproxy_host: Option<String>,
    pub sysproxy_port: Option<u16>,
    pub tun_on: bool,
    pub mode: Option<String>,
}

pub(crate) async fn proxy_network(cfg: &AppConfig) -> ProxyNetwork {
    let mut result = ProxyNetwork::default();
    if let Ok(configs) = core::controller_request(cfg, "GET", "/configs", None).await {
        result.tun_on = configs["tun"]["enable"].as_bool() == Some(true);
        result.mode = configs["mode"].as_str().map(str::to_string);
    }
    if let Ok(proxy) = crate::proxy::sysproxy::get_sysproxy() {
        result.sysproxy_on =
            proxy.enable && proxy.host == "127.0.0.1" && proxy.port == cfg.mixed_port;
        result.sysproxy_host = Some(proxy.host);
        result.sysproxy_port = Some(proxy.port);
    }
    result
}

pub async fn detect_native(cfg: &AppConfig, data_dir: &std::path::Path) -> NativeDetect {
    let mut r = NativeDetect::default();
    // 内核探测带重试 + 诊断日志：重启窗口与真实失联都留下证据链
    r.core_running = {
        let mut ok = false;
        for i in 0..3 {
            let attempt = core::probe_with_fallback(cfg, data_dir).await;
            if attempt.is_ok() {
                ok = true;
                core::write_probe_log(
                    data_dir,
                    &format!("attempt#{i} ok (ret={:?})", attempt.unwrap_or_default()),
                );
                break;
            }
            if i < 2 {
                tokio::time::sleep(std::time::Duration::from_millis(600)).await;
            }
        }
        ok
    };

    let network = proxy_network(cfg).await;
    r.tun_on = network.tun_on;
    r.global_on = network.mode.as_deref() == Some("global");
    r.sysproxy_on = network.sysproxy_on;
    r.sysproxy_host = network.sysproxy_host;
    r.sysproxy_port = network.sysproxy_port;
    if r.core_running {
        if let Ok(proxies) = core::controller_request(cfg, "GET", "/proxies", None).await {
            r.proxy_node_selected = crate::privacy::proxy_selected(&proxies["proxies"]);
        }
    }
    match core::privacy_status().await {
        Ok(status) => {
            r.privacy_armed = status["armed"].as_bool() == Some(true);
            r.privacy_healthy = status["healthy"].as_bool() == Some(true)
                && status["tunnelOn"].as_bool() == Some(true);
            r.privacy_error = status["error"].as_str().map(str::to_string);
        }
        Err(_) => {
            r.privacy_error = Some(
                "新版网络保护服务未就绪，请先到设置页安装或修复服务，再到代理中心启用 AI 严格保护"
                    .into(),
            )
        }
    }

    // 当前时区
    if let Ok(tz) = super::timezone::get_current_tz() {
        r.tz_is_cn = matches!(tz.iana_guess.as_str(), "Asia/Shanghai" | "Asia/Urumqi");
        r.current_tz = tz.key_name;
        r.current_tz_iana = tz.iana_guess;
    }

    // 代理未运行时不向公网发送直连探测。
    let proxy_port = if r.core_running {
        Some(cfg.mixed_port)
    } else {
        None
    };
    let (geo, connectivity) = tokio::join!(
        async {
            if proxy_port.is_some() {
                super::timezone::fetch_ip_geo(proxy_port).await
            } else {
                Err(anyhow::anyhow!("代理未运行"))
            }
        },
        async {
            if cfg.require_claude_reachable && r.core_running {
                let client = core::build_client_with_proxy(cfg.mixed_port);
                Some(match client {
                    Ok(client) => probe_claude(&client, "https://claude.ai/").await,
                    Err(_) => (false, None, Some("无法创建代理连通性检测连接".into())),
                })
            } else {
                None
            }
        }
    );
    match geo {
        Ok(geo) => {
            r.timezone_matches = timezone_matches_exit(&r.current_tz, &geo.timezone);
            r.exit_timezone = Some(geo.timezone);
            r.exit_ip = Some(geo.ip);
            r.exit_country = Some(geo.country_code.clone());
            r.exit_country_name = Some(geo.country_name);
            r.country_allowed = Some(!cfg.blocked_countries.contains(&geo.country_code));
        }
        Err(_) => {}
    }

    if let Some((reachable, status, error)) = connectivity {
        r.claude_reachable = Some(reachable);
        r.claude_status = status;
        r.claude_error = error;
    }
    r
}

fn timezone_matches_exit(current_windows: &str, exit_iana: &str) -> Option<bool> {
    if current_windows.is_empty() {
        return None;
    }
    super::timezone::iana_to_windows(exit_iana)
        .map(|expected| current_windows.eq_ignore_ascii_case(expected))
}

/// 经应用代理访问页面；HTTP 拦截、服务错误和网络错误均不算可达。
async fn probe_claude(client: &reqwest::Client, url: &str) -> (bool, Option<u16>, Option<String>) {
    let started = Instant::now();
    match client.get(url).timeout(Duration::from_secs(8)).send().await {
        Ok(response) => {
            let status = response.status();
            let reachable = status.is_success();
            let error = if reachable {
                None
            } else {
                Some(format!("claude.ai 返回 HTTP {}", status.as_u16()))
            };
            (reachable, Some(status.as_u16()), error)
        }
        Err(error) => {
            let message = if error.is_timeout() {
                format!("claude.ai 连接超时（{} 秒）", started.elapsed().as_secs())
            } else {
                "无法通过代理连接 claude.ai，请检查节点或网络".into()
            };
            (false, None, Some(message))
        }
    }
}

/// 启动门控判定（后端兜底校验）
pub fn gate_passes(
    cfg: &AppConfig,
    native: &NativeDetect,
    web_score: Option<i32>,
) -> (bool, Vec<String>) {
    let mut failures = Vec::new();

    if !native.core_running {
        failures.push("代理内核未运行".into());
    }
    if !native.tun_on {
        failures.push("TUN 未实际开启".into());
    }
    if !native.sysproxy_on {
        failures.push("系统代理未开启或未指向本软件".into());
    }
    if !native.global_on {
        failures.push("必须使用全局代理模式".into());
    }
    if !native.proxy_node_selected {
        failures.push("全局代理未选择有效节点（禁止 DIRECT）".into());
    }
    match native.country_allowed {
        Some(false) => failures.push(format!(
            "出口 IP 位于受限地区: {}",
            native.exit_country.clone().unwrap_or_default()
        )),
        None => failures.push("无法获取出口 IP 归属地".into()),
        _ => {}
    }
    if cfg.require_claude_reachable && native.claude_reachable != Some(true) {
        failures.push(
            native
                .claude_error
                .clone()
                .unwrap_or_else(|| "claude.ai 连通性未通过，请检查代理节点后重新检测".into()),
        );
    }
    if native.tz_is_cn {
        failures.push("系统时区仍为 Asia/Shanghai / Asia/Urumqi，请先同步时区".into());
    }
    match native.timezone_matches {
        Some(true) => {}
        Some(false) => failures.push("系统时区与当前出口不匹配，请按出口 IP 同步时区".into()),
        None => failures.push("无法确认系统时区与出口一致，请重新检测并同步时区".into()),
    }
    match web_score {
        None => failures.push("尚未完成环境检测，请到「环境检测并修复」页执行检测".into()),
        Some(score) if !(0..=100).contains(&score) => {
            failures.push("环境评分无效，请重新检测".into());
        }
        Some(score) if score >= cfg.gate_threshold => failures.push(format!(
            "环境风险评分 {score} 达到或超过阈值 {}",
            cfg.gate_threshold
        )),
        _ => {}
    }
    (failures.is_empty(), failures)
}

/// 启动的硬性要求：内核在跑、TUN 实际开启、系统代理指向本软件。
///
/// 其余检查（出口地区、连通性、时区、评分）在用户确认后可以让路，
/// 这三条不行——它们决定了流量是否真的经过本软件。
pub fn strict_failures(native: &NativeDetect) -> Vec<String> {
    let mut failures = Vec::new();
    if !native.core_running {
        failures.push("代理内核未运行".into());
    }
    if !native.tun_on {
        failures.push("TUN 未实际开启".into());
    }
    if !native.sysproxy_on {
        failures.push("系统代理未开启或未指向本软件".into());
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn allowed_native() -> NativeDetect {
        NativeDetect {
            core_running: true,
            sysproxy_on: true,
            tun_on: true,
            global_on: true,
            proxy_node_selected: true,
            privacy_armed: true,
            privacy_healthy: true,
            country_allowed: Some(true),
            claude_reachable: Some(true),
            claude_status: Some(200),
            timezone_matches: Some(true),
            ..Default::default()
        }
    }

    #[test]
    fn timezone_unknown_and_mismatch_never_authorize_launch() {
        let cfg = AppConfig::default();
        for matches in [None, Some(false)] {
            let mut native = allowed_native();
            native.timezone_matches = matches;
            assert!(!gate_passes(&cfg, &native, Some(10)).0);
        }
        assert_eq!(timezone_matches_exit("", "America/New_York"), None);
        assert_eq!(
            timezone_matches_exit("Eastern Standard Time", "unknown"),
            None
        );
        assert_eq!(
            timezone_matches_exit("Eastern Standard Time", "America/New_York"),
            Some(true)
        );
        assert_eq!(
            timezone_matches_exit("Pacific Standard Time", "America/New_York"),
            Some(false)
        );
    }

    #[test]
    fn no_scan_cannot_authorize_any_launch() {
        let cfg = AppConfig::default();
        let native = allowed_native();
        let scan = WebScanState::default();
        assert!(!gate_passes(&cfg, &native, scan.score()).0);
    }

    #[test]
    fn every_protection_is_required_independently() {
        let cfg = AppConfig::default();
        for field in 0..4 {
            let mut native = allowed_native();
            match field {
                0 => native.tun_on = false,
                1 => native.sysproxy_on = false,
                2 => native.global_on = false,
                _ => native.proxy_node_selected = false,
            }
            assert!(
                !gate_passes(&cfg, &native, Some(10)).0,
                "missing field {field}"
            );
        }
    }

    #[test]
    fn strict_protection_is_not_a_launch_requirement() {
        // 严格保护属于可选加固：未启用时不再拦截启动，也不再出现在检测结论里。
        let cfg = AppConfig::default();
        let mut native = allowed_native();
        native.privacy_armed = false;
        native.privacy_healthy = false;
        assert!(gate_passes(&cfg, &native, Some(10)).0);
    }

    #[test]
    fn tun_configuration_alone_cannot_authorize_launch() {
        let cfg = AppConfig {
            enable_tun: true,
            ..Default::default()
        };
        let mut native = allowed_native();
        native.tun_on = false;
        native.sysproxy_on = false;
        assert!(!gate_passes(&cfg, &native, Some(10)).0);
    }

    #[test]
    fn rescan_revokes_previous_score() {
        let cfg = AppConfig::default();
        let native = allowed_native();
        let mut scan = WebScanState::default();
        scan.record(Some(10)).unwrap();
        assert!(gate_passes(&cfg, &native, scan.score()).0);
        scan.record(None).unwrap();
        assert!(!gate_passes(&cfg, &native, scan.score()).0);
    }

    #[test]
    fn replacing_even_the_same_score_invalidates_inflight_snapshot() {
        let mut scan = WebScanState::default();
        scan.record(Some(10)).unwrap();
        let snapshot = scan.clone();
        scan.record(None).unwrap();
        scan.record(Some(10)).unwrap();
        assert_ne!(scan, snapshot);
    }

    #[test]
    fn report_requires_same_environment_as_started_scan() {
        let cfg = AppConfig::default();
        let baseline = super::super::report::EnvironmentBasis::from_config(&cfg, 1, 1);
        let changed = super::super::report::EnvironmentBasis::from_config(&cfg, 2, 1);
        let mut scan = WebScanState::default();
        assert!(scan.record_for(Some(10), baseline.clone()).is_err());
        scan.record_for(None, baseline.clone()).unwrap();
        assert!(scan.record_for(Some(10), changed).is_err());
        assert_eq!(scan.score(), None);
        scan.record_for(None, baseline.clone()).unwrap();
        scan.record_for(Some(10), baseline.clone()).unwrap();
        assert!(scan.valid_for(&baseline));
    }

    #[test]
    fn completed_and_pending_reports_expire() {
        let basis =
            super::super::report::EnvironmentBasis::from_config(&AppConfig::default(), 1, 1);
        let mut scan = WebScanState::default();
        scan.record_for(None, basis.clone()).unwrap();
        scan.pending.as_mut().unwrap().1 =
            Instant::now() - REPORT_LIFETIME - Duration::from_secs(1);
        assert!(scan.record_for(Some(10), basis.clone()).is_err());
        scan.record_for(None, basis.clone()).unwrap();
        scan.record_for(Some(10), basis.clone()).unwrap();
        scan.recorded_at = Some(Instant::now() - REPORT_LIFETIME - Duration::from_secs(1));
        assert_eq!(scan.score(), None);
        assert!(!scan.valid_for(&basis));
    }

    #[test]
    fn late_scan_cannot_complete_or_cancel_newer_round() {
        let basis =
            super::super::report::EnvironmentBasis::from_config(&AppConfig::default(), 1, 1);
        let mut scan = WebScanState::default();
        let first = scan.begin_scan(basis.clone());
        let second = scan.begin_scan(basis.clone());
        assert_ne!(first, second);
        assert!(scan.finish_scan(&first, 1, basis.clone()).is_err());
        assert!(scan.record_for(Some(1), basis.clone()).is_err());
        scan.finish_scan(&second, 20, basis.clone()).unwrap();
        assert!(scan.valid_for(&basis));
        assert_eq!(scan.score(), Some(20));
        assert!(scan.finish_scan(&first, 1, basis.clone()).is_err());
        assert_eq!(scan.score(), Some(20));
    }

    #[test]
    fn invalid_scores_revoke_previous_authorization() {
        let cfg = AppConfig::default();
        let native = allowed_native();
        for invalid in [-1, 101] {
            let mut scan = WebScanState::default();
            scan.record(Some(10)).unwrap();
            assert!(scan.record(Some(invalid)).is_err());
            assert!(!gate_passes(&cfg, &native, scan.score()).0);
            assert!(!gate_passes(&cfg, &native, Some(invalid)).0);
        }
    }

    #[test]
    fn score_at_threshold_is_blocked() {
        let cfg = AppConfig::default();
        let native = allowed_native();
        assert!(gate_passes(&cfg, &native, Some(cfg.gate_threshold - 1)).0);
        assert!(!gate_passes(&cfg, &native, Some(cfg.gate_threshold)).0);
    }

    #[test]
    fn connectivity_switch_controls_failed_and_missing_results() {
        let mut cfg = AppConfig::default();
        let mut native = allowed_native();
        for reachable in [None, Some(false)] {
            native.claude_reachable = reachable;
            native.claude_status = Some(403);
            cfg.require_claude_reachable = true;
            assert!(!gate_passes(&cfg, &native, Some(10)).0);
            cfg.require_claude_reachable = false;
            assert!(gate_passes(&cfg, &native, Some(10)).0);
        }
    }

    #[test]
    fn connectivity_does_not_override_proxy_country_or_timezone_checks() {
        let cfg = AppConfig::default();
        let mut native = allowed_native();
        native.country_allowed = Some(false);
        assert!(!gate_passes(&cfg, &native, Some(10)).0);
        native.country_allowed = Some(true);
        native.sysproxy_on = false;
        assert!(!gate_passes(&cfg, &native, Some(10)).0);
        native.sysproxy_on = true;
        native.tz_is_cn = true;
        assert!(!gate_passes(&cfg, &native, Some(10)).0);
    }

    #[tokio::test]
    async fn connectivity_probe_uses_proxy_and_rejects_http_errors() {
        for status in [200, 403, 503] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|v| v == b"\r\n\r\n") {
                    let count = socket.read(&mut buffer).unwrap();
                    assert!(count > 0 && request.len() < 8192);
                    request.extend_from_slice(&buffer[..count]);
                }
                assert!(String::from_utf8_lossy(&request).starts_with("GET http://claude.test/ "));
                write!(
                    socket,
                    "HTTP/1.1 {status} Test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
            });
            let client = core::build_client_with_proxy(port).unwrap();
            let (reachable, actual_status, error) =
                probe_claude(&client, "http://claude.test/").await;
            assert_eq!(reachable, status == 200);
            assert_eq!(actual_status, Some(status));
            assert_eq!(error.is_none(), status == 200);
            server.join().unwrap();
        }
    }

    #[tokio::test]
    async fn proxy_network_failure_is_not_reachable() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let client = core::build_client_with_proxy(port).unwrap();
        let (reachable, status, error) = probe_claude(&client, "http://claude.test/").await;
        assert!(!reachable);
        assert_eq!(status, None);
        assert!(error.is_some());
    }
}
