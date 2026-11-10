//! 只读 Windows 系统时间机制，不拥有时区修复策略与备份。
pub(crate) fn timezone_key() -> anyhow::Result<String> {
    use windows::Win32::System::Time::{
        DYNAMIC_TIME_ZONE_INFORMATION, GetDynamicTimeZoneInformation,
    };
    let mut information = DYNAMIC_TIME_ZONE_INFORMATION::default();
    let status = unsafe { GetDynamicTimeZoneInformation(&mut information) };
    if status == u32::MAX {
        anyhow::bail!("无法读取系统时区");
    }
    let end = information
        .TimeZoneKeyName
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(information.TimeZoneKeyName.len());
    Ok(String::from_utf16_lossy(
        &information.TimeZoneKeyName[..end],
    ))
}
