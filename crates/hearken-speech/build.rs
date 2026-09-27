//! Work out the link arguments Prism needs, pass them on to games, apply
//! them to this crate's own binaries, and link Prism's system libraries on
//! macOS and Linux.
//!
//! On MSVC Prism's Windows screen-reader client DLLs must be delay-loaded,
//! or a binary will not start on a machine missing one of them, and Prism's
//! delay-load failure hook must be forced into the link, or a missing DLL
//! crashes the game instead of Prism reporting that backend unavailable.
//! A link argument only takes effect on the final link, which a library
//! cannot do for a game, so this crate publishes the arguments as
//! `DEP_HEARKEN_SPEECH_LINK_ARGS` (Cargo hands a `links` crate's metadata
//! to its direct dependents only) and each game's build.rs passes them on.
//! This crate's own tests, benches and examples ARE a final link, so the
//! same arguments are applied directly too; `rustc-link-arg` from a build
//! script never reaches a dependent's binaries.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=DEP_PRISMER_DELAY_LOAD_DLLS");
    let args = prism_link_args(&env::var("DEP_PRISMER_DELAY_LOAD_DLLS").unwrap_or_default());
    println!("cargo:link_args={}", args.join(";"));
    for arg in &args {
        println!("cargo:rustc-link-arg={arg}");
    }
    link_prism_system_libraries();
}

/// What a binary linking Prism needs on MSVC: every screen-reader DLL
/// delay-loaded, and Prism's delay-load failure hook kept.
///
/// The hook (prism-sys's source/delayimp.cpp) substitutes stub functions
/// when a screen-reader client DLL is missing, instead of MSVC's default of
/// raising an unhandled SEH exception (0xC06D007E) the first time a
/// delay-loaded call fails. Nothing else references that translation
/// unit's symbols, so the static-archive linker drops it unless forced in
/// -- the same problem Prism's own PrismBackends.cmake solves for backend
/// registrars with `/include:`. Nothing is needed off MSVC.
fn prism_link_args(dlls: &str) -> Vec<String> {
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return Vec::new();
    }
    let mut args: Vec<String> = dlls
        .split(';')
        .filter(|dll| !dll.is_empty())
        .map(|dll| format!("/DELAYLOAD:{dll}"))
        .collect();
    args.push("/INCLUDE:__pfnDliFailureHook2".to_string());
    args
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
