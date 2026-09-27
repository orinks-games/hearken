# Hearken

Rust crates for audio-first, screen-reader-first games, extracted from
Freight Fate and Lucky Break Casino. Work in progress: this is milestone 1
of `docs/superpowers/specs/2026-09-26-hearken-engine-design.md`.

## What is here

- `hearken-app`: the window, the screen stack, spoken menus, settings,
  help and pause screens, and a headless harness for tests.
- `hearken-speech`: screen reader, braille and text-to-speech output
  through Prism.
- `hearken-bass-sys`: bindings to the BASS audio library, loaded at run
  time, including streams fed by a Rust closure.
- `template/`: Hello Hearken, the smallest complete game.

## Building

1. Install Rust with rustup; `rust-toolchain.toml` picks the version.
2. Install CMake 3.24 or newer and a C++23 compiler (Visual Studio 2022 on
   Windows). Prism is compiled from source the first time. To skip that,
   set `PRISM_LIB_DIR` to a folder holding a prebuilt Prism library.
3. Install Python 3, for `tools/fetch_bass.py`.
4. `python tools/fetch_bass.py` downloads BASS. BASS is free for
   non-commercial use; a commercial game needs its own licence from
   un4seen. Hearken never bundles it.
5. `cargo run -p hello-hearken` starts the template game. Turn your screen
   reader on first; without one, Windows' SAPI voices speak instead.

Every game needs a `build.rs` like `template/build.rs`, so the screen reader
DLLs are delay-loaded and the game still starts on machines that lack some
of them; it must also force Prism's delay-load failure hook into the link
(`/INCLUDE:__pfnDliFailureHook2`), or a machine missing any screen reader's
client DLL crashes the game at start instead of Prism reporting it
unavailable. The game must list `hearken-speech` as a direct dependency,
not only through `hearken-app`: Cargo hands a build script the list of
DLLs to delay-load only from a direct dependency.

## Licence

MIT or Apache-2.0, at your option. BASS and Prism's screen reader clients
keep their own licences.
