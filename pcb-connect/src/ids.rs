//! 稳定身份：所有身份都基于内容或 (父链, 序号) 推导，等价输入产生等价身份。

/// FNV-1a 64 位内容指纹。
pub fn fnv1a64(parts: &[String]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for part in parts {
        for byte in part.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0x1f;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// 形如 `src_a1b2c3d4e5f60718` 的短稳定标识。
pub fn short_id(prefix: &str, parts: &[String]) -> String {
    format!("{}_{:016x}", prefix, fnv1a64(parts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_inputs_get_identical_ids() {
        assert_eq!(short_id("src", &["a".into(), "b".into()]),
                   short_id("src", &["a".into(), "b".into()]));
        assert_ne!(short_id("src", &["a".into()]), short_id("src", &["b".into()]));
    }
}
