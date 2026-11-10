use crate::config::{FileStore, TzBackup, TzSyncInfo};
use anyhow::{Result, anyhow, bail};
use serde::Deserialize;
use std::path::PathBuf;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_NOT_ALL_ASSIGNED, ERROR_SUCCESS, GetLastError, HANDLE, SYSTEMTIME,
};
use windows::Win32::Security::{
    AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES,
    TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_READ, REG_VALUE_TYPE, RegCloseKey, RegEnumKeyExW, RegOpenKeyExW,
    RegQueryValueExW,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::System::Time::{
    DYNAMIC_TIME_ZONE_INFORMATION, GetDynamicTimeZoneInformation, SetDynamicTimeZoneInformation,
};
use windows::core::PCWSTR;

const TZ_ROOT: &str = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Time Zones";

/// 单个 Windows 时区定义（来自注册表）
#[derive(Debug, Clone, serde::Serialize)]
pub struct TzEntry {
    pub key_name: String,
    pub std_name: String,
    pub dlt_name: String,
    pub display: String,
    pub bias: i32,
    pub std_bias: i32,
    pub dlt_bias: i32,
    #[serde(skip)]
    pub std_date: SYSTEMTIME,
    #[serde(skip)]
    pub dlt_date: SYSTEMTIME,
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// 启用 SeTimeZonePrivilege（管理员令牌默认禁用，需要显式开启）
fn enable_timezone_privilege() -> Result<()> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
        .map_err(|e| anyhow!("OpenProcessToken 失败: {e}"))?;

        let name = to_wide("SeTimeZonePrivilege");
        let mut luid = windows::Win32::Foundation::LUID::default();
        windows::Win32::Security::LookupPrivilegeValueW(
            PCWSTR::null(),
            PCWSTR(name.as_ptr()),
            &mut luid,
        )
        .map_err(|e| {
            let _ = CloseHandle(token);
            anyhow!("LookupPrivilegeValue 失败: {e}")
        })?;

        let tp = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        AdjustTokenPrivileges(token, false, Some(&tp), 0, None, None)
            .map_err(|e| anyhow!("AdjustTokenPrivileges 失败: {e}"))?;
        if GetLastError() == ERROR_NOT_ALL_ASSIGNED {
            let _ = CloseHandle(token);
            bail!("当前账户没有修改时区的权限，请以管理员身份运行本软件");
        }
        let _ = CloseHandle(token);
    }
    Ok(())
}

fn read_reg_string(hkey: HKEY, name: &str) -> Result<String> {
    let wname = to_wide(name);
    let mut buf = [0u8; 1024];
    let mut size = buf.len() as u32;
    let mut rtype = REG_VALUE_TYPE::default();
    let ret = unsafe {
        RegQueryValueExW(
            hkey,
            PCWSTR(wname.as_ptr()),
            None,
            Some(&mut rtype),
            Some(buf.as_mut_ptr()),
            Some(&mut size),
        )
    };
    if ret != ERROR_SUCCESS {
        bail!("读注册表值 {name} 失败 (code {})", ret.0);
    }
    // REG_SZ / REG_EXPAND_SZ 均按 UTF-16 处理
    let u16s: Vec<u16> = buf[..size as usize]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    Ok(from_wide(&u16s))
}

fn read_reg_binary(hkey: HKEY, name: &str) -> Result<Vec<u8>> {
    let wname = to_wide(name);
    let mut buf = [0u8; 512];
    let mut size = buf.len() as u32;
    let ret = unsafe {
        RegQueryValueExW(
            hkey,
            PCWSTR(wname.as_ptr()),
            None,
            None,
            Some(buf.as_mut_ptr()),
            Some(&mut size),
        )
    };
    if ret != ERROR_SUCCESS {
        bail!("读注册表值 {name} 失败 (code {})", ret.0);
    }
    Ok(buf[..size as usize].to_vec())
}

fn parse_systemtime(bytes: &[u8]) -> SYSTEMTIME {
    let vals: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    SYSTEMTIME {
        wYear: *vals.first().unwrap_or(&0),
        wMonth: *vals.get(1).unwrap_or(&0),
        wDayOfWeek: *vals.get(2).unwrap_or(&0),
        wDay: *vals.get(3).unwrap_or(&0),
        wHour: *vals.get(4).unwrap_or(&0),
        wMinute: *vals.get(5).unwrap_or(&0),
        wSecond: *vals.get(6).unwrap_or(&0),
        wMilliseconds: *vals.get(7).unwrap_or(&0),
    }
}

fn i32le(bytes: &[u8]) -> i32 {
    i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// 枚举全部 Windows 时区
pub fn list_timezones() -> Result<Vec<TzEntry>> {
    let root = to_wide(TZ_ROOT);
    let mut hkey = HKEY::default();
    let ret = unsafe {
        RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(root.as_ptr()),
            None,
            KEY_READ,
            &mut hkey,
        )
    };
    if ret != ERROR_SUCCESS {
        bail!("打开时区注册表失败 (code {})", ret.0);
    }

    let mut entries = Vec::new();
    let mut index = 0u32;
    loop {
        let mut name_buf = [0u16; 256];
        let mut name_len = name_buf.len() as u32;
        let ret = unsafe {
            RegEnumKeyExW(
                hkey,
                index,
                Some(windows::core::PWSTR(name_buf.as_mut_ptr())),
                &mut name_len,
                None,
                None,
                None,
                None,
            )
        };
        if ret != ERROR_SUCCESS {
            break;
        }
        index += 1;
        let sub_name = from_wide(&name_buf);
        if sub_name.is_empty() {
            continue;
        }
        let sub_path = to_wide(&format!("{TZ_ROOT}\\{sub_name}"));
        let mut sub = HKEY::default();
        if unsafe {
            RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(sub_path.as_ptr()),
                None,
                KEY_READ,
                &mut sub,
            )
        }
        .is_err()
        {
            continue;
        }
        let entry = (|| -> Result<TzEntry> {
            let std_name = read_reg_string(sub, "Std").unwrap_or_default();
            let dlt_name = read_reg_string(sub, "Dlt").unwrap_or_default();
            let display = read_reg_string(sub, "").unwrap_or_default();
            let tzi = read_reg_binary(sub, "TZI").unwrap_or_default();
            let (bias, std_bias, dlt_bias, std_date, dlt_date) = if tzi.len() >= 44 {
                (
                    i32le(&tzi[0..4]),
                    i32le(&tzi[4..8]),
                    i32le(&tzi[8..12]),
                    parse_systemtime(&tzi[12..28]),
                    parse_systemtime(&tzi[28..44]),
                )
            } else {
                (0, 0, 0, SYSTEMTIME::default(), SYSTEMTIME::default())
            };
            Ok(TzEntry {
                key_name: sub_name,
                std_name,
                dlt_name,
                display,
                bias,
                std_bias,
                dlt_bias,
                std_date,
                dlt_date,
            })
        })();
        if let Ok(e) = entry {
            entries.push(e);
        }
        unsafe {
            let _ = RegCloseKey(sub);
        }
    }
    unsafe {
        let _ = RegCloseKey(hkey);
    }
    Ok(entries)
}

fn find_by_key(key: &str) -> Option<TzEntry> {
    list_timezones()
        .ok()?
        .into_iter()
        .find(|e| e.key_name.eq_ignore_ascii_case(key))
}

fn find_by_std(std_name: &str) -> Option<TzEntry> {
    list_timezones()
        .ok()?
        .into_iter()
        .find(|e| e.std_name == std_name)
}

/// 写入 Windows 系统时区
pub fn set_timezone(entry: &TzEntry) -> Result<()> {
    enable_timezone_privilege()?;
    let dtzi = timezone_information(entry)?;
    unsafe { SetDynamicTimeZoneInformation(&dtzi) }.map_err(|e| {
        anyhow!(
            "SetDynamicTimeZoneInformation 失败（错误码 {}）: {e}",
            unsafe { GetLastError().0 }
        )
    })?;
    if get_current_tz()?.key_name != entry.key_name {
        bail!("系统时区写入后的读回结果不一致，请使用备份恢复并重新检查");
    }
    Ok(())
}

fn timezone_information(entry: &TzEntry) -> Result<DYNAMIC_TIME_ZONE_INFORMATION> {
    let mut dtzi = DYNAMIC_TIME_ZONE_INFORMATION::default();
    dtzi.Bias = entry.bias;
    let key: Vec<u16> = entry.key_name.encode_utf16().collect();
    if key.len() >= dtzi.TimeZoneKeyName.len() {
        bail!("时区键名过长");
    }
    dtzi.TimeZoneKeyName[..key.len()].copy_from_slice(&key);
    fill_name(&mut dtzi.StandardName, &entry.std_name);
    dtzi.StandardDate = entry.std_date;
    dtzi.StandardBias = entry.std_bias;
    fill_name(&mut dtzi.DaylightName, &entry.dlt_name);
    dtzi.DaylightDate = entry.dlt_date;
    dtzi.DaylightBias = entry.dlt_bias;
    dtzi.DynamicDaylightTimeDisabled = false;
    Ok(dtzi)
}

fn fill_name(dst: &mut [u16; 32], s: &str) {
    let w: Vec<u16> = s.encode_utf16().take(31).collect();
    dst[..w.len()].copy_from_slice(&w);
    for c in &mut dst[w.len()..] {
        *c = 0;
    }
}

/// 当前时区信息
#[derive(Debug, Clone, serde::Serialize)]
pub struct CurrentTz {
    pub key_name: String,
    pub display: String,
    pub iana_guess: String,
    pub bias_minutes: i32,
}

pub fn get_current_tz() -> Result<CurrentTz> {
    let mut dtzi = DYNAMIC_TIME_ZONE_INFORMATION::default();
    unsafe {
        GetDynamicTimeZoneInformation(&mut dtzi);
    }
    let key = from_wide(&dtzi.TimeZoneKeyName);
    let display = find_by_key(&key)
        .map(|e| e.display)
        .unwrap_or_else(|| key.clone());
    let iana = windows_to_iana(&key).unwrap_or("unknown");
    Ok(CurrentTz {
        key_name: key,
        display,
        iana_guess: iana.into(),
        bias_minutes: dtzi.Bias,
    })
}

/// 首次同步前备份当前时区
pub fn backup_current(data_dir: &PathBuf) -> Result<TzBackup> {
    let store = FileStore::new(data_dir.join("tz-backup.json"));
    let b: TzBackup = store.load()?;
    let existing = Some(b).filter(|b| !b.key_name.is_empty());
    if let Some(b) = existing {
        return Ok(b);
    }
    let cur = get_current_tz()?;
    let backup = TzBackup {
        key_name: cur.key_name.clone(),
        display: cur.display.clone(),
        backed_up_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    };
    store.save(&backup)?;
    Ok(backup)
}

/// 还原为备份的原始时区
pub fn restore_original(data_dir: &PathBuf) -> Result<TzBackup> {
    let store = FileStore::new(data_dir.join("tz-backup.json"));
    let backup: TzBackup = store.load()?;
    if backup.key_name.is_empty() {
        bail!("没有可还原的时区备份");
    }
    let entry = find_by_key(&backup.key_name)
        .ok_or_else(|| anyhow!("找不到原始时区: {}", backup.key_name))?;
    set_timezone(&entry)?;
    Ok(backup)
}

pub fn get_backup(data_dir: &PathBuf) -> Option<TzBackup> {
    let store = FileStore::new(data_dir.join("tz-backup.json"));
    let backup: TzBackup = store.load().ok()?;
    Some(backup).filter(|b| !b.key_name.is_empty())
}

/// 手动按 Windows 时区名设置
pub fn set_timezone_by_name(name: &str) -> Result<()> {
    let entry = find_by_key(name)
        .or_else(|| find_by_std(name))
        .ok_or_else(|| anyhow!("找不到时区: {name}"))?;
    set_timezone(&entry)
}

// ---------- IP 定位 ----------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IpGeo {
    pub ip: String,
    pub country_code: String,
    pub country_name: String,
    pub timezone: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SyncResult {
    pub changed: bool,
    pub message: String,
    pub info: Option<TzSyncInfo>,
}

/// 通过（可选）代理查询出口 IP 归属地
pub async fn fetch_ip_geo(proxy_port: Option<u16>) -> Result<IpGeo> {
    let client = crate::http::loopback_or_direct(proxy_port, std::time::Duration::from_secs(10))?;

    // 1) ipwho.is：比 ipapi.co 更稳定，且与前端出口三角检测使用同一份时区口径。
    if let Ok(v) = client.get("https://ipwho.is/").send().await {
        if let Ok(json) = v.json::<serde_json::Value>().await {
            let ip = json.get("ip").and_then(|x| x.as_str());
            let cc = json.get("country_code").and_then(|x| x.as_str());
            let tz = json
                .get("timezone")
                .and_then(|x| x.get("id"))
                .and_then(|x| x.as_str());
            if let (Some(ip), Some(cc), Some(tz)) = (ip, cc, tz) {
                if !tz.is_empty() {
                    return Ok(IpGeo {
                        ip: ip.into(),
                        country_code: cc.into(),
                        country_name: json
                            .get("country")
                            .and_then(|x| x.as_str())
                            .unwrap_or_default()
                            .into(),
                        timezone: tz.into(),
                    });
                }
            }
        }
    }
    // 2) ipinfo.io（ipwho 不可达时兜底；免费接口仍带 timezone）
    if let Ok(v) = client.get("https://ipinfo.io/json").send().await {
        if let Ok(json) = v.json::<serde_json::Value>().await {
            let ip = json.get("ip").and_then(|x| x.as_str());
            let cc = json.get("country").and_then(|x| x.as_str());
            let tz = json.get("timezone").and_then(|x| x.as_str());
            if let (Some(ip), Some(cc), Some(tz)) = (ip, cc, tz) {
                if !tz.is_empty() {
                    return Ok(IpGeo {
                        ip: ip.into(),
                        country_code: cc.into(),
                        country_name: cc.into(),
                        timezone: tz.into(),
                    });
                }
            }
        }
    }
    // 3) ipapi.co
    if let Ok(v) = client.get("https://ipapi.co/json/").send().await {
        if let Ok(json) = v.json::<serde_json::Value>().await {
            if let (Some(ip), Some(cc)) = (
                json.get("ip").and_then(|x| x.as_str()),
                json.get("country_code").and_then(|x| x.as_str()),
            ) {
                return Ok(IpGeo {
                    ip: ip.into(),
                    country_code: cc.into(),
                    country_name: json
                        .get("country_name")
                        .and_then(|x| x.as_str())
                        .unwrap_or_default()
                        .into(),
                    timezone: json
                        .get("timezone")
                        .and_then(|x| x.as_str())
                        .unwrap_or_default()
                        .into(),
                });
            }
        }
    }
    // 2) api.ip.sb
    if let Ok(v) = client
        .get("https://api.ip.sb/geoip")
        .header("User-Agent", "curl/8.0")
        .send()
        .await
    {
        if let Ok(json) = v.json::<serde_json::Value>().await {
            if let (Some(ip), Some(cc)) = (
                json.get("ip").and_then(|x| x.as_str()),
                json.get("country_code").and_then(|x| x.as_str()),
            ) {
                return Ok(IpGeo {
                    ip: ip.into(),
                    country_code: cc.into(),
                    country_name: json
                        .get("country")
                        .and_then(|x| x.as_str())
                        .unwrap_or_default()
                        .into(),
                    timezone: json
                        .get("timezone")
                        .and_then(|x| x.as_str())
                        .unwrap_or_default()
                        .into(),
                });
            }
        }
    }
    // 3) ip-api.com（免费版仅 HTTP）
    #[derive(Deserialize)]
    struct IpApi {
        #[serde(rename = "query")]
        ip: String,
        #[serde(rename = "country")]
        country: String,
        #[serde(rename = "countryCode")]
        cc: String,
        timezone: String,
    }
    if let Ok(v) = client
        .get("http://ip-api.com/json/?fields=query,country,countryCode,timezone")
        .send()
        .await
    {
        if let Ok(data) = v.json::<IpApi>().await {
            return Ok(IpGeo {
                ip: data.ip,
                country_code: data.cc,
                country_name: data.country,
                timezone: data.timezone,
            });
        }
    }
    bail!("无法获取出口 IP 归属地，请检查网络/代理")
}

/// 核心：检测 IP 位置并据此设置系统时区
pub async fn sync_timezone_by_ip(
    data_dir: &PathBuf,
    proxy_port: Option<u16>,
) -> Result<SyncResult> {
    let geo = fetch_ip_geo(proxy_port).await?;
    if geo.timezone.is_empty() {
        bail!("IP 归属地未返回时区信息");
    }
    let windows_tz = iana_to_windows(&geo.timezone)
        .ok_or_else(|| anyhow!("无法映射时区 {} 到 Windows 时区", geo.timezone))?;
    let entry =
        find_by_key(&windows_tz).ok_or_else(|| anyhow!("系统中不存在时区: {windows_tz}"))?;

    let current = get_current_tz()?;
    if current.key_name == entry.key_name {
        return Ok(SyncResult {
            changed: false,
            message: format!(
                "系统时区已与出口位置一致（{} / {}）",
                geo.country_name, geo.timezone
            ),
            info: Some(TzSyncInfo {
                ip: geo.ip,
                country: geo.country_name,
                country_code: geo.country_code,
                iana_tz: geo.timezone,
                windows_tz: entry.key_name,
                at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            }),
        });
    }

    backup_current(data_dir)?;
    set_timezone(&entry)?;
    Ok(SyncResult {
        changed: true,
        message: format!(
            "时区已切换为 {}（{}，IP: {}）",
            entry.key_name, geo.timezone, geo.ip
        ),
        info: Some(TzSyncInfo {
            ip: geo.ip,
            country: geo.country_name,
            country_code: geo.country_code,
            iana_tz: geo.timezone,
            windows_tz: entry.key_name,
            at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        }),
    })
}

// ---------- IANA ↔ Windows 时区映射 ----------

/// 常见 IANA → Windows 时区映射
const IANA_TO_WINDOWS: &[(&str, &str)] = &[
    ("Asia/Shanghai", "China Standard Time"),
    ("Asia/Urumqi", "China Standard Time"),
    ("Asia/Hong_Kong", "China Standard Time"),
    ("Asia/Macau", "China Standard Time"),
    ("Asia/Taipei", "Taipei Standard Time"),
    ("Asia/Tokyo", "Tokyo Standard Time"),
    ("Asia/Seoul", "Korea Standard Time"),
    ("Asia/Singapore", "Singapore Standard Time"),
    ("Asia/Kuala_Lumpur", "Singapore Standard Time"),
    ("Asia/Kuching", "Singapore Standard Time"),
    ("Asia/Manila", "Singapore Standard Time"),
    ("Asia/Brunei", "Singapore Standard Time"),
    ("Asia/Makassar", "Singapore Standard Time"),
    ("Asia/Jakarta", "SE Asia Standard Time"),
    ("Asia/Bangkok", "SE Asia Standard Time"),
    ("Asia/Ho_Chi_Minh", "SE Asia Standard Time"),
    ("Asia/Saigon", "SE Asia Standard Time"),
    ("Asia/Phnom_Penh", "SE Asia Standard Time"),
    ("Asia/Vientiane", "SE Asia Standard Time"),
    ("Asia/Yangon", "Myanmar Standard Time"),
    ("Asia/Kolkata", "India Standard Time"),
    ("Asia/Calcutta", "India Standard Time"),
    ("Asia/Colombo", "India Standard Time"),
    ("Asia/Kathmandu", "Nepal Standard Time"),
    ("Asia/Dhaka", "Bangladesh Standard Time"),
    ("Asia/Karachi", "Pakistan Standard Time"),
    ("Asia/Kabul", "Afghanistan Standard Time"),
    ("Asia/Tehran", "Iran Standard Time"),
    ("Asia/Baghdad", "Arabic Standard Time"),
    ("Asia/Riyadh", "Arab Standard Time"),
    ("Asia/Qatar", "Arab Standard Time"),
    ("Asia/Kuwait", "Arab Standard Time"),
    ("Asia/Bahrain", "Arab Standard Time"),
    ("Asia/Dubai", "Arabian Standard Time"),
    ("Asia/Muscat", "Arabian Standard Time"),
    ("Asia/Jerusalem", "Israel Standard Time"),
    ("Asia/Amman", "Jordan Standard Time"),
    ("Asia/Beirut", "Middle East Standard Time"),
    ("Asia/Damascus", "Syria Standard Time"),
    ("Asia/Nicosia", "GTB Standard Time"),
    ("Asia/Almaty", "Central Asia Standard Time"),
    ("Asia/Tashkent", "West Asia Standard Time"),
    ("Asia/Yekaterinburg", "Ekaterinburg Standard Time"),
    ("Asia/Novosibirsk", "N. Central Asia Standard Time"),
    ("Asia/Kolkata1", "India Standard Time"),
    ("Europe/London", "GMT Standard Time"),
    ("Europe/Dublin", "GMT Standard Time"),
    ("Europe/Lisbon", "GMT Standard Time"),
    ("Atlantic/Reykjavik", "GMT Standard Time"),
    ("Atlantic/Canary", "GMT Standard Time"),
    ("Europe/Paris", "Romance Standard Time"),
    ("Europe/Madrid", "Romance Standard Time"),
    ("Europe/Brussels", "Romance Standard Time"),
    ("Europe/Amsterdam", "W. Europe Standard Time"),
    ("Europe/Berlin", "W. Europe Standard Time"),
    ("Europe/Rome", "W. Europe Standard Time"),
    ("Europe/Vienna", "W. Europe Standard Time"),
    ("Europe/Zurich", "W. Europe Standard Time"),
    ("Europe/Stockholm", "W. Europe Standard Time"),
    ("Europe/Oslo", "W. Europe Standard Time"),
    ("Europe/Copenhagen", "W. Europe Standard Time"),
    ("Europe/Prague", "Central European Standard Time"),
    ("Europe/Warsaw", "Central European Standard Time"),
    ("Europe/Budapest", "Central European Standard Time"),
    ("Europe/Belgrade", "Central European Standard Time"),
    ("Europe/Zagreb", "Central European Standard Time"),
    ("Europe/Athens", "GTB Standard Time"),
    ("Europe/Bucharest", "GTB Standard Time"),
    ("Europe/Kyiv", "FLE Standard Time"),
    ("Europe/Kiev", "FLE Standard Time"),
    ("Europe/Helsinki", "FLE Standard Time"),
    ("Europe/Riga", "FLE Standard Time"),
    ("Europe/Vilnius", "FLE Standard Time"),
    ("Europe/Tallinn", "FLE Standard Time"),
    ("Europe/Sofia", "FLE Standard Time"),
    ("Europe/Istanbul", "Turkey Standard Time"),
    ("Europe/Moscow", "Russian Standard Time"),
    ("Europe/Minsk", "Belarus Standard Time"),
    ("Atlantic/Azores", "Azores Standard Time"),
    ("America/New_York", "Eastern Standard Time"),
    ("America/Detroit", "Eastern Standard Time"),
    ("America/Toronto", "Eastern Standard Time"),
    ("America/Montreal", "Eastern Standard Time"),
    ("America/Havana", "Cuba Standard Time"),
    ("America/Chicago", "Central Standard Time"),
    ("America/Winnipeg", "Central Standard Time"),
    ("America/Mexico_City", "Central Standard Time (Mexico)"),
    ("America/Denver", "Mountain Standard Time"),
    ("America/Edmonton", "Mountain Standard Time"),
    ("America/Phoenix", "US Mountain Standard Time"),
    ("America/Los_Angeles", "Pacific Standard Time"),
    ("America/Vancouver", "Pacific Standard Time"),
    ("America/Tijuana", "Pacific Standard Time (Mexico)"),
    ("America/Anchorage", "Alaskan Standard Time"),
    ("Pacific/Honolulu", "Hawaiian Standard Time"),
    ("America/Halifax", "Atlantic Standard Time"),
    ("America/St_Johns", "Newfoundland Standard Time"),
    ("America/Bogota", "SA Pacific Standard Time"),
    ("America/Lima", "SA Pacific Standard Time"),
    ("America/Panama", "SA Pacific Standard Time"),
    ("America/Caracas", "Venezuela Standard Time"),
    ("America/Santiago", "Pacific SA Standard Time"),
    ("America/Sao_Paulo", "E. South America Standard Time"),
    ("America/Argentina/Buenos_Aires", "Argentina Standard Time"),
    ("America/Montevideo", "Montevideo Standard Time"),
    ("Australia/Sydney", "AUS Eastern Standard Time"),
    ("Australia/Melbourne", "AUS Eastern Standard Time"),
    ("Australia/Brisbane", "E. Australia Standard Time"),
    ("Australia/Perth", "W. Australia Standard Time"),
    ("Australia/Adelaide", "Cen. Australia Standard Time"),
    ("Pacific/Auckland", "New Zealand Standard Time"),
    ("Pacific/Fiji", "Fiji Standard Time"),
    ("Pacific/Guam", "West Pacific Standard Time"),
    ("Africa/Johannesburg", "South Africa Standard Time"),
    ("Africa/Cairo", "Egypt Standard Time"),
    ("Africa/Lagos", "W. Central Africa Standard Time"),
    ("Africa/Nairobi", "E. Africa Standard Time"),
    ("Africa/Casablanca", "Morocco Standard Time"),
    ("Africa/Algiers", "W. Central Africa Standard Time"),
    ("Africa/Tunis", "W. Central Africa Standard Time"),
    ("UTC", "UTC"),
    ("Etc/UTC", "UTC"),
    ("Etc/GMT", "GMT Standard Time"),
];

/// 反查：Windows 时区名 → 常用 IANA
pub fn windows_to_iana(win: &str) -> Option<&'static str> {
    const PREFERRED: &[(&str, &str)] = &[
        ("China Standard Time", "Asia/Shanghai"),
        ("Taipei Standard Time", "Asia/Taipei"),
        ("Tokyo Standard Time", "Asia/Tokyo"),
        ("Korea Standard Time", "Asia/Seoul"),
        ("Singapore Standard Time", "Asia/Singapore"),
        ("SE Asia Standard Time", "Asia/Bangkok"),
        ("India Standard Time", "Asia/Kolkata"),
        ("GMT Standard Time", "Europe/London"),
        ("Romance Standard Time", "Europe/Paris"),
        ("W. Europe Standard Time", "Europe/Berlin"),
        ("Central European Standard Time", "Europe/Warsaw"),
        ("GTB Standard Time", "Europe/Bucharest"),
        ("FLE Standard Time", "Europe/Kyiv"),
        ("Eastern Standard Time", "America/New_York"),
        ("Central Standard Time", "America/Chicago"),
        ("Mountain Standard Time", "America/Denver"),
        ("US Mountain Standard Time", "America/Phoenix"),
        ("Pacific Standard Time", "America/Los_Angeles"),
        ("Alaskan Standard Time", "America/Anchorage"),
        ("Hawaiian Standard Time", "Pacific/Honolulu"),
        ("AUS Eastern Standard Time", "Australia/Sydney"),
        ("E. Australia Standard Time", "Australia/Brisbane"),
        ("W. Australia Standard Time", "Australia/Perth"),
        ("Cen. Australia Standard Time", "Australia/Adelaide"),
        ("New Zealand Standard Time", "Pacific/Auckland"),
        ("Russian Standard Time", "Europe/Moscow"),
        ("Belarus Standard Time", "Europe/Minsk"),
        ("UTC", "UTC"),
    ];
    PREFERRED.iter().find(|(k, _)| *k == win).map(|(_, v)| *v)
}

pub fn iana_to_windows(iana: &str) -> Option<&'static str> {
    IANA_TO_WINDOWS
        .iter()
        .find(|(k, _)| *k == iana)
        .map(|(_, v)| *v)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_timezone_backup_is_kept_for_later_manual_and_automatic_changes() {
        let fixture = crate::test_support::FixtureDir::new();
        let root = fixture.path().to_path_buf();
        let first = backup_current(&root).unwrap();
        assert!(!first.key_name.is_empty());
        let repeated = backup_current(&root).unwrap();
        assert_eq!(repeated.key_name, first.key_name);
        assert_eq!(repeated.backed_up_at, first.backed_up_at);
    }
    #[test]
    fn timezone_information_preserves_windows_key_for_readback() {
        let entry = TzEntry {
            key_name: "Eastern Standard Time".into(),
            std_name: "Eastern Standard Time".into(),
            dlt_name: "Eastern Daylight Time".into(),
            display: "Eastern".into(),
            bias: 300,
            std_bias: 0,
            dlt_bias: -60,
            std_date: SYSTEMTIME::default(),
            dlt_date: SYSTEMTIME::default(),
        };
        let value = timezone_information(&entry).unwrap();
        assert_eq!(from_wide(&value.TimeZoneKeyName), entry.key_name);
        assert_eq!(value.Bias, 300);
        assert_eq!(value.DaylightBias, -60);
        let invalid = TzEntry {
            key_name: "x".repeat(129),
            ..entry
        };
        assert!(timezone_information(&invalid).is_err());
    }
}
