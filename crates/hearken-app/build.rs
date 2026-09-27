//! Link settings for this crate's own test binaries.
//!
//! Prism is linked statically through hearken-speech, and its Windows
//! screen-reader client DLLs must be delay-loaded, with Prism's failure hook
//! forced into the link, or a binary will not start (or will die) on a
//! machine missing one of them. Link arguments only apply to the final link
//! of the package that emits them, so every package with binaries repeats
//! this.

fn main() {
    delay_load_prism_backends();
}

fn delay_load_prism_backends() {
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
    println!("cargo:rustc-link-arg=/INCLUDE:__pfnDliFailureHook2");
}
