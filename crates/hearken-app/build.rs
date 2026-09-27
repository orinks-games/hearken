//! Link settings for this crate's own test binaries, and staging the
//! vendored SDL2 import library and DLL.
//!
//! Prism is linked statically through hearken-speech, which works out the
//! link arguments it needs (see its build.rs); link arguments only apply to
//! the final link of the package that emits them, so this crate passes them
//! on for its own test binaries, as every game does.
//!
//! Put the vendored SDL2 import library on the link path and stage its DLL
//! beside the build output. macOS and Linux compile SDL2 in statically
//! instead (see Cargo.toml), so they have nothing vendored.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    link_prism();
    stage_vendored_sdl2();
}

/// Prism's link arguments, worked out by hearken-speech, for this crate's
/// own test binaries.
fn link_prism() {
    println!("cargo:rerun-if-env-changed=DEP_HEARKEN_SPEECH_LINK_ARGS");
    let args = std::env::var("DEP_HEARKEN_SPEECH_LINK_ARGS").unwrap_or_default();
    for arg in args.split(';').filter(|arg| !arg.is_empty()) {
        println!("cargo:rustc-link-arg={arg}");
    }
}

fn stage_vendored_sdl2() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.ancestors().nth(2).unwrap().to_path_buf();
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let dir = root
        .join("vendor")
        .join("sdl2")
        .join(format!("{os}-{arch}"));
    println!("cargo:rerun-if-changed={}", dir.display());
    if !dir.is_dir() {
        // macOS and Linux are not vendored on purpose: SDL2 is compiled in
        // statically there (see Cargo.toml), so there is nothing missing and
        // nothing to warn about.
        if os != "macos" && os != "linux" {
            println!("cargo:warning=hearken-app: no vendored SDL2 for {os}-{arch} under vendor/sdl2; expecting a system SDL2");
        }
        return;
    }
    println!("cargo:rustc-link-search=native={}", dir.display());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    if let Some(profile) = out.ancestors().nth(3) {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = entry.path();
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            if matches!(ext, "dll" | "so" | "dylib") {
                let _ = fs::copy(&p, profile.join(p.file_name().unwrap()));
            }
        }
    }
}
