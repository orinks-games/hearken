//! Publish the link arguments Prism needs to games, and link Prism's system
//! libraries on macOS and Linux.
//!
//! Prism 0.1.4 loads the Windows screen-reader client DLLs at run time
//! (LoadLibrary) instead of importing them, so a missing screen reader no
//! longer needs delay-loading or a delay-load failure hook, and Prism needs
//! no link arguments on any platform today. The channel stays: a link
//! argument only takes effect on the final link, which a library cannot do
//! for a game, so this crate publishes them as `DEP_HEARKEN_SPEECH_LINK_ARGS`
//! (Cargo hands a `links` crate's metadata to its direct dependents only),
//! the `hearken` facade passes them on, and each game's build.rs applies
//! them. An argument added here reaches every game with no game changing.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let args = prism_link_args();
    println!("cargo:link_args={}", args.join(";"));
    // This crate's own tests, benches and examples are a final link too;
    // `rustc-link-arg` from a build script never reaches a dependent's.
    for arg in &args {
        println!("cargo:rustc-link-arg={arg}");
    }
    link_prism_system_libraries();
}

/// The arguments every binary linking Prism needs: none since Prism 0.1.4.
fn prism_link_args() -> Vec<String> {
    Vec::new()
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
        // object, and a static link keeps an object only when something
        // references it; without that Prism starts with an empty registry.
        // Since 0.1.4 prism.cpp references every backend it built (MSVC by
        // `/include:`), so the plain link keeps them too; the whole archive
        // stays as a second guarantee on GCC and Clang (ld64 `-force_load`
        // on macOS). `prism_registers_this_platforms_backends` in
        // tests/speech_live.rs checks the result on each CI platform.
        // `-bundle` defers this to the final link, where prism-sys's search
        // path finds libprism.a.
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
