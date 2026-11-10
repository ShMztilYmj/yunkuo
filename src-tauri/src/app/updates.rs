//! 本地安装包升级/回退：先核验用户提供的 SHA-256，再备份配置并启动安装器。
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{fs, io::Read, path::Path};
use tauri::{AppHandle, Manager};

fn verify(path: &Path, expected: &str) -> Result<String> {
    crate::platform::secure_storage::safe(path)?;
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
        || expected.len() != 64
        || !expected.bytes().all(|b| b.is_ascii_hexdigit())
    {
        bail!("请选择 EXE 安装包并填写完整的 64 位 SHA-256");
    }
    let mut file = fs::File::open(path)?;
    if file.metadata()?.len() > 1024 * 1024 * 1024 {
        bail!("安装包超过 1 GiB 上限");
    }
    let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
    let mut block = [0; 64 * 1024];
    loop {
        let size = file.read(&mut block)?;
        if size == 0 {
            break;
        }
        digest.update(&block[..size]);
    }
    // 大文件流式读取后仍用统一的大写十六进制转换。
    let hash = crate::platform::digest::hex_upper(digest.finish().as_ref());
    if !hash.eq_ignore_ascii_case(expected.trim()) {
        bail!("安装包 SHA-256 不匹配，未启动安装器");
    }
    Ok(hash)
}

#[tauri::command]
pub async fn inspect_installer(path: String, sha256: String) -> Result<Value, String> {
    tokio::task::spawn_blocking(move||->Result<Value>{
        let file=Path::new(&path);let hash=verify(file,&sha256)?;
        let details=crate::platform::files::shell("$ErrorActionPreference='Stop'; $f=Get-Item -LiteralPath $env:COC_INSTALLER; $s=Get-AuthenticodeSignature -LiteralPath $env:COC_INSTALLER; @{version=$f.VersionInfo.FileVersion;signature=$s.Status.ToString();bytes=$f.Length} | ConvertTo-Json -Compress",&[("COC_INSTALLER",&path)])?;
        let details:Value=serde_json::from_str(&details)?;
        Ok(json!({"sha256":hash,"fileName":file.file_name().unwrap_or_default().to_string_lossy(),"details":details}))
    }).await.map_err(|_|"核验安装包未完成".to_string())?.map_err(|e|format!("{e:#}"))
}

#[tauri::command]
pub async fn launch_verified_installer(
    app: AppHandle,
    path: String,
    sha256: String,
) -> Result<(), String> {
    let environment = app.state::<crate::platform::state::EnvironmentOperation>();
    let _environment = environment.lock().await;
    let operation = app.state::<crate::platform::state::ConfigUpdateMutex>();
    let _operation = operation.lock().await;
    crate::fingerprint_browser::browser::ensure_closed(&app).map_err(|e| format!("{e:#}"))?;
    let privacy = crate::proxy::core::privacy_status()
        .await
        .map_err(|_| "无法确认保护已解除，请先修复安全服务".to_string())?;
    if privacy["armed"].as_bool() != Some(false) {
        return Err("请先关闭 AI 客户端并解除严格保护，再升级或回退".into());
    }
    let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
    crate::app::maintenance::require_ready(&root).map_err(|e| format!("{e:#}"))?;
    // 核验后复制到应用管理的随机文件，启动前再次核验，避免误启动另一个同名包。
    let source = path.clone();
    let expected = sha256.clone();
    let stage = root
        .join("updates")
        .join(format!("{}.exe", uuid::Uuid::new_v4()));
    let target = stage.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        verify(Path::new(&source), &expected)?;
        crate::platform::secure_storage::safe(&target)?;
        fs::create_dir_all(target.parent().context("安装包目录无效")?)?;
        fs::copy(source, &target)?;
        verify(&target, &expected)?;
        Ok(())
    })
    .await
    .map_err(|_| "准备安装包未完成".to_string())?
    .map_err(|e| format!("{e:#}"))?;
    let cfg = app.state::<crate::config::ConfigMutex>().lock().clone();
    let save_root = root.clone();
    tokio::task::spawn_blocking(move || crate::app::maintenance::before_upgrade(&save_root, cfg))
        .await
        .map_err(|_| "升级前备份未完成".to_string())?
        .map_err(|e| format!("备份失败，未启动安装器：{e:#}"))?;
    crate::app::maintenance::pause_for_maintenance(&app).await?;
    crate::ai::launcher::launch_shortcut(&stage.to_string_lossy())
        .map_err(|e| format!("启动安装器失败：{e:#}"))?;
    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mismatched_package_is_never_accepted() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("fixture.exe");
        fs::write(&path, b"fixture").unwrap();
        assert!(verify(&path, &"0".repeat(64)).is_err());
        let expected = ring::digest::digest(&ring::digest::SHA256, b"fixture")
            .as_ref()
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<String>();
        assert_eq!(verify(&path, &expected).unwrap(), expected);
    }
    #[tokio::test]
    async fn inspector_reads_fixture_metadata_without_launching_it() {
        let fixture = crate::test_support::FixtureDir::new();
        let path = fixture.path().join("fixture.exe");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("binaries/yunkuo-service-x86_64-pc-windows-msvc.exe"),
            &path,
        )
        .unwrap();
        let expected = ring::digest::digest(&ring::digest::SHA256, &fs::read(&path).unwrap())
            .as_ref()
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<String>();
        let result = inspect_installer(path.to_string_lossy().into(), expected)
            .await
            .unwrap();
        assert_eq!(result["details"]["signature"], "NotSigned");
    }
}
