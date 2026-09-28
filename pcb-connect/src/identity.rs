
//! Stable, content-addressed identifiers.
//!
//! Identities never depend on insertion order, wall clock time, or row ids, so
//! an export/import round-trip preserves every id. They are derived with
//! SHA-256 over a canonical, length-prefixed encoding and rendered as
//! lowercase hex.

const HEX: &[u8; 16] = b"0123456789abcdef";

/// SHA-256 implementation (FIPS 180-4), kept dependency-free.
#[derive(Clone)]
pub struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    len: usize,
}

impl Sha256 {
    pub fn new() -> Self {
        Sha256 {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c,
                0x1f83d9ab, 0x5be0cd19,
            ],
            buffer: [0u8; 64],
            len: 0,
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.len = self.len.wrapping_add(data.len());
        let mut filled = self.len % 64;
        if filled != 0 {
            let take = data.len().min(64 - filled);
            self.buffer[filled..filled + take].copy_from_slice(&data[..take]);
            data = &data[take..];
            filled += take;
            if filled == 64 {
                let block = self.buffer;
                Self::compress(&mut self.state, &block);
                filled = 0;
            }
        }
        while data.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&data[..64]);
            Self::compress(&mut self.state, &block);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
        }
    }

    pub fn finish(mut self) -> [u8; 32] {
        let bitlen = (self.len as u64).wrapping_mul(8);
        let filled = self.len % 64;
        self.buffer[filled] = 0x80;
        if filled + 1 > 56 {
            for b in &mut self.buffer[filled + 1..] {
                *b = 0;
            }
            let block = self.buffer;
            Self::compress(&mut self.state, &block);
            self.buffer = [0u8; 64];
        } else {
            for b in &mut self.buffer[filled + 1..] {
                *b = 0;
            }
        }
        self.buffer[56..64].copy_from_slice(&bitlen.to_be_bytes());
        let block = self.buffer;
        Self::compress(&mut self.state, &block);
        let mut out = [0u8; 32];
        for (i, word) in self.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    pub fn hex(data: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(data);
        let digest = h.finish();
        let mut s = String::with_capacity(64);
        for b in digest {
            s.push(HEX[(b >> 4) as usize] as char);
            s.push(HEX[(b & 0xf) as usize] as char);
        }
        s
    }

    fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

/// Length-prefixed feed, so `("ab","c")` and `("a","bc")` cannot collide.
fn feed_str(h: &mut Sha256, s: &str) {
    h.update(&(s.len() as u64).to_be_bytes());
    h.update(s.as_bytes());
    h.update(&[0]);
}

/// Stable identity for an immutable source material:
/// `sha256("pcb-material-v1"|kind|filename|content)`.
pub fn material_id(kind: &str, filename: &str, content: &str) -> String {
    let mut h = Sha256::new();
    feed_str(&mut h, "pcb-material-v1");
    feed_str(&mut h, kind);
    feed_str(&mut h, filename);
    feed_str(&mut h, content);
    format!("mat_{}", Sha256::hex(&h.finish()))
}

/// Stable identity for a project seed: content only, so the same case pack
/// creates the same project id.
pub fn project_id(seed_key: &str) -> String {
    let mut h = Sha256::new();
    feed_str(&mut h, "pcb-project-v1");
    feed_str(&mut h, seed_key);
    format!("prj_{}", Sha256::hex(&h.finish()))
}

/// Stable identity for an event, derived from its branch, predecessor and
/// canonical payload. Replaying the same decisions reproduces event ids.
pub fn event_id(branch: &str, seq: i64, parent_id: &str, payload_json: &str) -> String {
    let mut h = Sha256::new();
    feed_str(&mut h, "pcb-event-v1");
    feed_str(&mut h, branch);
    h.update(&(seq as u64).to_be_bytes());
    feed_str(&mut h, parent_id);
    feed_str(&mut h, payload_json);
    format!("evt_{}", Sha256::hex(&h.finish()))
}

/// Content identity for an immutable computed result:
/// `sha256("pcb-result-v2"|canonical json)`.
pub fn result_id(canonical_json: &str) -> String {
    let mut h = Sha256::new();
    feed_str(&mut h, "pcb-result-v2");
    feed_str(&mut h, canonical_json);
    format!("res_{}", Sha256::hex(&h.finish()))
}

/// Canonical JSON: recursively sort object keys. Numbers are emitted by
/// serde_json as compact decimal strings (no f64 is ever used for geometry,
/// see [`crate::model::geom::Point`]).
pub fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut parts = Vec::with_capacity(keys.size_hint().0);
            for key in keys {
                parts.push(format!("{}:{}", json_string(key), canonical_json(&map[key])));
            }
            format!("{{{}}}", parts.join(","))
        }
        other => other.to_string(),
    }
}

fn json_string(s: &str) -> String {
    serde_json::Value::String(s.to_string()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FIPS 180-2 example vector.
    #[test]
    fn sha256_known_answer() {
        let mut h = Sha256::new();
        h.update(b"abc");
        assert_eq!(
            Sha256::hex(&h.finish()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn length_prefix_prevents_collision() {
        let mut a = Sha256::new();
        feed_str(&mut a, "ab");
        feed_str(&mut a, "c");
        let mut b = Sha256::new();
        feed_str(&mut b, "a");
        feed_str(&mut b, "bc");
        assert_ne!(a.finish(), b.finish());
    }
}
