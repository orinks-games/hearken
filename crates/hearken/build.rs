//! Pass Prism's link arguments on to the game.
//!
//! hearken-speech publishes them as `DEP_HEARKEN_SPEECH_LINK_ARGS`, but
//! Cargo hands that to its direct dependents only, and a game depends on
//! `hearken` alone. Republishing them under this crate's `links` name gives
//! the game's build.rs `DEP_HEARKEN_LINK_ARGS`.

fn main() {
    println!("cargo:rerun-if-env-changed=DEP_HEARKEN_SPEECH_LINK_ARGS");
    let args = std::env::var("DEP_HEARKEN_SPEECH_LINK_ARGS").unwrap_or_default();
    println!("cargo:link_args={args}");
}
