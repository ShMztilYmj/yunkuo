//! 统一的 SHA-256 与十六进制转换。
//!
//! 各处校验对大小写的要求不同：安装包校验（`updates`）与 `build.rs` 绑定的大写哈希必须
//! 用 `hex_upper`，其余内部摘要保持小写，调用点不要互相替换。

use ring::digest::{SHA256, digest};

/// 原始 SHA-256 摘要。
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let digest = digest(&SHA256, data);
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(digest.as_ref());
    bytes
}

/// 小写十六进制，内部摘要与文件名使用这种形式。
pub fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 大写十六进制，仅用于与已固定的哈希声明比较。
pub fn hex_upper(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// SHA-256 摘要的小写十六进制。
pub fn sha256_lower(data: &[u8]) -> String {
    hex_lower(&sha256(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors_match_ring_and_keep_case() {
        assert_eq!(
            sha256_lower(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex_upper(&sha256(b"abc")),
            "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"
        );
        assert_eq!(
            hex_upper(&sha256(b"abc")),
            sha256_lower(b"abc").to_uppercase()
        );
        assert_eq!(sha256_lower(b"").len(), 64);
        assert_eq!(hex_lower(&[0x00, 0x0f, 0xff]), "000fff");
        assert_eq!(hex_upper(&[0x00, 0x0f, 0xff]), "000FFF");
    }
}
