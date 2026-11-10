//! HTTP 客户端工厂。
//!
//! 只覆盖配置完全相同的几种情况，避免每个模块各写一份超时与代理组合。网关流式请求、
//! 回环地址、`https_only` 与自定义 UA 等特殊配置仍由各自模块构造。

use anyhow::Result;
use std::time::Duration;

fn loopback(port: u16) -> Result<reqwest::Proxy> {
    Ok(reqwest::Proxy::all(format!("http://127.0.0.1:{port}"))?)
}

/// 直连：不使用系统代理或环境变量里的代理。
pub fn direct(timeout: Duration) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .no_proxy()
        .timeout(timeout)
        .build()?)
}

/// 直连的阻塞版本，服务组件里的短超时探测使用。
#[cfg_attr(not(test), allow(dead_code))]
pub fn direct_blocking(timeout: Duration) -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(timeout)
        .build()?)
}

/// 经本机混合端口代理。
pub fn via_loopback(port: u16, timeout: Duration) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(timeout)
        .proxy(loopback(port)?)
        .build()?)
}

/// 有代理端口就走本机代理，没有就直连。
pub fn loopback_or_direct(port: Option<u16>, timeout: Duration) -> Result<reqwest::Client> {
    match port {
        Some(port) => via_loopback(port, timeout),
        None => direct(timeout),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configurations_build_without_touching_the_network() {
        assert!(direct(Duration::from_secs(1)).is_ok());
        assert!(direct_blocking(Duration::from_secs(1)).is_ok());
        assert!(via_loopback(7897, Duration::from_secs(1)).is_ok());
        assert!(loopback_or_direct(Some(7897), Duration::from_secs(1)).is_ok());
        assert!(loopback_or_direct(None, Duration::from_secs(1)).is_ok());
    }
}
