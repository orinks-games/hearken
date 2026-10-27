//! A game's data files, sealed against editing in a release.
//!
//! A released game keeps its players' records and progress in files on
//! their computer, where anyone with a text editor could otherwise change
//! them. A [`Vault`] reads and writes those files through a seal: in a
//! release built with the game's secret, every file is encrypted and
//! authenticated (XChaCha20-Poly1305) with its own name bound in, and a
//! sealed ledger in the same folder remembers how many times each one has
//! been written. A file that was edited, swapped for another, restored
//! from an older copy, deleted, or replaced by a hand-written plain one
//! reads as [`Opened::Tampered`], never as good data.
//!
//! The engine only reports what it found. Each game decides what to do
//! with a tampered file, and with a plain file from an older version
//! ([`Opened::Legacy`]).
//!
//! In every other build (a developer build, `cargo test`, `hearken play`)
//! there is no key, and the vault reads and writes plain files, so a
//! developer can look at and change them. Such a build reports a sealed
//! file as [`Opened::Locked`] and will not overwrite it.
//!
//! What this cannot do: the key is inside the program the player runs.
//! Someone willing to take that program apart can recover it and forge any
//! file. Restoring a whole data folder, ledger included, from one earlier
//! moment also cannot be told apart from never having played since. This
//! raises cheating from "open a text editor" to "reverse engineer the
//! program"; no offline scheme can do more.
//!
//! The file format:
//!
//! ```text
//! offset size
//! 0      4    "HKSL"
//! 4      1    version = 1
//! 5      3    reserved, zero
//! 8      8    generation (u64 LE), 1 for the first write of a name in this folder
//! 16     24   nonce (random)
//! 40     ..   ciphertext of the file's bytes, then the 16-byte Poly1305 tag
//! ```
//!
//! The key is `HKDF-SHA256(salt = "hearken-seal", ikm = secret, info =
//! "files v1")`, and the associated data is the 40-byte header, then
//! `"hearken-seal/v1\0"`, then the file's name.

mod format;
mod key;
mod ledger;
mod name;
mod vault;

pub use key::Key;
pub use vault::{Opened, Tamper, Vault};

/// The build variable `hearken release` passes the game's secret in.
pub const KEY_VAR: &str = "HEARKEN_SEAL_KEY";

/// The ledger's file name, reserved in every data folder.
pub const LEDGER: &str = "hearken-seal.ledger";

/// The key this build seals with: set by `hearken release` through
/// `HEARKEN_SEAL_KEY`, and `None` in every other build. Setting the
/// variable when the game runs does nothing: the key exists only at build
/// time.
pub fn build_key() -> Option<Key> {
    let bytes: Option<[u8; 32]> = include!(concat!(env!("OUT_DIR"), "/seal_key.rs"));
    bytes.map(Key::from_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_build_key_is_the_one_it_was_built_with() {
        // Meaningful when the tests are built with HEARKEN_SEAL_KEY set.
        match std::env::var(KEY_VAR).ok().filter(|k| !k.trim().is_empty()) {
            Some(hex) => assert_eq!(
                build_key().map(|k| k.to_hex()),
                Some(hex.trim().to_ascii_lowercase())
            ),
            None => assert_eq!(build_key(), None),
        }
    }
}
