//! Delay-load the screen reader DLLs behind Prism's Windows backends.
//!
//! Without this every screen reader's client DLL becomes a hard import and
//! the game will not start on a machine missing any of them. A link flag
//! only applies to the final link, which is this crate's, so every game
//! needs this build script.

fn main() {
    println!("cargo:rerun-if-env-changed=DEP_HEARKEN_SPEECH_DELAY_LOAD_DLLS");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }
    let Ok(dlls) = std::env::var("DEP_HEARKEN_SPEECH_DELAY_LOAD_DLLS") else {
        return;
    };
    for dll in dlls.split(';').filter(|dll| !dll.is_empty()) {
        println!("cargo:rustc-link-arg=/DELAYLOAD:{dll}");
    }
    // Prism's delay-load failure hook must be forced into the link, or a
    // machine missing any screen reader's client DLL crashes the game at
    // start.
    println!("cargo:rustc-link-arg=/INCLUDE:__pfnDliFailureHook2");
}
