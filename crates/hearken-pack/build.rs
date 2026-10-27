//! Bakes the release's pack key into the build.
//!
//! `hearken release` makes a new random key for every release, packs the
//! sounds with it and builds the game with it in `HEARKEN_PACK_KEY` (64 hex
//! digits). Any other build has no key and reads loose sound files.
//!
//! The key is never written into the program whole: `hearken-bake` splits
//! it into shares that change with every build.

use std::path::PathBuf;

fn main() {
    let code = hearken_bake::from_env("HEARKEN_PACK_KEY");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(out.join("pack_key.rs"), code).expect("write pack_key.rs");
}
