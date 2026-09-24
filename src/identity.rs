//! Stable, content-addressed identity helpers.
//!
//! Identities are derived from the logical content they name. Re-importing an
//! identical material or re-running an engine with identical inputs therefore
//! yields an identical id, which is what makes export/delete/import preserve
//! identity and provenance without relying on mutable rowids.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub type Id = String;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdRef {
    pub id: Id,
    pub kind: String,
}

/// Deterministic content hash over canonical JSON.
pub fn content_hash<T: Serialize>(value: &T) -> String {
    let mut ser = serde_json::Serializer::with_formatter(
            Vec::new(),
            serde_json::ser::CompactFormatter,
        );
    value.serialize(&mut ser).expect("canonical serialize");
    let bytes = ser.into_inner();
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(16);
    for b in digest.iter().take(8) {
        hex.push_str(&format!("{:02x}", b));
    }
    hex
}

/// Stable id of the form `<prefix>-<16 hex>`.
pub fn derive_id<T: Serialize>(prefix: &str, value: &T) -> Id {
    format!("{}-{}", prefix, content_hash(value))
}

/// Composite deterministic id from already-stable parts.
pub fn composite_id(prefix: &str, parts: &[&str]) -> Id {
    let mut hasher = Sha256::new();
    for p in parts {
        hasher.update(p.as_bytes());
        hasher.update([0x1f]);
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(16);
    for b in digest.iter().take(8) {
        hex.push_str(&format!("{:02x}", b));
    }
    format!("{}-{}", prefix, hex)
}
