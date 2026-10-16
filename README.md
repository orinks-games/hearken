# Hearken

Rust crates for audio-first, screen-reader-first games, extracted from
Freight Fate and Lucky Break Casino. Its first game is Freight Fate:
Locomotive. Work in progress: this is milestone 1
of `docs/superpowers/specs/2026-09-26-hearken-engine-design.md`.

## What is here

- `hearken`: the one dependency for a game; it re-exports the rest.
- `hearken-app`: the window, the screen stack, spoken menus, settings,
  help, pause, confirm and text-entry screens, and a headless harness for
  tests. Closing a screen can leave the one beneath silent
  (`ctx.pop_quiet()`), hand it a result to speak before its current row
  instead of its whole announcement (`ctx.pop_with("Route deleted.")`), or
  go back several screens to a named one (`ctx.pop_to("routes")`, with
  `pop_to_quiet` and `pop_to_with`). `ConfirmScreen` speaks its question
  before its options, Escape picks the cancel option, and an option marked
  `ends_game` quits without the screen beneath speaking. A menu row, and
  a Settings row through `SettingsRow::value_first`, can read back only its
  new value on Left and Right. A text field can go without a label
  (`TextEntryScreen::unlabelled`) and take the game's own instructions.
- `hearken-speech`: screen reader, braille and text-to-speech output
  through Prism.
- `hearken-audio`: a pool of positioned sounds, buses and live voices,
  played through Kira, which compiles into the game. A looping clip can
  repeat a region inside its file and play out the rest when released, so
  a horn is one file with its start, loop and release: the loop points come
  from a WAV's `smpl` chunk, `LOOPSTART` and `LOOPEND` (or `LOOPLENGTH`)
  tags in an Ogg, Opus or FLAC file, or `Audio::start_loop_region`.
- `hearken-synth`: fundsp recipes for live voices and the default sound for
  every category, so a game makes sound with no sound files.
- `hearken-radio`: internet radio, played as a feed: HTTP and ICY stations,
  playlists and live HLS, with now-playing titles.
- `template/`: Hello Hearken, the smallest complete game.

## Features

`hearken`'s `kira` feature, which plays sound, is on by default. These are
opt-in:

- `radio`: internet radio, as `hearken::radio::Stream`.
- `fdk-aac`: Fraunhofer's FDK AAC decodes radio's AAC in place of
  Symphonia's, adding HE-AAC, which Symphonia's does not decode. It turns on
  `radio`. Fraunhofer's licence permits using FDK AAC only under AAC patent
  licences the game holds, and grants none itself: a game without them is
  not licensed to use it at all.

## Building

1. Install Rust with rustup; `rust-toolchain.toml` picks the version.
2. Install CMake 3.24 or newer and a C++23 compiler (Visual Studio 2022 on
   Windows). Prism and libopus, the Opus decoder, are compiled from source
   the first time. To skip Prism's build, set `PRISM_LIB_DIR` to a folder
   holding a prebuilt Prism library.
3. On Linux, install ALSA's development files (`libasound2-dev` on Debian
   and Ubuntu), which Kira's sound output builds against, and
   speech-dispatcher's (`libspeechd-dev`): Prism will not build without at
   least one Linux speech backend.
4. `cargo run -p hello-hearken` starts the template game. Turn your screen
   reader on first; without one, Windows' SAPI voices speak instead.

Every game needs a `build.rs` like `template/build.rs`. It passes on the link
arguments hearken-speech works out for Prism: the screen reader DLLs
delay-loaded, so the game still starts on a machine that lacks some of them,
and Prism's delay-load failure hook kept, so a missing DLL is reported
unavailable instead of crashing the game. The game must list `hearken` as a
direct dependency, which is all it needs: Cargo hands a build script those
arguments only from a direct dependency, and `hearken` passes them on.

## Licence

MIT or Apache-2.0, at your option. Prism's screen reader clients keep their
own licences. Two decoders compile into every game with sound, and its
credits or licence notes carry what they ask:

- Symphonia (MPL-2.0): where its source is, when shipped unchanged.
- libopus (BSD-3-Clause): its copyright notice and licence text, from
  [opus-codec.org/license](https://opus-codec.org/license/). Xiph's,
  Broadcom's and Microsoft's Opus patents are licensed royalty-free; Vectis
  IP's Opus pool covers hardware devices, not software.

The opt-in features compile in more, all of it into the game, with no DLLs:

- `radio`: `ureq`, `icy-metadata` and `url` (each MIT or Apache-2.0),
  `m3u8-rs` and `mpeg2ts` (MIT), and Symphonia's AAC decoder (MPL-2.0, as
  the rest of Symphonia). `ureq`'s TLS is rustls, which is built on `ring`
  (Apache-2.0 and ISC) and brings in `rustls-webpki` and `untrusted` (ISC).
  `url` brings in ICU's Unicode data through `idna` (Unicode-3.0).
- `fdk-aac`: `symphonia-adapter-fdk-aac` ((MIT or Apache-2.0) and MPL-2.0),
  the `fdk-aac` and `fdk-aac-sys` crates (MIT), and Fraunhofer's FDK AAC
  library, under its own licence. That licence asks the game to keep its
  complete text with its credits or licence notes and to make the FDK AAC
  source available free of charge to anyone who gets the game. It permits
  use only under AAC patent licences the game holds, and grants none: see
  Features.

These lists name the notable licences, not every crate's. `cargo about`
builds the complete notice for what a game actually compiles in.
