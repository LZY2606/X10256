use sha2::{Digest, Sha256};

/// Stable short digest (hex, 16 chars) for identity derivation.
pub fn digest16(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update(b"\x1f");
    }
    let out = h.finalize();
    out.iter().take(8).map(|b| format!("{:02x}", b)).collect()
}

pub fn full_sha(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update(b"\x1f");
    }
    h.finalize().iter().map(|b| format!("{:02x}", b)).collect()
}
