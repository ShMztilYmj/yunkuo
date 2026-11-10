//! 主程序与服务共享的文件存储机制；不包含应用业务配置。
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

/// 简单文件存储封装
pub struct FileStore {
    path: PathBuf,
}

impl FileStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load<T: for<'de> Deserialize<'de> + Default>(&self) -> Result<T> {
        match fs::read(&self.path) {
            Ok(bytes) => match serde_yaml::from_slice(&bytes) {
                Ok(value) => Ok(value),
                Err(_) => {
                    let backup = self.path.with_extension("bak");
                    if let Ok(bytes) = fs::read(&backup) {
                        if let Ok(value) = serde_yaml::from_slice(&bytes) {
                            let preserved = self
                                .path
                                .with_extension(format!("corrupt-{}", uuid::Uuid::new_v4()));
                            fs::copy(&self.path, &preserved)?;
                            atomic_write(&self.path, &bytes)?;
                            return Ok(value);
                        }
                    }
                    Err(anyhow!(
                        "配置文件格式损坏，原文件已保留，请恢复备份：{}",
                        self.path.display()
                    ))
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let backup = self.path.with_extension("bak");
                match fs::read(&backup) {
                    Ok(bytes) => {
                        let value = serde_yaml::from_slice(&bytes)
                            .with_context(|| format!("备份配置损坏：{}", backup.display()))?;
                        atomic_write(&self.path, &bytes)?;
                        Ok(value)
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
                    Err(error) => Err(error).context("无法读取备份配置"),
                }
            }
            Err(error) => {
                Err(error).with_context(|| format!("无法读取配置文件：{}", self.path.display()))
            }
        }
    }

    pub fn save<T: Serialize + for<'de> Deserialize<'de>>(&self, value: &T) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = serde_yaml::to_string(value)?;
        if let Ok(previous) = fs::read(&self.path) {
            if serde_yaml::from_slice::<T>(&previous).is_ok() {
                atomic_write(&self.path.with_extension("bak"), &previous)?;
            }
        }
        atomic_write(&self.path, text.as_bytes())
    }
}

/// 同目录写入并 flush 后原子替换；失败时保留原文件。
pub fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    use windows::core::PCWSTR;
    let parent = path.parent().ok_or_else(|| anyhow!("存储路径缺少父目录"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        let from: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            MoveFileExW(
                PCWSTR(from.as_ptr()),
                PCWSTR(to.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
