//! Pass Prism's delay-load list on to games, delay-load it for this crate's
//! own binaries, and link Prism's system libraries on macOS and Linux.
//!
//! Cargo hands a `links` crate's metadata to its direct dependents only.
//! prismer publishes the Windows screen-reader DLLs that must be
//! delay-loaded; a game depends on this crate, not on prismer, so the list
//! is re-published here as `DEP_HEARKEN_SPEECH_DELAY_LOAD_DLLS`. The game's
//! own build.rs turns it into `/DELAYLOAD` link arguments: a link flag only
//! takes effect on the final link, which a library cannot do for a game.
//! This crate's own tests, benches and examples ARE a final link, so the
//! same flags are applied directly below too (`rustc-link-arg` from a
//! build script only ever reaches its own package's binaries, never a
//! dependent's, so this has no effect on a game using the passthrough
//! above).

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=DEP_PRISMER_DELAY_LOAD_DLLS");
    if let Ok(dlls) = env::var("DEP_PRISMER_DELAY_LOAD_DLLS") {
        println!("cargo:delay_load_dlls={dlls}");
        delay_load_for_own_binaries(&dlls);
    }
    link_prism_system_libraries();
}

/// Delay-load the DLLs behind Prism's Windows backends (NVDA, JAWS, ...) for
/// this crate's own tests, benches and examples.
///
/// Without this, Prism's static library leaves them as hard imports, and a
/// test binary refuses to start on a machine missing any one of them.
fn delay_load_for_own_binaries(dlls: &str) {
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }
    for dll in dlls.split(';').filter(|dll| !dll.is_empty()) {
        println!("cargo:rustc-link-arg=/DELAYLOAD:{dll}");
    }
}

/// Link what Prism's macOS and Linux backends import.
///
/// prism-sys names these for Windows only, and a static library leaves them
/// to the final link. macOS: the frameworks its CMake links (AVSpeech,
/// VoiceOver, power management). Linux: whichever of speech-dispatcher and
/// glibmm and giomm (Orca) pkg-config finds -- the same test Prism's CMake used to
/// decide whether to build those backends at all.
fn link_prism_system_libraries() {
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os == "linux" || os == "macos" {
        // Each backend registers itself from a static initializer in its own
        // object, which nothing references, so a plain static link drops
        // every one and Prism starts with an empty registry. Prism anchors
        // them for MSVC only (`/include:`); GCC and Clang need the whole
        // archive. `-bundle` defers this to the final link, where prism-sys's
        // search path finds libprism.a.
        println!("cargo:rustc-link-lib=static:+whole-archive,-bundle=prism");
    }
    match env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("macos") => {
            for framework in [
                "Foundation",
                "AVFoundation",
                "AppKit",
                "IOKit",
                "CoreFoundation",
            ] {
                println!("cargo:rustc-link-lib=framework={framework}");
            }
            println!("cargo:rustc-link-lib=objc");
        }
        Ok("linux") => {
            link_libstdcxx_statically();
            for module in ["speech-dispatcher", "glibmm-2.68", "giomm-2.68"] {
                let Ok(out) = std::process::Command::new("pkg-config")
                    .args(["--libs", module])
                    .output()
                else {
                    continue;
                };
                if !out.status.success() {
                    continue;
                }
                for flag in String::from_utf8_lossy(&out.stdout).split_whitespace() {
                    if let Some(dir) = flag.strip_prefix("-L") {
                        println!("cargo:rustc-link-search=native={dir}");
                    } else if let Some(lib) = flag.strip_prefix("-l") {
                        println!("cargo:rustc-link-lib={lib}");
                    }
                }
            }
        }
        _ => {}
    }
}

/// Make prism-sys's `-lstdc++` resolve to the compiler's static archive.
///
/// Prism needs a newer libstdc++ than the oldest distribution the Linux
/// build supports ships, so it is linked in rather than required of the
/// player's system. prism-sys asks for plain `stdc++`; the linker takes the
/// first search directory holding any libstdc++, and this one holds only the
/// archive.
fn link_libstdcxx_statically() {
    println!("cargo:rerun-if-env-changed=CXX");
    let cxx = env::var("CXX").unwrap_or_else(|_| "c++".to_string());
    let Ok(out) = std::process::Command::new(&cxx)
        .arg("-print-file-name=libstdc++.a")
        .output()
    else {
        return;
    };
    // gcc echoes the bare name back when it has no such file.
    let archive = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    if !archive.is_absolute() {
        println!(
            "cargo:warning=hearken-speech: {cxx} has no libstdc++.a; linking libstdc++ dynamically"
        );
        return;
    }
    let dir = PathBuf::from(env::var("OUT_DIR").unwrap()).join("static-libstdcxx");
    if fs::create_dir_all(&dir).is_ok() && fs::copy(&archive, dir.join("libstdc++.a")).is_ok() {
        println!("cargo:rustc-link-search=native={}", dir.display());
    }
}
