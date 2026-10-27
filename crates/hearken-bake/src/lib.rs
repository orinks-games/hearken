//! Bakes a 32-byte key into a build without writing it whole.
//!
//! `hearken release` hands a game's build its keys through environment
//! variables: the pack key (`HEARKEN_PACK_KEY`, new for every release) and
//! the seal key (`HEARKEN_SEAL_KEY`, the game's own, kept across releases).
//! The build scripts of `hearken-pack` and `hearken-seal` turn each into
//! Rust code with this crate, and include that code.
//!
//! The key is split into three random-looking shares, kept in separate
//! statics and put back together only when it is used, so searching the
//! program's bytes for the key, or its hex digits, finds nothing. The
//! shares change with every build. This is a build-dependency only: no game
//! ships it.

/// Code that rebuilds `key` at run time from three shares: word `i` of the
/// key is `a[i] ^ b[i].rotate_left(r[i]) ^ c[(i + s) % 4]`.
///
/// The code is one block expression of type `Option<[u8; 32]>`, for a build
/// script to write out and a crate to `include!` as a function's body.
pub fn scattered(key: &[u8; 32]) -> String {
    let words: Vec<u64> = key
        .chunks(8)
        .map(|w| u64::from_le_bytes(w.try_into().unwrap()))
        .collect();
    let a: Vec<u64> = (0..4).map(|_| random()).collect();
    let b: Vec<u64> = (0..4).map(|_| random()).collect();
    let r: Vec<u32> = (0..4).map(|_| (random() % 63 + 1) as u32).collect();
    let s = (random() % 4) as usize;
    let mut c = [0u64; 4];
    for i in 0..4 {
        c[(i + s) % 4] = words[i] ^ a[i] ^ b[i].rotate_left(r[i]);
    }
    format!(
        "{{
    static A: [u64; 4] = {a:?};
    static B: [u64; 4] = {b:?};
    static C: [u64; 4] = {c:?};
    const R: [u32; 4] = {r:?};
    let (a, b, c) = (
        std::hint::black_box(&A),
        std::hint::black_box(&B),
        std::hint::black_box(&C),
    );
    let mut key = [0u8; 32];
    for i in 0..4 {{
        let word = a[i] ^ b[i].rotate_left(R[i]) ^ c[(i + {s}) % 4];
        key[i * 8..i * 8 + 8].copy_from_slice(&word.to_le_bytes());
    }}
    Some(key)
}}"
    )
}

/// 64 bits from the operating system's random seed for hash maps: enough
/// for shares that only need to differ between builds.
fn random() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(0x9e37_79b9_7f4a_7c15);
    hasher.finish()
}

/// A key from 64 hex digits, surrounding whitespace allowed.
pub fn parse_hex(hex: &str) -> Option<[u8; 32]> {
    let hex = hex.trim();
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    let mut key = [0u8; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(key)
}

/// The code a build script writes for the key in the variable `var`:
/// `None` when it is unset or empty, else the scattered key. Panics, and so
/// fails the build, when the variable holds anything but 64 hex digits.
pub fn from_env(var: &str) -> String {
    println!("cargo:rerun-if-env-changed={var}");
    match std::env::var(var).ok().filter(|k| !k.trim().is_empty()) {
        None => "None".to_string(),
        Some(hex) => {
            let key = parse_hex(&hex)
                .unwrap_or_else(|| panic!("{var} must be 64 hexadecimal digits (32 bytes)."));
            scattered(&key)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_64_hex_digits_and_nothing_else() {
        let hex = "00112233445566778899aabbccddeeff00112233445566778899AABBCCDDEEFF";
        let key = parse_hex(&format!("  {hex}\n")).unwrap();
        assert_eq!(key[1], 0x11);
        assert_eq!(key[31], 0xff);
        assert_eq!(parse_hex(&hex[1..]), None);
        assert_eq!(parse_hex(&hex.replace('0', "g")), None);
        assert_eq!(parse_hex(""), None);
    }

    #[test]
    fn shares_differ_between_calls_and_never_hold_the_key() {
        let key = [0x5au8; 32];
        let one = scattered(&key);
        let two = scattered(&key);
        assert_ne!(one, two);
        let word = u64::from_le_bytes([0x5a; 8]).to_string();
        assert!(!one.contains(&word));
    }
}
