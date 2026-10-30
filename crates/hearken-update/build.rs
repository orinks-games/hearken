//! Bakes the game's update identity into the build.
//!
//! `hearken release` builds a game with its public update key in
//! `HEARKEN_UPDATE_KEY` (`ed25519:` and 64 hex digits), and the game's
//! package, version and channel beside it. Every other build has no key, and
//! a build with no key never updates itself. The key is public, so unlike
//! the pack and seal keys it is written out whole.

use std::fmt::Write;
use std::path::PathBuf;

const VARS: [&str; 4] = [
    "HEARKEN_UPDATE_KEY",
    "HEARKEN_UPDATE_GAME",
    "HEARKEN_UPDATE_VERSION",
    "HEARKEN_UPDATE_CHANNEL",
];

fn var(name: &str) -> String {
    println!("cargo:rerun-if-env-changed={name}");
    std::env::var(name).unwrap_or_default().trim().to_string()
}

fn parse_key(text: &str) -> Option<[u8; 32]> {
    let hex = text.strip_prefix("ed25519:").unwrap_or(text);
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    let mut key = [0u8; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(key)
}

/// Letters, digits and `-`, `_`, `.`, `+`: what a package name, a version
/// and a channel are made of.
fn plain(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'))
}

fn main() {
    let [key, game, version, channel] = VARS.map(var);
    let mut code = String::new();
    if key.is_empty() {
        code.push_str("pub const KEY: Option<[u8; 32]> = None;\n");
        code.push_str("pub const GAME: &str = \"\";\npub const VERSION: &str = \"\";\n");
        code.push_str("pub const CHANNEL: &str = \"\";\n");
    } else {
        let bytes = parse_key(&key)
            .unwrap_or_else(|| panic!("HEARKEN_UPDATE_KEY must be ed25519: and 64 hex digits."));
        for (name, value) in [
            ("HEARKEN_UPDATE_GAME", &game),
            ("HEARKEN_UPDATE_VERSION", &version),
            ("HEARKEN_UPDATE_CHANNEL", &channel),
        ] {
            if !plain(value) {
                panic!("{name} must be set beside HEARKEN_UPDATE_KEY, in letters, digits, - _ . +");
            }
        }
        let _ = writeln!(code, "pub const KEY: Option<[u8; 32]> = Some({bytes:?});");
        let _ = writeln!(code, "pub const GAME: &str = {game:?};");
        let _ = writeln!(code, "pub const VERSION: &str = {version:?};");
        let _ = writeln!(code, "pub const CHANNEL: &str = {channel:?};");
    }
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(out.join("baked.rs"), code).expect("write baked.rs");
}
