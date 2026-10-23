//! Bakes the release's pack key into the build.
//!
//! `hearken release` makes a new random key for every release, packs the
//! sounds with it and builds the game with it in `HEARKEN_PACK_KEY` (64 hex
//! digits). Any other build has no key and reads loose sound files.

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
            format!("Some({bytes:?})")
        }
    };
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(out.join("pack_key.rs"), code).expect("write pack_key.rs");
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
