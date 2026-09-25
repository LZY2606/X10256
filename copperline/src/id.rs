//! Deterministic, stable identities. IDs are content-addressed so a replay of
//! the same material/decision sequence produces identical IDs, and an
//! export/import round trip keeps them (they are also stored verbatim).

/// FNV-1a 64 bit, hex encoded. Pure and dependency free.
pub fn fnv1a64(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub fn short(parts: &[&str]) -> String {
    let mut joined = String::new();
    for p in parts {
        joined.push_str(p);
        joined.push('\u{1}');
    }
    format!("{:016x}", fnv1a64(joined.as_bytes()))
}

pub fn material_id(case: &str, file_name: &str, raw: &str) -> String {
    format!("mat_{}", short(&[case, file_name, raw]))
}

pub fn revision_id(material: &str, seq: usize) -> String {
    format!("rev_{}_{}", material, seq)
}

pub fn event_id(seq: usize, payload_hash: &str) -> String {
    format!("ev_{:04}_{}", seq, payload_hash)
}

pub fn branch_id(case: &str, name: &str) -> String {
    format!("br_{}", short(&[case, name]))
}

pub fn param_version_id(case: &str, version: u32) -> String {
    format!("pv_{}_{:03}", short(&[case]), version)
}

pub fn result_id(parts: &[&str]) -> String {
    format!("res_{}", short(parts))
}
