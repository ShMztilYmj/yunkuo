//! Explicit live acceptance: config remains local; output contains only results.
use anyhow::{Context, Result, bail};
use mihomo_runtime::{Runtime, StartOptions};
use std::{path::PathBuf, time::Duration};
fn main() -> Result<()> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(run())
}
async fn run() -> Result<()> {
    let source = PathBuf::from(std::env::args_os().nth(1).context("config path required")?);
    let work = PathBuf::from(
        std::env::args_os()
            .nth(2)
            .context("isolated work directory required")?,
    );
    let mut config: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(source)?)?;
    config["mixed-port"] = 27897.into();
    config["external-controller"] = "127.0.0.1:29098".into();
    config["secret"] = "isolated-mihomo-live-test-secret".into();
    config["tun"]["enable"] = false.into();
    config["tun"]["auto-route"] = true.into();
    config["dns"]["listen"] = "127.0.0.1:21057".into();
    config
        .as_mapping_mut()
        .unwrap()
        .remove(serde_yaml::Value::from("external-ui"));
    let yaml = serde_yaml::to_string(&config)?.replace("?proxy=", "#");
    let mut runtime = Runtime::start(StartOptions {
        yaml,
        work_dir: work,
    })
    .await?;
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all("http://127.0.0.1:27897")?)
        .timeout(Duration::from_secs(25))
        .build()?;
    let result=async {
        for target in ["https://example.com/","https://ippure.com/"] {
            let now=std::time::Instant::now();
            let response=client.get(target).send().await.context("live HTTPS proxy request failed")?;
            let status=response.status();
            let bytes=response.bytes().await?.len();
            println!("{}",serde_json::json!({"target":target,"status":status.as_u16(),"bytes":bytes,"elapsedMs":now.elapsed().as_millis(),"engine":mihomo_runtime::ENGINE_ID,"childPid":runtime.snapshot().pid}));
            if !status.is_success() {bail!("live HTTPS request returned {status}");}
        }
        Result::<()>::Ok(())
    }.await;
    runtime.stop().await?;
    result
}
