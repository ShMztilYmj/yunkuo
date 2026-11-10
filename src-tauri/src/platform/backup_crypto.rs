//! 应用备份与 AI 备份共用的密码封装；保持旧格式的字节布局与认证参数。
use anyhow::{Result, bail};
use ring::{aead, pbkdf2};
use std::num::NonZeroU32;
use zeroize::Zeroizing;

fn cipher(password: &str, salt: &[u8]) -> Result<aead::LessSafeKey> {
    if !(12..=256).contains(&password.chars().count()) {
        bail!("备份密码须为 12–256 个字符");
    }
    let mut key = Zeroizing::new([0u8; 32]);
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(210_000).unwrap(),
        salt,
        password.as_bytes(),
        key.as_mut(),
    );
    Ok(aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, key.as_ref())
            .map_err(|_| anyhow::anyhow!("加密初始化失败"))?,
    ))
}
#[cfg(test)]
pub(crate) fn seal(
    magic: &[u8; 8],
    limit: usize,
    mut plain: Zeroizing<Vec<u8>>,
    password: &str,
) -> Result<Vec<u8>> {
    use ring::rand::{SecureRandom, SystemRandom};
    if plain.len() > limit.saturating_sub(128) {
        bail!("备份内容过大");
    }
    let mut salt = [0; 16];
    let mut nonce = [0; 12];
    let rng = SystemRandom::new();
    rng.fill(&mut salt)
        .map_err(|_| anyhow::anyhow!("随机数生成失败"))?;
    rng.fill(&mut nonce)
        .map_err(|_| anyhow::anyhow!("随机数生成失败"))?;
    cipher(password, &salt)?
        .seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(magic.as_slice()),
            &mut *plain,
        )
        .map_err(|_| anyhow::anyhow!("备份加密失败"))?;
    let mut out = Vec::with_capacity(36 + plain.len());
    out.extend_from_slice(magic);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&plain);
    Ok(out)
}
pub(crate) fn open(
    magic: &[u8; 8],
    limit: usize,
    bytes: &[u8],
    password: &str,
) -> Result<Zeroizing<Vec<u8>>> {
    if bytes.len() < 52 || bytes.len() > limit || !bytes.starts_with(magic) {
        bail!("备份格式无效或超过大小限制");
    }
    let mut plain = Zeroizing::new(bytes[36..].to_vec());
    let len = cipher(password, &bytes[8..24])?
        .open_in_place(
            aead::Nonce::assume_unique_for_key(bytes[24..36].try_into()?),
            aead::Aad::from(magic.as_slice()),
            &mut plain,
        )
        .map_err(|_| anyhow::anyhow!("密码错误或备份损坏，当前数据未修改"))?
        .len();
    plain.truncate(len);
    Ok(plain)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_previous_envelope_layout_and_checks_type_password_and_integrity() {
        // 按旧实现的固定盐、nonce 和字节布局制造文件，避免只验证新实现自身往返。
        let magic = b"COCBACK1";
        let salt = [7u8; 16];
        let nonce = [9u8; 12];
        let mut old_body = b"previous-format-fixture".to_vec();
        let mut key = [0u8; 32];
        pbkdf2::derive(
            pbkdf2::PBKDF2_HMAC_SHA256,
            NonZeroU32::new(210_000).unwrap(),
            &salt,
            b"fixture-password-long",
            &mut key,
        );
        aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_256_GCM, &key).unwrap())
            .seal_in_place_append_tag(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(magic.as_slice()),
                &mut old_body,
            )
            .unwrap();
        let mut bytes = [magic.as_slice(), &salt, &nonce, &old_body].concat();
        assert_eq!(
            &*open(magic, 4096, &bytes, "fixture-password-long").unwrap(),
            b"previous-format-fixture"
        );
        assert!(open(b"YUNAI001", 4096, &bytes, "fixture-password-long").is_err());
        assert!(open(magic, 4096, &bytes, "wrong-password-long").is_err());
        *bytes.last_mut().unwrap() ^= 1;
        assert!(open(magic, 4096, &bytes, "fixture-password-long").is_err());
        assert!(open(magic, 4096, &bytes[..20], "fixture-password-long").is_err());
    }
}
