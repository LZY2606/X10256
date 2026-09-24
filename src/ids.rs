//! Stable content identities (FNV-1a-128) and canonical JSON hashing.
//! Identities never depend on wall-clock time or map iteration order.

use serde_json::Value;

const FNV_OFFSET: u128 = 0x6c62_272e_07bb_0142_62b7_2165_7762_3d69;
const FNV_PRIME: u128 = 0x0000_0100_0000_0000_0000_0000_0000_013b;

pub fn fnv128_hex(bytes: &[u8]) -> String {
    let mut hash = FNV_OFFSET;
    for &b in bytes {
        hash ^= b as u128;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:032x}")
}

pub fn short_id(prefix: &str, bytes: &[u8], len: usize) -> String {
    let h = fnv128_hex(bytes);
    format!("{prefix}_{}", &h[..len])
}

fn canon(buf: &mut Vec<u8>, v: &Value) {
    match v {
        Value::Null => buf.extend_from_slice(b"null"),
        Value::Bool(b) => buf.extend_from_slice(if *b { b"true" } else { b"false" }),
        Value::Number(n) => buf.extend_from_slice(n.to_string().as_bytes()),
        Value::String(s) => {
            buf.push(b'"');
            escape(s, buf);
            buf.push(b'"');
        }
        Value::Array(a) => {
            buf.push(b'[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 { buf.push(b','); }
                canon(buf, x);
            }
            buf.push(b']');
        }
        Value::Object(o) => {
            let sorted: std::collections::BTreeMap<&str, &Value> =
                o.iter().map(|(k, v)| (k.as_str(), v)).collect();
            buf.push(b'{');
            for (i, (k, x)) in sorted.iter().enumerate() {
                if i > 0 { buf.push(b','); }
                buf.push(b'"');
                escape(k, buf);
                buf.extend_from_slice(b"\":");
                canon(buf, x);
            }
            buf.push(b'}');
        }
    }
}

fn escape(s: &str, buf: &mut Vec<u8>) {
    for c in s.chars() {
        match c {
            '"' => buf.extend_from_slice(b"\\\""),
            '\\' => buf.extend_from_slice(b"\\\\"),
            '\n' => buf.extend_from_slice(b"\\n"),
            '\r' => buf.extend_from_slice(b"\\r"),
            '\t' => buf.extend_from_slice(b"\\t"),
            c if (c as u32) < 0x20 => buf.extend_from_slice(format!("\\u{:04x}", c as u32).as_bytes()),
            c => {
                let mut tmp = [0u8; 4];
                buf.extend_from_slice(c.encode_utf8(&mut tmp).as_bytes());
            }
        }
    }
}

pub fn canonical(v: &Value) -> Vec<u8> {
    let mut buf = Vec::new();
    canon(&mut buf, v);
    buf
}

pub fn id_of(prefix: &str, v: &Value, len: usize) -> String {
    short_id(prefix, &canonical(v), len)
}
