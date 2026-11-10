use anyhow::Result;
use mihomo_runtime::{Runtime, StartOptions};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};
fn main() -> Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(run())
}
async fn run() -> Result<()> {
    let source = PathBuf::from(std::env::args_os().nth(1).unwrap());
    let directory = PathBuf::from(std::env::args_os().nth(2).unwrap());
    let mut value: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(source)?)?;
    value["mixed-port"] = 17890.into();
    value["external-controller"] = "127.0.0.1:19097".into();
    value["secret"] = "isolated-installed-startup-test-secret".into();
    value["tun"]["enable"] = false.into();
    value["dns"]["listen"] = "127.0.0.1:11053".into();
    value["interface-name"] = "WLAN".into();
    let mut runtime = Runtime::start(StartOptions {
        yaml: serde_yaml::to_string(&value)?,
        work_dir: directory,
    })
    .await?;
    let direct = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()?;
    let nodes: Value = direct
        .get("http://127.0.0.1:19097/proxies")
        .bearer_auth("isolated-installed-startup-test-secret")
        .send()
        .await?
        .json()
        .await?;
    for group in ["GLOBAL", "节点选择", "ChatGPT"] {
        println!(
            "{}",
            json!({"group":group,"selected":nodes["proxies"][group]["now"],"kind":nodes["proxies"][group]["type"]})
        );
    }
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all("http://127.0.0.1:17890")?)
        .timeout(Duration::from_secs(8))
        .build()?;
    for url in [
        "https://example.com/",
        "https://ippure.com/",
        "https://www.baidu.com/",
    ] {
        let result = client.get(url).send().await;
        match result {
            Ok(r) => println!("{}", json!({"target":url,"status":r.status().as_u16()})),
            Err(e) => println!(
                "{}",
                json!({"target":url,"status":0,"timeout":e.is_timeout(),"connect":e.is_connect()})
            ),
        }
    }
    runtime.stop().await?;
    Ok(())
}
