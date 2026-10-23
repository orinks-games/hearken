//! Bakes the release's pack key into the build.
//!
//! `hearken release` makes a new random key for every release, packs the
//! sounds with it and builds the game with it in `HEARKEN_PACK_KEY` (64 hex
//! digits). Any other build has no key and reads loose sound files.
//!
//! The key is never written into the program whole. It is split into three
//! random-looking shares, kept in separate statics and put back together
//! only when the pack is opened, so searching the program's bytes for a
//! 32-byte key finds nothing. The shares change with every build.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=HEARKEN_PACK_KEY");
    let key = std::env::var("HEARKEN_PACK_KEY")
        .ok()
        .filter(|k| !k.is_empty());
    let code = match key {
        None => "None".to_string(),
        Some(hex) => {
            let bytes = parse(&hex).unwrap_or_else(|| {
                panic!("HEARKEN_PACK_KEY must be 64 hexadecimal digits (32 bytes).")
            });
            scattered(&bytes)
        }
    };
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(out.join("pack_key.rs"), code).expect("write pack_key.rs");
}

/// Code that rebuilds `key` at run time from three shares: word `i` of the
/// key is `a[i] ^ b[i].rotate_left(r[i]) ^ c[(i + s) % 4]`.
fn scattered(key: &[u8; 32]) -> String {
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

fn parse(hex: &str) -> Option<[u8; 32]> {
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
