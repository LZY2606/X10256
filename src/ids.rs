//! Stable content identifiers and deterministic timestamps.
//!
//! Identifiers are derived from content so equivalent inputs always produce
//! the same id and ordering, before/after export/import.

use sha2::{Digest, Sha256};

pub fn short_hash(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for (i, p) in parts.iter().enumerate() {
        h.update((i as u64).to_le_bytes());
        h.update((p.len() as u64).to_le_bytes());
        h.update(p.as_bytes());
    }
    let d = h.finalize();
    let mut s = String::with_capacity(16);
    for b in d.iter().take(8) {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Deterministic, monotonic logical time ("sequence number").
pub fn logical_time(seq: u64) -> String {
    format!("seq:{seq:08}")
}
