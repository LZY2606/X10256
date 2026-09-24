//! Deterministic identity helpers.
//!
//! Identities are content-derived where possible so that exporting, deleting
//! the local database and re-importing reconstructs the same stable ids.

use serde::Serialize;
use sha2::{Digest, Sha256};

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// Canonical hash of any serializable value (sorted object keys, no whitespace).
pub fn canonical_hash<T: Serialize>(value: &T) -> String {
    let v = serde_json::to_value(value).expect("hashing values are serializable");
    canonical_hash_value(&v)
}

pub fn canonical_hash_value(v: &serde_json::Value) -> String {
    let mut buf = Vec::new();
    write_canonical(&mut buf, v);
    sha256_hex(&buf)
}

fn write_canonical(out: &mut Vec<u8>, v: &serde_json::Value) {
    use serde_json::Value;
    match v {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Value::Number(n) => out.extend_from_slice(n.to_string().as_bytes()),
        Value::String(s) => {
            out.push(b'"');
            write_json_string_escaped(out, s);
            out.push(b'"');
        }
        Value::Array(items) => {
            out.push(b'[');
            for (i, it) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_canonical(out, it);
            }
            out.push(b']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push(b'{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                out.push(b'"');
                write_json_string_escaped(out, k);
                out.extend_from_slice(b"\":");
                write_canonical(out, &map[*k]);
            }
            out.push(b'}');
        }
    }
}

fn write_json_string_escaped(out: &mut Vec<u8>, s: &str) {
    for b in s.as_bytes() {
        match b {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            0x08 => out.extend_from_slice(b"\\b"),
            0x0c => out.extend_from_slice(b"\\f"),
            c if *c < 0x20 => {
                out.extend_from_slice(format!("\\u{:04x}", c).as_bytes());
            }
            c => out.push(*c),
        }
    }
}

pub fn short_id(prefix: &str, full_hex: &str) -> String {
    format!("{prefix}{}", &full_hex[..full_hex.len().min(12)])
}
