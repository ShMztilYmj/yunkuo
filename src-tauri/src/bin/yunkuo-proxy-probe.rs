//! Opt-in release measurement host. Only loopback sockets; never runs desktop setup.
use proxy_runtime::{Runtime, StartOptions};
use tokio::io::{AsyncBufReadExt, BufReader};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let directory = std::env::args_os().nth(1).ok_or_else(|| {
        anyhow::anyhow!("Expected an isolated work directory as the first argument")
    })?;
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory)?;
    let yaml = "listeners:\n- name: release-benchmark\n  type: mixed\n  listen: 127.0.0.1\n  port: 0\nexternal-controller: 127.0.0.1:0\nsecret: isolated-benchmark-secret-32-bytes\ndns: {enable: false}\nrules: ['MATCH,DIRECT']\n";
    let start = std::time::Instant::now();
    let mut runtime = Runtime::start(StartOptions {
        yaml: yaml.into(),
        work_dir: directory,
    })
    .await?;
    let startup_ms = start.elapsed().as_secs_f64() * 1000.;
    let address = runtime.snapshot().controller.unwrap().to_string();
    // Warm-up and memory-control samples execute the production local_request seam.
    let mut samples = Vec::with_capacity(500);
    for index in 0..530 {
        let now = std::time::Instant::now();
        proxy_runtime::local_request(
            &address,
            "isolated-benchmark-secret-32-bytes",
            "GET",
            "/version",
            None,
        )
        .await
        .ok_or_else(|| anyhow::anyhow!("Local controller was not registered"))??;
        if index >= 30 {
            samples.push(now.elapsed().as_secs_f64() * 1000.);
        }
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "{}",
        serde_json::json!({
            "pid": std::process::id(), "startupMs": startup_ms,
            "profile": "release; opt-level=s; lto=true; codegen-units=1; panic=abort; strip=true",
            "mode": "loopback DIRECT; DNS disabled; no TUN or OS network changes",
            "memoryControlP50Ms": samples[249], "memoryControlP95Ms": samples[474],
            "runtime": runtime.snapshot()
        })
    );
    let mut line = String::new();
    BufReader::new(tokio::io::stdin())
        .read_line(&mut line)
        .await?;
    let start = std::time::Instant::now();
    let stopped = runtime.stop().await?;
    println!(
        "{}",
        serde_json::json!({"stopMs":start.elapsed().as_secs_f64()*1000.,"runtime":stopped})
    );
    Ok(())
}
