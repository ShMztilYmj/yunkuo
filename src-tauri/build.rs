fn binary_hash(path: &std::path::Path) -> String {
    use base64::Engine;
    let literal = path.to_string_lossy().replace("'", "''");
    let script = format!(
        "$stream=[IO.File]::OpenRead('{literal}'); $hash=[Security.Cryptography.SHA256]::Create(); try {{ [BitConverter]::ToString($hash.ComputeHash($stream)).Replace('-','') }} finally {{ $stream.Dispose(); $hash.Dispose() }}"
    );
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let output =
        std::process::Command::new(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded])
            .output()
            .expect("无法计算打包组件哈希");
    assert!(
        output.status.success(),
        "打包组件缺失，无法计算哈希：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let hash = String::from_utf8(output.stdout)
        .expect("哈希格式无效")
        .trim()
        .to_string();
    assert!(hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()));
    hash
}

fn main() {
    // 单元测试也可能链接桌面事件/对话框路径，需要启用包含 TaskDialogIndirect 的 v6 控件。
    // 统一由链接器生成清单，避免它与 Tauri 的资源清单双重嵌入。
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
    }
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let path = root
        .join("binaries")
        .join("yunkuo-service-x86_64-pc-windows-msvc.exe");
    println!("cargo:rerun-if-changed={}", path.display());
    println!(
        "cargo:rustc-env=YUNKUO_SERVICE_SHA256={}",
        binary_hash(&path)
    );
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest()),
    )
    .expect("构建 Tauri 资源失败")
}
