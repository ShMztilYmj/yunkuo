// 默认浏览器联动：全局代理 + TUN 同时开启时，把系统 http/https 默认打开方式
// 切换为受控指纹 Chromium（每身份独立资料）；条件撤销或应用退出时自动还原。
//
// Windows 8+ 的 UserChoice 键带签名哈希，直接改写会被系统忽略。这里移植
// PS-SFTA（MIT License, © DanysysTeam）逆向的哈希算法：MD5 + 双轮魔数混淆，
// 输入串 = 协议 + 用户SID + ProgId + 分钟级 FILETIME + UserExperience 标记串（全小写）。
// 写入前先删 UserChoice 键再重建，写入后用键的最后写入时间回读自校验。
use crate::config::{ConfigMutex, DefaultBrowserBackup};
use anyhow::{Result, anyhow, bail};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::Manager;
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, FILETIME, HANDLE};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteKeyValueW, RegDeleteTreeW, RegOpenKeyExW, RegQueryInfoKeyW,
    RegQueryValueExW, RegSetValueExW,
};
use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHChangeNotify};
use windows::core::PCWSTR;

/// HTTP/HTTPS 关联只进入主程序门控，不能直接启动引擎。
pub const BROWSER_PROGID: &str = "Yunkuo.FingerprintBrowser";
/// 「默认应用」里的登记名：StartMenuInternet 子键与 RegisteredApplications 值共用。
pub const BROWSER_CLIENT: &str = "Yunkuo";
/// 历史遗留的 ProgId：只用于迁移既有用户关联与卸载清理，新装不再写入。
const LEGACY_PROGIDS: [&str; 2] = ["ClaudeOC.FingerprintBrowser", "FirefoxBuiltin.HTML"];
/// 历史遗留的应用登记名（旧版本叫 Claude One Click）。
const LEGACY_CLIENTS: [&str; 1] = ["ClaudeOneClick"];

const URL_ASSOC_ROOT: &str = "Software\\Microsoft\\Windows\\Shell\\Associations\\UrlAssociations";

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn data_dir(app: &tauri::AppHandle) -> PathBuf {
    app.path().app_data_dir().expect("无法获取数据目录")
}

// ---------------- 哈希算法（移植自 PS-SFTA Get-Hash） ----------------

fn get_long(bytes: &[u8], idx: usize) -> i32 {
    let mut b = [0u8; 4];
    b.copy_from_slice(&bytes[idx..idx + 4]);
    i32::from_le_bytes(b)
}

/// PS Get-ShiftRight：对带符号 64 位值做算术右移；bit31 为 1 时与 0xFFFF0000 异或
fn shr_ps(v: i64, n: u32) -> i64 {
    if v & 0x8000_0000i64 != 0 {
        (v >> n) ^ 0xFFFF_0000i64
    } else {
        v >> n
    }
}

/// PS Convert-Int32：取低 32 位（回绕）
fn conv_i32(v: i64) -> i32 {
    v as i32
}

/// 标准 MD5（RFC 1321）。常量用定义式现算，避免手抄 64 个魔数出错。
fn md5(data: &[u8]) -> [u8; 16] {
    // K[i] = floor(2^32 * |sin(i+1)|)，S 为各轮移位表
    let k: [u32; 64] =
        std::array::from_fn(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32);
    let s: [u32; 64] = std::array::from_fn(|i| match i / 16 {
        0 => [7, 12, 17, 22][i % 4],
        1 => [5, 9, 14, 20][i % 4],
        2 => [4, 11, 16, 23][i % 4],
        _ => [6, 10, 15, 21][i % 4],
    });

    // 填充：0x80 + 补零至 56 (mod 64) + 64 位小端比特长度
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());

    let mut a0: u32 = 0x67452301;
    let mut b0: u32 = 0xefcdab89;
    let mut c0: u32 = 0x98badcfe;
    let mut d0: u32 = 0x10325476;

    for chunk in msg.chunks_exact(64) {
        let mut m = [0u32; 16];
        for (i, w) in m.iter_mut().enumerate() {
            *w = u32::from_le_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f2 = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f2.rotate_left(s[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }

    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..16].copy_from_slice(&d0.to_le_bytes());
    out
}

const B64_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_encode(data: &[u8]) -> String {
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(B64_TABLE[(n >> 18) as usize & 63] as char);
        out.push(B64_TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64_TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64_TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Get-Hash 的 Rust 移植：输入串（UTF-16LE + 2 个 0x00）→ MD5 → 双轮混淆 → 8 字节 Base64
fn sfta_hash(base_info: &str) -> Option<String> {
    let mut base: Vec<u8> = base_info
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    base.extend_from_slice(&[0x00, 0x00]);
    let md5b = md5(&base);

    let length_base = (base_info.encode_utf16().count() * 2 + 2) as i64;
    let length = (((length_base & 4) <= 1) as i64) + shr_ps(length_base, 2) - 1;
    if length <= 1 {
        return None;
    }
    let index = shr_ps(length - 2, 1);
    let counter0 = index + 1;
    // 预补零，模拟 PS 数组的越界读取不会发生（保留原语义：读到的都是输入或 0）
    let need = (8 * (counter0.max(0) as usize) + 64).max(base.len());
    base.resize(need, 0);

    // 第一轮
    let (mut cache, mut outhash1, mut outhash2) = (0i64, 0i32, 0i32);
    let md51 = ((get_long(&md5b, 0) | 1) as i64) + 0x69FB_0000i64;
    let md52 = ((get_long(&md5b, 4) | 1) as i64) + 0x13DB_0000i64;
    let mut pdata = 0usize;
    let mut counter = counter0;
    while counter != 0 {
        let r0 = conv_i32((get_long(&base, pdata) as i64).wrapping_add(outhash1 as i64));
        let r1 = get_long(&base, pdata + 4);
        pdata += 8;
        let r2_0 = conv_i32(
            (r0 as i64)
                .wrapping_mul(md51)
                .wrapping_sub(0x10FA_9605i64.wrapping_mul(shr_ps(r0 as i64, 16))),
        );
        let r2_1 = conv_i32(
            0x79F8_A395i64
                .wrapping_mul(r2_0 as i64)
                .wrapping_add(0x689B_6B9Fi64.wrapping_mul(shr_ps(r2_0 as i64, 16))),
        );
        let r3 = conv_i32(
            0xEA97_0001i64
                .wrapping_mul(r2_1 as i64)
                .wrapping_sub(0x3C10_1569i64.wrapping_mul(shr_ps(r2_1 as i64, 16))),
        );
        let r4 = conv_i32((r3 as i64).wrapping_add(r1 as i64));
        let r5_0 = conv_i32(cache.wrapping_add(r3 as i64));
        let r6_0 = conv_i32(
            (r4 as i64)
                .wrapping_mul(md52)
                .wrapping_sub(0x3CE8_EC25i64.wrapping_mul(shr_ps(r4 as i64, 16))),
        );
        let r6_1 = conv_i32(
            0x59C3_AF2Di64
                .wrapping_mul(r6_0 as i64)
                .wrapping_sub(0x2232_E0F1i64.wrapping_mul(shr_ps(r6_0 as i64, 16))),
        );
        outhash1 = conv_i32(
            0x1EC9_0001i64
                .wrapping_mul(r6_1 as i64)
                .wrapping_add(0x35BD_1EC9i64.wrapping_mul(shr_ps(r6_1 as i64, 16))),
        );
        outhash2 = conv_i32((r5_0 as i64).wrapping_add(outhash1 as i64));
        cache = outhash2 as i64;
        counter -= 1;
    }
    let (h1, h2) = (outhash1, outhash2);

    // 第二轮（种子不同：无 +0x????0000）
    let mut out8 = [0u8; 16];
    out8[0..4].copy_from_slice(&h1.to_le_bytes());
    out8[4..8].copy_from_slice(&h2.to_le_bytes());
    let (mut cache, mut outhash1, mut outhash2) = (0i64, 0i32, 0i32);
    let md51 = (get_long(&md5b, 0) | 1) as i64;
    let md52 = (get_long(&md5b, 4) | 1) as i64;
    let mut pdata = 0usize;
    let mut counter = counter0;
    while counter != 0 {
        let r0 = conv_i32((get_long(&base, pdata) as i64).wrapping_add(outhash1 as i64));
        pdata += 8;
        let r1_0 = conv_i32((r0 as i64).wrapping_mul(md51));
        let r1_1 = conv_i32(
            0xB111_0000i64
                .wrapping_mul(r1_0 as i64)
                .wrapping_sub(0x3067_4EEFi64.wrapping_mul(shr_ps(r1_0 as i64, 16))),
        );
        let r2_0 = conv_i32(
            0x5B9F_0000i64
                .wrapping_mul(r1_1 as i64)
                .wrapping_sub(0x78F7_A461i64.wrapping_mul(shr_ps(r1_1 as i64, 16))),
        );
        let r2_1 = conv_i32(
            0x12CE_B96Di64
                .wrapping_mul(shr_ps(r2_0 as i64, 16))
                .wrapping_sub(0x4693_0000i64.wrapping_mul(r2_0 as i64)),
        );
        let r3 = conv_i32(
            0x1D83_0000i64
                .wrapping_mul(r2_1 as i64)
                .wrapping_add(0x257E_1D83i64.wrapping_mul(shr_ps(r2_1 as i64, 16))),
        );
        let r4 = conv_i32(
            md52.wrapping_mul((r3 as i64).wrapping_add(get_long(&base, pdata - 4) as i64)),
        );
        let r4_1 = conv_i32(
            0x16F5_0000i64
                .wrapping_mul(r4 as i64)
                .wrapping_sub(0x5D8B_E90Bi64.wrapping_mul(shr_ps(r4 as i64, 16))),
        );
        let r5_0 = conv_i32(
            0x96FF_0000i64
                .wrapping_mul(r4_1 as i64)
                .wrapping_sub(0x2C7C_6901i64.wrapping_mul(shr_ps(r4_1 as i64, 16))),
        );
        let r5_1 = conv_i32(
            0x2B89_0000i64
                .wrapping_mul(r5_0 as i64)
                .wrapping_add(0x7C93_2B89i64.wrapping_mul(shr_ps(r5_0 as i64, 16))),
        );
        outhash1 = conv_i32(
            0x9F69_0000i64
                .wrapping_mul(r5_1 as i64)
                .wrapping_sub(0x405B_6097i64.wrapping_mul(shr_ps(r5_1 as i64, 16))),
        );
        outhash2 = conv_i32(
            (outhash1 as i64)
                .wrapping_add(cache)
                .wrapping_add(r3 as i64),
        );
        cache = outhash2 as i64;
        counter -= 1;
    }
    out8[8..12].copy_from_slice(&outhash1.to_le_bytes());
    out8[12..16].copy_from_slice(&outhash2.to_le_bytes());

    let hv1 = get_long(&out8, 8) ^ get_long(&out8, 0);
    let hv2 = get_long(&out8, 12) ^ get_long(&out8, 4);
    let mut final8 = [0u8; 8];
    final8[0..4].copy_from_slice(&hv1.to_le_bytes());
    final8[4..8].copy_from_slice(&hv2.to_le_bytes());
    Some(b64_encode(&final8))
}

// ---------------- 输入串各分量 ----------------

/// 当前用户 SID（小写）：直接从进程令牌读取并手写转字符串，
/// 完全独立于 USERPROFILE/USERNAME 等环境变量（沙箱/提权启动下仍可靠）。
fn user_sid() -> Result<String> {
    use windows::Win32::Security::{GetTokenInformation, TOKEN_USER, TokenUser};
    use windows::Win32::System::Threading::GetCurrentProcess;
    unsafe {
        let mut token = HANDLE::default();
        windows::Win32::System::Threading::OpenProcessToken(
            GetCurrentProcess(),
            windows::Win32::Security::TOKEN_QUERY,
            &mut token,
        )
        .map_err(|e| anyhow!("OpenProcessToken 失败: {e}"))?;

        let mut len = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
        let mut buf = vec![0u8; len as usize];
        GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr() as *mut _),
            len,
            &mut len,
        )
        .map_err(|e| anyhow!("GetTokenInformation 失败: {e}"))?;
        let _ = CloseHandle(token);

        let user = *(buf.as_ptr() as *const TOKEN_USER);
        // user.User.Sid 指向 SID 结构：
        // [0]=Revision, [1]=SubAuthorityCount, [2..8]=IdentifierAuthority(BE), 之后为小端 SubAuthority
        let p = user.User.Sid.0 as *const u8;
        if p.is_null() {
            bail!("令牌无用户 SID");
        }
        let sid_bytes = std::slice::from_raw_parts(p, 8 + (*p.add(1) as usize) * 4);
        let _ = CloseHandle(token);
        Ok(sid_to_string(sid_bytes)?.to_lowercase())
    }
}

/// SID 结构 → "S-1-5-21-…" 标准字符串（ConvertSidToStringSidW 的最小实现）
fn sid_to_string(bytes: &[u8]) -> Result<String> {
    if bytes.len() < 8 {
        bail!("SID 长度不足");
    }
    let rev = bytes[0];
    let count = bytes[1] as usize;
    if 8 + count * 4 > bytes.len() {
        bail!("SID 缓冲区不完整");
    }
    let auth = ((bytes[2] as u64) << 40)
        | ((bytes[3] as u64) << 32)
        | ((bytes[4] as u64) << 24)
        | ((bytes[5] as u64) << 16)
        | ((bytes[6] as u64) << 8)
        | (bytes[7] as u64);
    let mut out = if auth > 0xFFFF_FFFFu64 {
        format!("S-{rev}-0x{auth:X}-")
    } else {
        format!("S-{rev}-{auth}-")
    };
    for i in 0..count {
        let v = u32::from_le_bytes([
            bytes[8 + i * 4],
            bytes[8 + i * 4 + 1],
            bytes[8 + i * 4 + 2],
            bytes[8 + i * 4 + 3],
        ]);
        out.push_str(&v.to_string());
        if i + 1 < count {
            out.push('-');
        }
    }
    Ok(out)
}

/// UserExperience 标记串：优先从 SysWOW64\shell32.dll 内读取，失败用硬编码兜底
fn user_experience() -> String {
    const FALLBACK: &str =
        "User Choice set via Windows User Experience {D18B6DD5-6124-4341-9318-804003BAFA0B}";
    let path = std::env::var("windir")
        .map(|w| format!("{w}\\SysWOW64\\shell32.dll"))
        .unwrap_or_else(|_| "C:\\Windows\\SysWOW64\\shell32.dll".to_string());
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return FALLBACK.to_string(),
    };
    let bytes = &bytes[..bytes.len().min(5 * 1024 * 1024)];
    let units: Vec<u16> = bytes
        .chunks(2)
        .map(|c| u16::from_le_bytes([c[0], *c.get(1).unwrap_or(&0)]))
        .collect();
    let pat: Vec<u16> = "User Choice set via Windows User Experience"
        .encode_utf16()
        .collect();
    let Some(p1) = units.windows(pat.len()).position(|w| w == pat.as_slice()) else {
        return FALLBACK.to_string();
    };
    let Some(rel) = units[p1..].iter().position(|&u| u == u16::from(b'}')) else {
        return FALLBACK.to_string();
    };
    String::from_utf16_lossy(&units[p1..=p1 + rel])
}

/// 分钟级 FILETIME 的 hex（高 32 位在前），与 PS Get-HexDateTime 一致
fn hex_datetime_now() -> String {
    let unix_min = (chrono::Utc::now().timestamp() / 60) * 60;
    let ft = (unix_min + 11_644_473_600) * 10_000_000i64;
    format!("{:08x}{:08x}", ft >> 32, ft & 0xFFFF_FFFF)
}

/// 完整哈希输入串（全小写）：协议 + SID + ProgId + 时间戳 + 体验串
fn base_info(protocol: &str, progid: &str) -> Result<String> {
    let sid = user_sid()?;
    Ok(format!(
        "{protocol}{sid}{progid}{}{}",
        hex_datetime_now(),
        user_experience()
    )
    .to_lowercase())
}

// ---------------- 注册表读写 ----------------

fn open_read(root: HKEY, path: &str) -> Result<HKEY> {
    unsafe {
        let mut h = HKEY::default();
        let pw = to_wide(path);
        let ret = RegOpenKeyExW(root, PCWSTR(pw.as_ptr()), None, KEY_READ, &mut h);
        if ret != ERROR_SUCCESS {
            bail!("打开注册表失败 {path} (code {})", ret.0);
        }
        Ok(h)
    }
}

fn reg_read_string(hkey: HKEY, name: &str) -> Result<String> {
    unsafe {
        let mut ty = REG_SZ;
        let mut cb = 0u32;
        let name_wide = to_wide(name);
        let pname = PCWSTR(name_wide.as_ptr());
        // 先查大小再读，避免超长截断
        let _ = RegQueryValueExW(hkey, pname, None, Some(&mut ty), None, Some(&mut cb));
        if cb == 0 {
            bail!("值不存在: {name}");
        }
        let mut buf = vec![0u8; cb as usize];
        let ret = RegQueryValueExW(
            hkey,
            pname,
            None,
            Some(&mut ty),
            Some(buf.as_mut_ptr()),
            Some(&mut cb),
        );
        if ret != ERROR_SUCCESS {
            bail!("读取注册表值失败 {name} (code {})", ret.0);
        }
        while buf.len() >= 2 && buf[buf.len() - 1] == 0 && buf[buf.len() - 2] == 0 {
            buf.pop();
            buf.pop();
        }
        let units: Vec<u16> = buf
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let s = String::from_utf16_lossy(&units);
        Ok(s.trim_end_matches('\0').to_string())
    }
}

fn reg_write_sz(h: HKEY, name: Option<&str>, value: &str) -> Result<()> {
    unsafe {
        let bytes: Vec<u8> = value
            .encode_utf16()
            .chain(Some(0))
            .flat_map(|u| u.to_le_bytes())
            .collect();
        let name_wide = name.map(to_wide);
        let name_ptr = match name_wide.as_ref() {
            Some(n) => PCWSTR(n.as_ptr()),
            None => PCWSTR::null(),
        };
        let ret = RegSetValueExW(h, name_ptr, None, REG_SZ, Some(&bytes));
        if ret != ERROR_SUCCESS {
            bail!("写注册表值失败 (code {})", ret.0);
        }
    }
    Ok(())
}

/// 读取某协议当前 UserChoice ProgId（不存在返回 None）
pub fn read_user_choice_progid(protocol: &str) -> Option<String> {
    let path = format!("{URL_ASSOC_ROOT}\\{protocol}\\UserChoice");
    let h = open_read(HKEY_CURRENT_USER, &path).ok()?;
    let v = reg_read_string(h, "ProgId").ok();
    unsafe {
        let _ = RegCloseKey(h);
    }
    v.filter(|s| !s.is_empty())
}

/// 删除并重建 UserChoice 键，写入 ProgId + 签名哈希；写后自校验（最多重试 3 次）
fn set_user_choice(protocol: &str, progid: &str) -> Result<()> {
    let mut last_err = String::new();
    for _ in 0..3 {
        let input = base_info(protocol, progid)?;
        let Some(hash) = sfta_hash(&input) else {
            bail!("哈希计算失败（输入过短）");
        };
        unsafe {
            let del_path = format!("{URL_ASSOC_ROOT}\\{protocol}\\UserChoice");
            let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(to_wide(&del_path).as_ptr()));
            let mut h = HKEY::default();
            let pw = to_wide(&del_path);
            let ret = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(pw.as_ptr()),
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_READ | KEY_WRITE,
                None,
                &mut h,
                None,
            );
            if ret != ERROR_SUCCESS {
                bail!("创建 UserChoice 键失败 (code {})", ret.0);
            }
            let r = reg_write_sz(h, Some("Hash"), &hash)
                .and_then(|_| reg_write_sz(h, Some("ProgId"), progid));
            let _ = RegCloseKey(h);
            r?;
        }
        notify_assoc_changed();
        match verify_user_choice(protocol) {
            Ok(true) => return Ok(()),
            Ok(false) => last_err = "写入后自校验不匹配（可能为系统策略变更）".into(),
            Err(e) => last_err = format!("{e:#}"),
        }
    }
    bail!("设置默认浏览器失败：{last_err}")
}

/// 用注册表键的最后写入时间重算哈希并与存储 Hash 比对
fn verify_user_choice(protocol: &str) -> Result<bool> {
    let path = format!("{URL_ASSOC_ROOT}\\{protocol}\\UserChoice");
    unsafe {
        let mut h = HKEY::default();
        let ret = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(to_wide(&path).as_ptr()),
            None,
            KEY_READ,
            &mut h,
        );
        if ret != ERROR_SUCCESS {
            bail!("打开 UserChoice 失败 (code {})", ret.0);
        }
        let progid = reg_read_string(h, "ProgId");
        let stored = reg_read_string(h, "Hash");
        let mut ft = FILETIME::default();
        let q = RegQueryInfoKeyW(
            h,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(&mut ft),
        );
        let _ = RegCloseKey(h);
        let progid = progid?;
        let stored = stored?;
        if q != ERROR_SUCCESS {
            bail!("读取键时间戳失败 (code {})", q.0);
        }

        // 键最后写入时间截断到分钟
        let t = ((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64;
        let t_min = t - t.rem_euclid(600_000_000);
        let hex = format!("{:08x}{:08x}", t_min >> 32, t_min & 0xFFFF_FFFF);
        let sid = user_sid()?;
        let input = format!("{protocol}{sid}{progid}{hex}{}", user_experience()).to_lowercase();
        let Some(calc) = sfta_hash(&input) else {
            bail!("哈希计算失败");
        };
        Ok(calc == stored)
    }
}

fn notify_assoc_changed() {
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED,
            windows::Win32::UI::Shell::SHCNF_IDLIST,
            None,
            None,
        );
    }
}

// ---------------- 受控浏览器 ProgId 注册 ----------------

fn open_command(exe: &Path) -> String {
    format!("\"{}\" --browser-url \"%1\"", exe.to_string_lossy())
}

fn registration(exe: &Path, legacy: &[&str]) -> Vec<(String, String, String)> {
    let command = open_command(exe);
    let exe = exe.to_string_lossy();
    let capabilities = format!(
        "Software\\Clients\\StartMenuInternet\\{}\\Capabilities",
        BROWSER_CLIENT
    );
    let mut rows = vec![];
    let mut put = |path: String, name: &str, value: String| rows.push((path, name.into(), value));
    for id in std::iter::once(BROWSER_PROGID).chain(legacy.iter().copied()) {
        let base = format!("Software\\Classes\\{id}");
        put(base.clone(), "", "指纹 Chromium（云阔）".into());
        put(format!("{base}\\DefaultIcon"), "", format!("\"{exe}\",0"));
        put(format!("{base}\\shell\\open\\command"), "", command.clone());
    }
    put(
        capabilities.clone(),
        "ApplicationName",
        "云阔指纹浏览器".into(),
    );
    put(
        capabilities.clone(),
        "ApplicationDescription",
        "经网络保护与浏览器实测核验后打开网页".into(),
    );
    for protocol in ["http", "https"] {
        put(
            format!("{capabilities}\\URLAssociations"),
            protocol,
            BROWSER_PROGID.into(),
        );
    }
    put(
        "Software\\RegisteredApplications".into(),
        BROWSER_CLIENT,
        capabilities,
    );
    rows
}

/// 更新已有旧 ProgId 的打开命令，使残留关联同样通过主程序；保留旧个人资料。
pub fn prepare_registration() -> Result<()> {
    register_browser_progid(&std::env::current_exe()?)?;
    migrate_user_choice()
}

/// 本机还存在的历史 ProgId：只有从旧版本升级上来的用户才会有。
fn legacy_progids() -> Vec<&'static str> {
    LEGACY_PROGIDS
        .into_iter()
        .filter(|id| {
            open_read(HKEY_CURRENT_USER, &format!("Software\\Classes\\{id}"))
                .map(|handle| unsafe {
                    let _ = RegCloseKey(handle);
                })
                .is_ok()
        })
        .collect()
}

/// 旧 ProgId 仍占着 http/https 时改指向新 ProgId，避免升级后链接无人接管。
fn migrate_user_choice() -> Result<()> {
    for protocol in ["http", "https"] {
        let legacy = read_user_choice_progid(protocol)
            .is_some_and(|id| LEGACY_PROGIDS.contains(&id.as_str()));
        if legacy {
            set_user_choice(protocol, BROWSER_PROGID)?;
        }
    }
    Ok(())
}

/// 旧登记名只做清理，避免「默认应用」里同时出现两个云阔。
fn drop_legacy_clients() {
    unsafe {
        for client in LEGACY_CLIENTS {
            let _ = RegDeleteTreeW(
                HKEY_CURRENT_USER,
                PCWSTR(
                    to_wide(&format!("Software\\Clients\\StartMenuInternet\\{client}")).as_ptr(),
                ),
            );
            let _ = RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                PCWSTR(to_wide("Software\\RegisteredApplications").as_ptr()),
                PCWSTR(to_wide(client).as_ptr()),
            );
        }
    }
}

fn register_browser_progid(exe: &Path) -> Result<()> {
    let legacy = legacy_progids();
    for (path, name, value) in registration(exe, &legacy) {
        unsafe {
            let mut h = HKEY::default();
            let ret = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(to_wide(&path).as_ptr()),
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_READ | KEY_WRITE,
                None,
                &mut h,
                None,
            );
            if ret != ERROR_SUCCESS {
                bail!("注册受控浏览器失败 (code {})", ret.0);
            }
            let result = reg_write_sz(h, if name.is_empty() { None } else { Some(&name) }, &value);
            let _ = RegCloseKey(h);
            result?;
        }
    }
    drop_legacy_clients();
    notify_assoc_changed();
    Ok(())
}

// ---------------- 切换 / 恢复 / 联动 ----------------

fn persist_backup(app: &tauri::AppHandle, backup: Option<DefaultBrowserBackup>) -> Result<()> {
    let mut cfg = app.state::<ConfigMutex>().lock().clone();
    cfg.default_browser_backup = backup;
    crate::config::FileStore::new(data_dir(app).join("config.yaml")).save(&cfg)?;
    *app.state::<ConfigMutex>().lock() = cfg;
    Ok(())
}

fn set_protocol_choice(protocol: &str, progid: &str) -> Result<()> {
    if !progid.is_empty() {
        return set_user_choice(protocol, progid);
    }
    if !["http", "https"].contains(&protocol) {
        bail!("协议不合法");
    }
    let path = to_wide(&format!("{URL_ASSOC_ROOT}\\{protocol}\\UserChoice"));
    let result = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr())) };
    if result != ERROR_SUCCESS && result.0 != 2 {
        bail!("恢复默认协议关联失败 (code {})", result.0);
    }
    notify_assoc_changed();
    Ok(())
}

fn restore_pair(
    backup: &DefaultBrowserBackup,
    mut set: impl FnMut(&str, &str) -> Result<()>,
) -> Result<()> {
    let http = set("http", &backup.http);
    let https = set("https", &backup.https);
    match (http, https) {
        (Ok(()), Ok(())) => Ok(()),
        (http, https) => Err(anyhow!(
            "协议恢复未完成：http={:?}, https={:?}",
            http.err(),
            https.err()
        )),
    }
}

fn switch_pair(
    backup: &DefaultBrowserBackup,
    mut persist: impl FnMut(Option<DefaultBrowserBackup>) -> Result<()>,
    mut set: impl FnMut(&str, &str) -> Result<()>,
) -> Result<()> {
    persist(Some(backup.clone()))?;
    if let Err(error) = set("http", BROWSER_PROGID).and_then(|_| set("https", BROWSER_PROGID)) {
        if let Err(rollback) = restore_pair(backup, &mut set) {
            return Err(anyhow!(
                "浏览器切换失败，已保留恢复记录：{error}; {rollback}"
            ));
        }
        persist(None)?;
        return Err(error);
    }
    Ok(())
}

fn backup_for_switch(
    saved: Option<DefaultBrowserBackup>,
    mut read: impl FnMut(&str) -> Option<String>,
) -> DefaultBrowserBackup {
    saved.unwrap_or_else(|| DefaultBrowserBackup {
        http: read("http").unwrap_or_default(),
        https: read("https").unwrap_or_default(),
    })
}
fn switch_to_builtin(app: &tauri::AppHandle) -> Result<()> {
    if super::browser::engine(app).is_none() {
        bail!("指纹 Chromium 引擎未准备，未切换默认浏览器");
    }
    prepare_registration()?;
    // 升级/重试时继续使用原始备份，不能把本软件自己的关联当作原默认值。
    let saved = app
        .state::<ConfigMutex>()
        .lock()
        .default_browser_backup
        .clone();
    let backup = backup_for_switch(saved, read_user_choice_progid);
    switch_pair(
        &backup,
        |value| persist_backup(app, value),
        set_protocol_choice,
    )
}

pub fn restore(app: &tauri::AppHandle) -> Result<()> {
    let backup = app
        .state::<ConfigMutex>()
        .lock()
        .default_browser_backup
        .clone();
    if let Some(backup) = backup {
        restore_pair(&backup, set_protocol_choice)?;
        persist_backup(app, None)?;
    }
    Ok(())
}

/// 卸载用，不依赖 Tauri 状态：只还原仍指向本软件的协议，用户之后自己改过的默认浏览器不动；
/// 没有可用原值时清除关联，交给系统默认浏览器。
pub fn release_for_uninstall(backup: Option<&DefaultBrowserBackup>) -> Result<()> {
    release_pairs(backup, read_user_choice_progid, set_protocol_choice)
}

fn release_pairs(
    backup: Option<&DefaultBrowserBackup>,
    mut read: impl FnMut(&str) -> Option<String>,
    mut set: impl FnMut(&str, &str) -> Result<()>,
) -> Result<()> {
    let owned = |progid: &str| progid == BROWSER_PROGID || LEGACY_PROGIDS.contains(&progid);
    let mut errors = Vec::new();
    for protocol in ["http", "https"] {
        if !read(protocol).is_some_and(|current| owned(&current)) {
            continue;
        }
        let original = backup
            .map(|b| {
                if protocol == "http" {
                    b.http.as_str()
                } else {
                    b.https.as_str()
                }
            })
            .filter(|original| !owned(original))
            .unwrap_or("");
        if let Err(error) = set(protocol, original) {
            errors.push(format!("{protocol}={error:#}"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(anyhow!("协议恢复未完成：{}", errors.join(", ")))
    }
}

/// 卸载用：删除本软件写入的浏览器入口。旧 ProgId 仅在仍指向本程序时删除。
pub fn unregister() -> Result<()> {
    let delete_tree = |path: &str| -> Result<()> {
        let result = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(to_wide(path).as_ptr())) };
        if result != ERROR_SUCCESS && result.0 != 2 {
            bail!("删除 {path} 失败 (code {})", result.0);
        }
        Ok(())
    };
    delete_tree(&format!("Software\\Classes\\{BROWSER_PROGID}"))?;
    let command = open_command(&std::env::current_exe()?);
    for legacy in LEGACY_PROGIDS {
        let path = format!("Software\\Classes\\{legacy}");
        let current = open_read(HKEY_CURRENT_USER, &format!("{path}\\shell\\open\\command"))
            .ok()
            .and_then(|h| {
                let value = reg_read_string(h, "").ok();
                unsafe {
                    let _ = RegCloseKey(h);
                }
                value
            });
        if current.as_deref() == Some(command.as_str()) {
            delete_tree(&path)?;
        }
    }
    delete_tree(&format!(
        "Software\\Clients\\StartMenuInternet\\{}",
        BROWSER_CLIENT
    ))?;
    for client in LEGACY_CLIENTS {
        delete_tree(&format!("Software\\Clients\\StartMenuInternet\\{client}"))?;
    }
    let result = unsafe {
        RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            PCWSTR(to_wide("Software\\RegisteredApplications").as_ptr()),
            PCWSTR(to_wide(BROWSER_CLIENT).as_ptr()),
        )
    };
    if result != ERROR_SUCCESS && result.0 != 2 {
        bail!("删除浏览器应用登记失败 (code {})", result.0);
    }
    drop_legacy_clients();
    notify_assoc_changed();
    Ok(())
}

pub fn sync(app: &tauri::AppHandle) -> Result<()> {
    let cfg = app.state::<ConfigMutex>().lock().clone();
    let st = app.state::<crate::proxy::core::CoreState>();
    let cond = cfg.mode == "global"
        && cfg.enable_tun
        && proxy_runtime::adapter_up(crate::privacy::TUN_NAME)
        && st.is_running()
        && st.applied_tun("global");
    let switched = cfg.default_browser_backup.is_some();
    let target_matches = ["http", "https"]
        .iter()
        .all(|p| read_user_choice_progid(p).as_deref() == Some(BROWSER_PROGID));
    if cond && (!switched || !target_matches) {
        switch_to_builtin(app)
    } else if !cond && switched {
        restore(app)
    } else {
        Ok(())
    }
}

// ---------------- 状态查询命令 ----------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultBrowserStatus {
    /// http 协议当前 ProgId
    pub current: Option<String>,
    pub https: Option<String>,
    /// 是否保存了待还原的原始协议关联
    pub switched: bool,
}

#[tauri::command]
pub fn default_browser_status(app: tauri::AppHandle) -> DefaultBrowserStatus {
    let switched = app
        .state::<ConfigMutex>()
        .lock()
        .default_browser_backup
        .is_some();
    DefaultBrowserStatus {
        current: read_user_choice_progid("http"),
        https: read_user_choice_progid("https"),
        switched,
    }
}

// ---------------- 测试 ----------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_preserves_original_backup_and_unset_associations() {
        let original = DefaultBrowserBackup {
            http: "ChromeHTML".into(),
            https: "MSEdgeHTM".into(),
        };
        let reused = backup_for_switch(Some(original), |_| {
            panic!("已有备份时不能再读取旧版联动关联")
        });
        assert_eq!(reused.http, "ChromeHTML");
        assert_eq!(reused.https, "MSEdgeHTM");
        let fresh = backup_for_switch(None, |p| (p == "https").then(|| "OriginalHttps".into()));
        assert_eq!(fresh.http, "");
        assert_eq!(fresh.https, "OriginalHttps");
    }

    #[test]
    fn registration_routes_both_protocols_and_existing_legacy_through_host() {
        let exe = Path::new("C:\\Program Files\\云阔\\yunkuo.exe");
        let rows = registration(exe, &["ClaudeOC.FingerprintBrowser", "FirefoxBuiltin.HTML"]);
        let commands: Vec<_> = rows
            .iter()
            .filter(|(p, _, _)| p.ends_with("\\shell\\open\\command"))
            .collect();
        assert_eq!(
            commands.len(),
            3,
            "新 ProgId + 两个历史 ProgId 都改走主程序"
        );
        for (_, _, command) in commands {
            assert_eq!(
                command,
                "\"C:\\Program Files\\云阔\\yunkuo.exe\" --browser-url \"%1\""
            );
            assert!(!command.contains("chrome.exe"));
            assert!(!command.contains("firefox.exe"));
        }
        assert!(
            rows.iter()
                .any(|(p, n, _)| p == "Software\\RegisteredApplications" && n == BROWSER_CLIENT)
        );
        assert!(rows.iter().any(|(p, n, v)| p.ends_with("\\URLAssociations")
            && n == "https"
            && v == BROWSER_PROGID));
        assert!(
            !registration(exe, &[])
                .iter()
                .any(|(p, _, _)| p.contains("ClaudeOC.FingerprintBrowser"))
        );
    }

    #[test]
    fn legacy_client_names_are_cleaned_and_never_reused() {
        assert_eq!(BROWSER_CLIENT, "Yunkuo");
        assert!(!LEGACY_CLIENTS.contains(&BROWSER_CLIENT));
        assert!(!LEGACY_PROGIDS.contains(&BROWSER_PROGID));
    }

    #[test]
    fn registry_strings_roundtrip_unicode_and_include_terminator() {
        let path = to_wide(&format!(
            "Software\\YunkuoRegression\\{}",
            uuid::Uuid::new_v4()
        ));
        unsafe {
            let mut key = HKEY::default();
            let status = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(path.as_ptr()),
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_READ | KEY_WRITE,
                None,
                &mut key,
                None,
            );
            // 受管桌面或受限会话可能禁止写 HKCU：这类环境跳过用例，而不是误报回归。
            if status == windows::Win32::Foundation::ERROR_ACCESS_DENIED {
                eprintln!("跳过：当前会话不允许写 HKEY_CURRENT_USER");
                return;
            }
            assert_eq!(status, ERROR_SUCCESS);
            let value = "Firefox 中文 🦊".repeat(12);
            let result = (|| -> Result<()> {
                reg_write_sz(key, Some("StringValue"), &value)?;
                assert_eq!(reg_read_string(key, "StringValue")?, value);
                let name = to_wide("StringValue");
                let mut bytes = vec![0u8; 2048];
                let mut length = bytes.len() as u32;
                assert_eq!(
                    RegQueryValueExW(
                        key,
                        PCWSTR(name.as_ptr()),
                        None,
                        None,
                        Some(bytes.as_mut_ptr()),
                        Some(&mut length)
                    ),
                    ERROR_SUCCESS
                );
                assert_eq!(length as usize, (value.encode_utf16().count() + 1) * 2);
                assert_eq!(&bytes[length as usize - 2..length as usize], &[0, 0]);
                Ok(())
            })();
            let _ = RegCloseKey(key);
            assert_eq!(
                RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr())),
                ERROR_SUCCESS
            );
            result.unwrap();
        }
    }

    #[test]
    fn switching_persists_before_mutation_and_rolls_back_both_protocols() {
        use std::cell::RefCell;
        let calls = RefCell::new(Vec::<String>::new());
        let backup = DefaultBrowserBackup {
            http: "old-http".into(),
            https: "old-https".into(),
        };
        let result = switch_pair(
            &backup,
            |b| {
                calls
                    .borrow_mut()
                    .push(if b.is_some() { "save" } else { "clear" }.into());
                Ok(())
            },
            |protocol, value| {
                calls.borrow_mut().push(format!("{protocol}={value}"));
                if protocol == "https" && value == BROWSER_PROGID {
                    bail!("模拟 HTTPS 写入失败");
                }
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(
            *calls.borrow(),
            vec![
                "save",
                "http=Yunkuo.FingerprintBrowser",
                "https=Yunkuo.FingerprintBrowser",
                "http=old-http",
                "https=old-https",
                "clear"
            ]
        );
    }

    #[test]
    fn failed_backup_blocks_mutation_and_failed_rollback_keeps_backup() {
        use std::cell::Cell;
        let backup = DefaultBrowserBackup {
            http: "old".into(),
            https: "old".into(),
        };
        let count = Cell::new(0);
        assert!(
            switch_pair(
                &backup,
                |_| Err(anyhow!("磁盘故障")),
                |_, _| {
                    count.set(count.get() + 1);
                    Ok(())
                }
            )
            .is_err()
        );
        assert_eq!(count.get(), 0);
        let saved = Cell::new(false);
        assert!(
            switch_pair(
                &backup,
                |value| {
                    saved.set(value.is_some());
                    Ok(())
                },
                |_, _| Err(anyhow!("模拟注册表故障"))
            )
            .is_err()
        );
        assert!(saved.get());
    }

    #[test]
    fn uninstall_only_releases_protocols_still_owned_by_this_app() {
        let current = |http: &'static str, https: &'static str| {
            move |p: &str| Some(if p == "http" { http } else { https }.to_string())
        };
        let backup = DefaultBrowserBackup {
            http: "ChromeHTML".into(),
            https: "ChromeHTML".into(),
        };
        let mut calls = Vec::new();
        release_pairs(
            Some(&backup),
            current(BROWSER_PROGID, "MSEdgeHTM"),
            |p, id| {
                calls.push((p.to_string(), id.to_string()));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            calls,
            [("http".to_string(), "ChromeHTML".to_string())],
            "用户后来改过的 https 不能被覆盖"
        );

        let stale = DefaultBrowserBackup {
            http: BROWSER_PROGID.into(),
            https: String::new(),
        };
        let mut calls = Vec::new();
        release_pairs(
            Some(&stale),
            current(BROWSER_PROGID, LEGACY_PROGIDS[1]),
            |p, id| {
                calls.push((p.to_string(), id.to_string()));
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            calls,
            [
                ("http".into(), String::new()),
                ("https".into(), String::new())
            ],
            "原值仍是本软件时清除关联"
        );

        let mut calls = 0;
        let failed = release_pairs(None, current(BROWSER_PROGID, BROWSER_PROGID), |p, _| {
            calls += 1;
            if p == "http" {
                Err(anyhow!("模拟注册表故障"))
            } else {
                Ok(())
            }
        });
        assert!(
            failed.is_err() && calls == 2,
            "单个协议失败不能跳过另一个协议"
        );
    }

    #[test]
    fn md5_known_vectors() {
        let hex = |d: &[u8]| b64_encode(&md5(d)); // 仅验证可运行；标准向量用十六进制比对
        let h = md5(b"abc");
        let expect = "900150983cd24fb0d6963f7d28e17f72";
        let got: String = h.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(got, expect, "md5(abc) 应为标准向量");
        assert_eq!(hex(b"a").len(), 24); // base64(16 bytes) == 24 字符
    }

    /// 对照现有系统 UserChoice 键验证哈希算法（不写入任何值）
    #[test]
    fn existing_userchoice_hash_matches() {
        for proto in ["http", "https"] {
            let Some(progid) = read_user_choice_progid(proto) else {
                println!("[skip] {proto} 无 UserChoice");
                continue;
            };
            let path = format!("{URL_ASSOC_ROOT}\\{proto}\\UserChoice");
            unsafe {
                let h = open_read(HKEY_CURRENT_USER, &path).unwrap();
                let stored = reg_read_string(h, "Hash").unwrap();
                let mut ft = FILETIME::default();
                if RegQueryInfoKeyW(
                    h,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(&mut ft),
                ) != ERROR_SUCCESS
                {
                    panic!("RegQueryInfoKeyW 失败");
                }
                let _ = RegCloseKey(h);
                let t = ((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64;
                let t_min = t - t.rem_euclid(600_000_000);
                let hex = format!("{:08x}{:08x}", t_min >> 32, t_min & 0xFFFF_FFFF);
                let sid = user_sid().unwrap();
                let input =
                    format!("{proto}{sid}{progid}{hex}{}", user_experience()).to_lowercase();
                let calc = sfta_hash(&input).unwrap();
                println!("{proto}: progid={progid} stored={stored} calc={calc}");
                assert_eq!(calc, stored, "{proto} 哈希应与系统存储一致");
            }
        }
    }

    /// 真机回环：切换 → 校验 → 还原 → 校验原值（需 --ignored 运行，会短暂改变默认浏览器）
    #[test]
    #[ignore]
    fn live_switch_restore_roundtrip() {
        let exe = std::env::current_exe().unwrap();
        let orig_http = read_user_choice_progid("http");
        let orig_https = read_user_choice_progid("https");
        println!("原默认: http={orig_http:?} https={orig_https:?}");

        register_browser_progid(&exe).unwrap();
        set_user_choice("http", BROWSER_PROGID).unwrap();
        set_user_choice("https", BROWSER_PROGID).unwrap();
        assert_eq!(
            read_user_choice_progid("http").as_deref(),
            Some(BROWSER_PROGID)
        );
        assert_eq!(
            read_user_choice_progid("https").as_deref(),
            Some(BROWSER_PROGID)
        );
        println!("切换成功且自校验通过");

        // 还原
        if let Some(p) = &orig_http {
            set_user_choice("http", p).unwrap();
        } else {
            unsafe {
                let _ = RegDeleteTreeW(
                    HKEY_CURRENT_USER,
                    PCWSTR(to_wide(&format!("{URL_ASSOC_ROOT}\\http\\UserChoice")).as_ptr()),
                );
            }
        }
        if let Some(p) = &orig_https {
            set_user_choice("https", p).unwrap();
        } else {
            unsafe {
                let _ = RegDeleteTreeW(
                    HKEY_CURRENT_USER,
                    PCWSTR(to_wide(&format!("{URL_ASSOC_ROOT}\\https\\UserChoice")).as_ptr()),
                );
            }
        }
        assert_eq!(read_user_choice_progid("http"), orig_http, "http 应还原");
        assert_eq!(read_user_choice_progid("https"), orig_https, "https 应还原");
        println!("还原成功，原默认浏览器已恢复");
    }
}
