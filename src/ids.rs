pub fn hash16(input: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in input.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", h)
}

pub fn make_id(prefix: &str, parts: &[&str]) -> String {
    format!("{}_{}", prefix, hash16(&parts.join("|")))
}
