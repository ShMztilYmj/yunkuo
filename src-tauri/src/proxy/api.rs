//! 产品代理接口：控制器结构和路径在这里转换，调用方只看到产品视图。
use crate::config::AppConfig;
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Lifecycle {
    Stopped,
    Starting,
    Ready,
    Reconfiguring,
    Stopping,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LatencySample {
    pub time: String,
    pub delay: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Node {
    pub id: String,
    pub protocol: String,
    pub udp: Option<bool>,
    pub samples: Vec<LatencySample>,
    pub selected: Option<String>,
    pub choices: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Rule {
    pub kind: String,
    pub pattern: String,
    pub target: String,
    pub size: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Connection {
    pub id: String,
    pub network: String,
    pub inbound: String,
    pub source_ip: String,
    pub destination_ip: String,
    pub source_port: String,
    pub destination_port: String,
    pub host: String,
    pub process_path: Option<String>,
    pub process: Option<String>,
    pub uploaded: u64,
    pub downloaded: u64,
    pub started_at: String,
    pub chain: Vec<String>,
    pub rule: String,
    pub rule_pattern: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Connections {
    pub downloaded: u64,
    pub uploaded: u64,
    pub items: Vec<Connection>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Capabilities {
    pub engine: &'static str,
    pub hosting: &'static str,
    pub tcp_dns_listener: bool,
    pub embedded: bool,
    pub tun_available: bool,
    pub full_reload_preserves_connections: bool,
}

pub(crate) fn capabilities(_embedded: bool) -> Capabilities {
    Capabilities {
        engine: proxy_runtime::ENGINE_ID,
        hosting: "managed-process",
        tcp_dns_listener: true,
        embedded: false,
        tun_available: cfg!(windows),
        full_reload_preserves_connections: false,
    }
}

fn text(value: &Value, key: &str) -> String {
    match &value[key] {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        _ => String::new(),
    }
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn inventory(value: Value) -> Result<BTreeMap<String, Node>> {
    let nodes = value["proxies"]
        .as_object()
        .ok_or_else(|| anyhow!("代理列表响应缺少节点集合"))?;
    nodes
        .iter()
        .map(|(id, node)| {
            let protocol = node["type"]
                .as_str()
                .ok_or_else(|| anyhow!("代理节点缺少协议类型"))?;
            let samples = node["history"]
                .as_array()
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|sample| serde_json::from_value(sample.clone()).ok())
                        .collect()
                })
                .unwrap_or_default();
            Ok((
                id.clone(),
                Node {
                    id: id.clone(),
                    protocol: protocol.to_owned(),
                    udp: node["udp"].as_bool(),
                    samples,
                    selected: node["now"].as_str().map(str::to_owned),
                    choices: strings(&node["all"]),
                },
            ))
        })
        .collect()
}

pub(crate) fn endpoint(segments: &[&str], query: &[(&str, String)]) -> String {
    let mut url = reqwest::Url::parse("http://127.0.0.1/").expect("固定本机 URL 有效");
    url.path_segments_mut()
        .expect("HTTP URL 支持路径")
        .clear()
        .extend(segments);
    if !query.is_empty() {
        url.query_pairs_mut()
            .extend_pairs(query.iter().map(|(name, value)| (*name, value.as_str())));
    }
    let mut path = url.path().to_owned();
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    path
}

pub(crate) async fn nodes(cfg: &AppConfig) -> Result<BTreeMap<String, Node>> {
    inventory(super::core::controller_request(cfg, "GET", "/proxies", None).await?)
}

pub(crate) async fn rules(cfg: &AppConfig) -> Result<Vec<Rule>> {
    let value = super::core::controller_request(cfg, "GET", "/rules", None).await?;
    value["rules"]
        .as_array()
        .ok_or_else(|| anyhow!("路由响应缺少规则集合"))?
        .iter()
        .map(|rule| {
            Ok(Rule {
                kind: text(rule, "type"),
                pattern: text(rule, "payload"),
                target: text(rule, "proxy"),
                size: rule["size"].as_u64().unwrap_or(0),
            })
        })
        .collect()
}

pub(crate) async fn connections(cfg: &AppConfig) -> Result<Connections> {
    let value = super::core::controller_request(cfg, "GET", "/connections", None).await?;
    let downloaded = value["downloadTotal"]
        .as_u64()
        .ok_or_else(|| anyhow!("连接响应缺少下载累计值"))?;
    let uploaded = value["uploadTotal"]
        .as_u64()
        .ok_or_else(|| anyhow!("连接响应缺少上传累计值"))?;
    let items = if value["connections"].is_null() {
        Vec::new()
    } else {
        value["connections"]
            .as_array()
            .ok_or_else(|| anyhow!("连接集合格式错误"))?
            .iter()
            .map(|item| {
                let metadata = &item["metadata"];
                Connection {
                    id: text(item, "id"),
                    network: text(metadata, "network"),
                    inbound: text(metadata, "type"),
                    source_ip: text(metadata, "sourceIP"),
                    destination_ip: text(metadata, "destinationIP"),
                    source_port: text(metadata, "sourcePort"),
                    destination_port: text(metadata, "destinationPort"),
                    host: text(metadata, "host"),
                    process_path: metadata["processPath"].as_str().map(str::to_owned),
                    process: metadata["process"].as_str().map(str::to_owned),
                    uploaded: item["upload"].as_u64().unwrap_or(0),
                    downloaded: item["download"].as_u64().unwrap_or(0),
                    started_at: text(item, "start"),
                    chain: strings(&item["chains"]),
                    rule: text(item, "rule"),
                    rule_pattern: text(item, "rulePayload"),
                }
            })
            .collect()
    };
    Ok(Connections {
        downloaded,
        uploaded,
        items,
    })
}

pub(crate) async fn select(cfg: &AppConfig, group: &str, node: &str) -> Result<()> {
    super::core::controller_request(
        cfg,
        "PUT",
        &endpoint(&["proxies", group], &[]),
        Some(serde_json::json!({"name":node})),
    )
    .await?;
    Ok(())
}

pub(crate) async fn close(cfg: &AppConfig, id: Option<&str>) -> Result<()> {
    let path = id
        .map(|id| endpoint(&["connections", id], &[]))
        .unwrap_or_else(|| "/connections".into());
    super::core::controller_request(cfg, "DELETE", &path, None).await?;
    Ok(())
}

pub(crate) async fn delay(
    cfg: &AppConfig,
    node: &str,
    group: bool,
    timeout: u64,
    url: &str,
) -> Result<Value> {
    if !(100..=60_000).contains(&timeout) {
        return Err(anyhow!("测速超时应在 100—60000 毫秒之间"));
    }
    let parsed = reqwest::Url::parse(url)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(anyhow!("测速地址必须为 HTTP(S)"));
    }
    super::core::controller_request(
        cfg,
        "GET",
        &endpoint(
            &[if group { "group" } else { "proxies" }, node, "delay"],
            &[("url", url.into()), ("timeout", timeout.to_string())],
        ),
        None,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controller_fields_are_translated_at_the_interface() {
        let value = serde_json::json!({"proxies":{"组/一":{"name":"untrusted display","type":"Selector","udp":false,"now":"节点 A","all":["节点 A"],"history":[{"time":"2026-10-23","delay":42}]}}});
        let node = &inventory(value).unwrap()["组/一"];
        assert_eq!(node.id, "组/一");
        assert_eq!(node.selected.as_deref(), Some("节点 A"));
        assert!(!node.udp.unwrap());
        assert_eq!(node.samples[0].delay, 42);
        assert!(inventory(serde_json::json!({"error":"unavailable"})).is_err());
    }
    #[test]
    fn node_names_remain_single_encoded_path_segments() {
        let path = endpoint(&["proxies", "a/b?x=# 空"], &[]);
        assert!(!path.contains("?x="));
        assert!(path.contains("a%2Fb%3Fx="));
        assert!(path.contains("%23"));
    }
}
