# Hearken

Rust crates for audio-first, screen-reader-first games, extracted from
Freight Fate and Lucky Break Casino. Work in progress: this is milestone 1
of `docs/superpowers/specs/2026-09-26-hearken-engine-design.md`.

## What is here

- `hearken-app`: the window, the screen stack, spoken menus, settings,
  help and pause screens, and a headless harness for tests.
- `hearken-speech`: screen reader, braille and text-to-speech output
  through Prism.
- `hearken-audio`: a pool of positioned sounds, buses and live voices,
  played through Kira, which compiles into the game.
- `hearken-synth`: fundsp recipes for live voices and the default sound for
  every category, so a game makes sound with no sound files.
- `template/`: Hello Hearken, the smallest complete game.

## Building

1. Install Rust with rustup; `rust-toolchain.toml` picks the version.
2. Install CMake 3.24 or newer and a C++23 compiler (Visual Studio 2022 on
   Windows). Prism and libopus, the Opus decoder, are compiled from source
   the first time. To skip Prism's build, set `PRISM_LIB_DIR` to a folder
   holding a prebuilt Prism library.
3. On Linux, install ALSA's development files (`libasound2-dev` on Debian
   and Ubuntu), which Kira's sound output builds against.
4. `cargo run -p hello-hearken` starts the template game. Turn your screen
   reader on first; without one, Windows' SAPI voices speak instead.

Every game needs a `build.rs` like `template/build.rs`. It passes on the link
arguments hearken-speech works out for Prism: the screen reader DLLs
delay-loaded, so the game still starts on a machine that lacks some of them,
and Prism's delay-load failure hook kept, so a missing DLL is reported
unavailable instead of crashing the game. The game must list `hearken-speech`
as a direct dependency, not only through `hearken-app`: Cargo hands a build
script those arguments only from a direct dependency.

## Licence

MIT or Apache-2.0, at your option. Prism's screen reader clients keep their
own licences. Two decoders compile into every game with sound, and its
credits or licence notes carry what they ask:

- Symphonia (MPL-2.0): where its source is, when shipped unchanged.
- libopus (BSD-3-Clause): its copyright notice and licence text, from
  [opus-codec.org/license](https://opus-codec.org/license/). Xiph's,
  Broadcom's and Microsoft's Opus patents are licensed royalty-free; Vectis
  IP's Opus pool covers hardware devices, not software.
