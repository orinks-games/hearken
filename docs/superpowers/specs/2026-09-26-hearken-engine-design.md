# Hearken Engine Design

- Date: 2026-09-26
- Status: Draft for owner review
- Repository: `Orinks/hearken` (local only until the owner says to publish)
- Sources: Freight Fate `c5d3ef8d` (2026-09-25), Lucky Break `a101970` (2026-09-23)

## Summary

Hearken is a set of Rust crates for audio-first, screen-reader-first games,
extracted from the two games that already share its design: Freight Fate and
Lucky Break Casino. It gives a new game a window and loop, a screen stack,
menus, key and controller bindings, settings and help screens, speech through
Prism, 3D audio through BASS, synthesized and recorded sound, optional online
services, and a headless test harness an AI assistant can play through.

It exists because the next two games (a watersports arcade game and a
football game) would otherwise copy that shell a third and fourth time. Lucky
Break's copy has already drifted from Freight Fate's.

## Goals

1. One shared implementation of everything that is not a game's own rules.
2. A `cargo generate` template that starts as a playable, accessible shell:
   title menu, settings, key rebinding, help, Learn Sounds, pause.
3. A sound pipeline where every sound works as pure synthesis (prototype
   tier) and improves with real recordings layered on top (shipping tier),
   from whatever sources the game's author chooses.
4. Live procedural sound (engines, water, wind, crowds) driven by game
   parameters, on the same recipes that bake offline sounds.
5. Headless, transcript-backed testing and the agent-server MCP for every
   game, not only Freight Fate.
6. Open source under MIT/Apache-2.0 once Lucky Break runs on it and the
   template builds from a clean clone.

## Non-goals (v1)

- A scripting language. The audience is Rust developers and people building
  with AI coding assistants. Revisit when someone asks.
- A visual editor, a no-code workflow, or 3D graphics.
- Localization. Neither source game has it today.
- macOS and Linux support in CI. v1 targets Windows, like Lucky Break;
  a game that ships elsewhere follows Freight Fate's nightly recipes
  (SDL2 compiled in with `bundled` + `static-link`, never Homebrew's sdl2).
- Moving Freight Fate before Career 1.9 ships.

## Decisions already made

| Decision | Made | Notes |
|---|---|---|
| Name is Hearken; crates `hearken-*` | 2026-09-26 | All names were free on crates.io |
| Audience: Rust developers and AI-assisted creators | 2026-09-26 | No scripting layer in v1 |
| Shipping sound is hybrid; pure synthesis must stand alone | 2026-09-26 | From an A/B spike, below |
| Sound sources are the game author's choice | 2026-09-26 | No assumed provider |
| Speech uses the `prismer` crate | 2026-09-26 | Lucky Break's in-tree `prism` crates are dropped |
| Freight Fate moves only after 1.9 ships | 2026-09-26 | |
| The Python `fate-engine` repo is archived, not reused | 2026-09-26 | It targets the pygame stack Freight Fate left on 2026-08-22 |

### Evidence behind the sound decision

A throwaway spike re-made three Lucky Break sounds and one jet ski loop and
measured each against CC0 recordings (mean third-octave band deviation):

| Sound | Current | New |
|---|---|---|
| Chip click (Faust modal model, modes measured from a recording) | 15.6 dB | 4.4 dB |
| Crowd murmur (fundsp babble: moving formants, consonants, room) | 11.0 dB | 4.2 dB |
| Jet ski ride (fundsp, throttle-driven) | none | 5.7 dB |
| Win jingle (Surge XT through pedalboard) | not measured | not measured |

The owner's verdict after a loudness-matched listen: recording plus synthesis
is best for a shipped game, and pure synthesis must still be enough to
prototype with. All four toolchains (fundsp 0.23, Faust 2.88 to Rust,
pedalboard 0.9, Surge XT 1.3.4) worked on the owner's machine, and rendering
every clip took 0.2 seconds.

## Crates

```
hearken/
  crates/
    hearken-bass-sys   BASS C ABI, loaded at run time
    hearken-speech     prismer, speech worker, two channels, capture, fakes
    hearken-audio      buses, 3D, emitters, beds, music, live voices, null
    hearken-synth      fundsp + Faust bridge, recipe library, layer helpers
    hearken-app        shell, loop, screen stack, menus, input, settings
    hearken-online     updater, sign-in, keyring, cloud saves, presence
    hearken-playtest   headless harness, scenario runner, agent-server MCP
  tools/
    hearken-bake       sound manifest -> sound pack, credits, --check
  template/            cargo-generate starter game
```

Dependency direction is strictly downward: `hearken-app` depends on speech
and audio; audio depends on bass-sys and synth; nothing depends on
`hearken-app` except games, playtest and the template. `hearken-online` is
optional and depends only on `hearken-app`'s service traits.

### hearken-bass-sys

Lucky Break's `bass-sys` (effects and the 3D API) is the base, plus the
tracker-music and HLS-tag bindings Lucky Break dropped and Freight Fate still
uses. The package is `hearken-bass-sys` because `bass-sys` is taken on
crates.io; its library name stays `bass_sys`, so ported code keeps its
imports. Changes:

- Drop the `FREIGHT_FATE_BASS_PATH` legacy variable; the path variable
  becomes `HEARKEN_BASS_PATH`, and a game may set its own name for it.
- Add `BASS_StreamCreate` with a `STREAMPROC` callback, which live voices
  need. Neither game binds it today.
- BASS is never redistributed by the engine. `tools/fetch_bass.py` moves here;
  the README states that a commercial game needs its own BASS licence.

### hearken-speech

Freight Fate's speech stack, which is the more complete of the two:

- `prismer` (crates.io, `static` feature) on its own worker thread, so speech
  never blocks the loop.
- Two channels, never interchangeable. `say` is the menu and screen-reader
  channel. `say_event` is the gameplay channel: priority ladder, anti-backlog
  pacer, audio duck, and it can be silenced or cut. Freight Fate's
  `SpeechSink` (`say`, `say_event`, `stop_main`, `stop_event`, `stop`,
  `poll`) is the interface.
- A capture sink that records a transcript for tests, and fakes.
- Both channels log to one transcript log target.
- Prism's delay-load list passed on to games: `hearken-speech` has
  `links = "hearken_speech"` and re-publishes prismer's list, and each game's
  own `build.rs` (a dozen lines, shipped in the template) turns
  `DEP_HEARKEN_SPEECH_DELAY_LOAD_DLLS` into `/DELAYLOAD` link arguments.
  Linker flags only apply to the final link, which is the game's.
- Toolchain: prismer compiles Prism from source (CMake 3.24+, a C++23
  compiler) unless `PRISM_LIB_DIR` points at a prebuilt library. The template
  README documents both.

The ladder, pacer and duck live in Freight Fate's delivery layer, which is
tied to its context and audio, so they arrive with audio in M2; until then
`say_event` goes straight to the event voice. The pacer's thresholds become
configuration with Freight Fate's values as defaults.

### hearken-audio

Lucky Break's model with Freight Fate's safety:

- A backend trait with BASS and silent (null) implementations. A machine
  without BASS runs silent.
- Buses (a game-extensible set: UI, effects, voice, ambience, music), a
  listener, 3D one-shots, looping emitters, ambience beds, music, occlusion.
  Nothing game-specific in the trait (Freight Fate's `engine_volume` and
  `weather_volume` stay in Freight Fate).
- `missing_clips()` reporting, as Lucky Break has.
- **Live voices.** `audio.start_voice(id, recipe, params, pos, bus)` runs a
  `hearken-synth` recipe on a BASS stream channel with 3D positioning.
  `audio.set_voice_param(id, "throttle", 0.8)` updates a named parameter
  lock-free (fundsp shared values). Under the null backend the voice is not
  rendered. This is what drives a jet ski, a boat, wind, or a football crowd
  that swells with the play.

### hearken-synth

- fundsp for graphs, band-limited oscillators, filters, reverb and noise.
- A Faust bridge: the small set of types Faust's Rust backend expects (the
  spike needed about 50 lines), plus a build step that compiles `.dsp` files
  to Rust when the Faust compiler is present and uses the committed generated
  code when it is not, so games build without Faust installed.
- A recipe is a Rust function from parameters to a fundsp node or a rendered
  buffer. The engine ships the recipes that proved out in the spike: modal
  impacts (chips, glasses, bells), crowd babble, a pulse-train engine through
  fixed resonances (the firing rate moves; resonances never do, and loops
  are never pitch-shifted), water and spray, UI tones.
- Layer helpers: re-cutting a recording into fresh grains, pitch and level
  variation, so a layered sound never repeats exactly.
- A measurement module (third-octave band balance, centroid, crest, decay)
  used by `hearken-bake --check`.

### hearken-app

The shell and everything a menu-driven audio game needs:

- **Shell**: Lucky Break's `Shell` trait (`poll`, `render`, `frame`) with an
  SDL implementation and a headless one. Freight Fate's shell is concrete; it
  gains the trait when it moves.
- **Screens**: one `State` trait, generic over the game.
  ```rust
  pub trait Game: 'static {
      type Event;          // what the game's rules emit (Lucky Break: protocol::Event)
      type Data;           // the game's own services and session
  }
  pub trait State<G: Game> {
      fn name(&self) -> &'static str;
      fn enter(&mut self, ctx: &mut Ctx<G>) {}
      fn exit(&mut self, ctx: &mut Ctx<G>) {}
      fn resume(&mut self, ctx: &mut Ctx<G>) {}
      fn handle_input(&mut self, ctx: &mut Ctx<G>, event: &InputEvent) {}
      fn on_event(&mut self, ctx: &mut Ctx<G>, event: &G::Event) {}
      fn update(&mut self, ctx: &mut Ctx<G>, dt: f64) {}
      fn lines(&self, ctx: &Ctx<G>) -> Vec<String> { Vec::new() }
      fn help(&self) -> String;
  }
  ```
  This is Lucky Break's trait with the event type lifted out. Controller
  input arrives as `InputEvent` like keys do, replacing Freight Fate's
  separate `handle_controller`. Presence hooks move to `hearken-online`.
- **Stack**: Lucky Break's queued `StackOp { Push, Pop, Replace, Reset }`,
  applied between frames, so a state never re-enters itself mid-handler.
- **Context**: `Ctx<G>` holds speech, audio, settings, bindings, the stack
  queue, logging, and `game: G::Data`.
- **Menus**: Lucky Break's `Menu`, `MenuCore`, `MenuItem` and `Label`, which
  are Freight Fate's design with the trucking hooks already removed. Every
  `Menu` is a `State` through a blanket impl, so no macro is needed. F1 reads
  the focused row's help, or the menu's own help when the row has none.
  Freight Fate's text-entry state comes with input in M1b.
- **Input**: Freight Fate's key bindings (with names for rebinding) and
  controller support, plus held-key tracking.
- **Built-in screens**: settings (volumes per bus, speech, verbosity, key
  rebinding), help (F1 on every screen reads that screen's help), Learn
  Sounds (built from the sound manifest's descriptions), pause, a timed
  message screen, and a save notice.
- **Services**: single-instance guard, file logging with a transcript
  target, a data directory named after the game (`<GAME>_DATA_DIR`
  overrides it).

### hearken-online (optional feature)

Freight Fate's online services, generalized: updater with download and apply
(Lucky Break's only checks for a release), device-code sign-in, keyring
token storage, cloud saves, presence heartbeat, Discord presence. The server
address and product identifiers come from the game; nothing points at
orinks.net by default. Everything runs off the loop and talks to it through
bounded channels, with an explicit, idempotent, time-bounded shutdown.

### hearken-playtest

Freight Fate's playtest library and agent server, generalized:

- A headless harness: dummy SDL drivers, capture speech, silent audio, a
  throwaway data directory, so tests never touch a player's settings or
  saves.
- A scenario runner a game registers its own scenarios with (Freight Fate's
  adversarial battery is one such game-side set).
- The agent-server MCP: keys in through the normal input path; both speech
  channels, earcons and cues out. Games add their own tools (Freight Fate's
  `pedal`, `cruise`, `scenario`) through a registration hook. Behind a Cargo
  feature that player builds leave out.

### hearken-bake and the sound manifest

Every sound is one entry, keyed by the name the game plays:

```toml
[sound."sfx/chip_25"]
describe = "Two chips clacked onto felt"   # Learn Sounds reads this
recipe = "chips"                           # a recipe the game registers
params = { count = 2, spread = 0.10 }
seconds = 0.5
reference = "refs/chip_single.wav"         # optional; for --check

[[sound."sfx/chip_25".layer]]
file = "chips/clack_03.wav"                # relative to the layers folder
source = "Freesound 633997"
license = "CC0-1.0"
attribution = "reg7783"
gain_db = -6.0
vary = { pitch_semitones = 1.0 }

[[sound."sfx/crowd_murmur".layer]]
generate = { provider = "elevenlabs", prompt = "brief swell of casino crowd murmur and a laugh" }
```

- **Tiers resolve per sound at bake time.** Layers present means a hybrid;
  none, or none on disk, means the recipe alone. Game code never knows which.
  A project with only recipes has a complete prototype sound set.
- **Sources are the author's choice.** A layer is a file: their own
  recordings, royalty-free packs, CC0 downloads, ElevenLabs output. Sources
  that generate or fetch files are plug-ins behind one trait; the engine
  ships a folder source and ElevenLabs. A failed or unconfigured provider
  (bad key, no credits, offline) leaves that layer out and the sound bakes as
  synthesis, reported per sound, never failing the bake. This is Lucky
  Break's current fallback, moved into the tool.
- **Licensed layers stay out of the repo.** The layers folder can live
  outside the checkout (`[paths] layers = ...` or `HEARKEN_LAYERS_DIR`), the
  way Freight Fate's `sounds-licensed` overlay works today, so a public repo
  ships synthesis and a private build ships the hybrid.
- **Nothing refuses; everything falls back to synthesis.**
  - A sound the game plays with no manifest entry, or an entry with no
    recipe, gets a default synth sound chosen by its key's category
    (`ui/` a tone, `sfx/` an impact, `amb/` a noise bed, `music/` a pad,
    anything else a neutral blip), so a brand-new project is audible before
    anyone writes a recipe. The bake lists which sounds are on defaults.
  - A layer without `source` and `license` is left out of that sound, which
    bakes from its recipe instead, and the bake report names it. Unlabelled
    audio therefore never ships by accident, and the build still succeeds.
  - At run time, a clip missing from the pack plays its category default
    rather than silence, and `missing_clips()` still reports it.
- **Credits are generated.** `hearken-bake` writes the credits file from the
  layers' provenance, so attribution-required packs are credited without
  anyone doing it by hand.
- **Output**: a sound pack (one file of Ogg Vorbis and WAV entries plus an
  index carrying each sound's tier and provenance). During development,
  loose files override the pack.
- **`--check`** measures every recipe that names a reference and prints its
  band deviation. It informs; the owner's listening pass decides.
- **Music and jingles** can come from an optional Python studio tool
  (pedalboard hosting Surge XT or any VST3), rendered to files that enter as
  layers. No game needs Python to build.

## Template

`cargo generate --git https://github.com/Orinks/hearken template` produces a
game that:

- opens a title menu with New Game, Settings, Help, Learn Sounds, Quit;
- has one playable screen that moves a sound around the listener in 3D and
  runs one live voice from a parameter;
- has a `sounds.toml` with five recipes and no layers, and plays category
  defaults for any sound it has not defined yet;
- has the `build.rs` that delay-loads the screen-reader DLLs;
- has one headless test that walks the menus and asserts the transcript;
- has CI on pull requests, `v*` tags and manual dispatch, Windows only;
- has a README for a first-time Rust developer covering the toolchain, BASS
  and its licence, Prism's build requirements, and running the agent server
  so an assistant can play the game.

## Failure handling

Carried over from Freight Fate's engineering rules:

- Optional everything: no BASS, silent audio; no screen reader, SAPI through
  Prism; missing layers, synthesis; no network, online features say so once
  and the game plays on.
- Nothing unbounded on the loop: speech, audio device discovery, HTTP,
  cloud saves and Discord are off-loop with bounded channels and a defined
  policy for full queues.
- No network, IPC or thread joins in `Drop`; each worker has an explicit
  shutdown that signals, stops accepting work and waits within a measured
  bound, logging each step.
- `unwrap` and indexing only for real invariants; player data, devices and
  network responses are fallible and return contextual errors.

## Testing

- Each crate tests headless: speech through the capture sink, audio through
  the null backend (BASS-backed tests skip themselves without BASS and say
  so), recipes render and are measured, the manifest parser and tier
  resolution are property-tested.
- `hearken-playtest` is itself tested by driving the template.
- Lucky Break's existing tests are the acceptance test for M3: they must
  pass unchanged on Hearken, apart from import paths.
- `cargo fmt --check`, Clippy with warnings denied on all targets, and the
  tests are separate CI gates. `actionlint` runs on every workflow change.

## Milestones

Each milestone gets its own implementation plan.

1. **M1 Skeleton.** Workspace, `hearken-bass-sys` (with callback streams),
   `hearken-speech`, `hearken-app` (shell, stack, menus, settings, help and
   pause screens), and a bare template that boots and passes a headless
   walkthrough.
   **M1b Input.** Rebindable actions and the rebinding screen, the
   controller manager, held-key tracking, text entry, the timed message
   screen.
2. **M2 Sound.** `hearken-audio` including live voices, `hearken-synth`,
   `hearken-bake` with tiers, providers, credits and `--check`.
3. **M3 Lucky Break on Hearken.** Lucky Break drops its `prism`, `prism-sys`,
   `bass-sys` and shell code for Hearken. Its existing Python-rendered WAVs
   enter the manifest as layers, so nothing sounds different on day one;
   recipes then replace them one sound at a time, each with a listening pass.
4. **M4 Playtest and template.** Generic agent server; the template complete;
   the watersports game starts from it.
5. **M5 Public.** Licence files, README, a clean-clone CI run, crates.io
   publishing, then the repo goes public when the owner says.
6. **M6 Freight Fate**, after Career 1.9 ships, one crate at a time.

## Risks

- **Freight Fate keeps changing the code Hearken copies.** Speech, menus,
  bindings and controller code are extracted from `c5d3ef8d`. Until Freight
  Fate moves, a fix to any of those modules there gets ported to Hearken, so
  the move stays a swap. The extraction commit is recorded in each crate's
  docs.
- **First-time setup is heavy.** A C++23 compiler and CMake for Prism, BASS
  from a fetch script, SDL2 vendored for Windows. The template README covers
  all of it on one page.
- **BASS licence.** Free for non-commercial use only; commercial games need
  their own licence, and the engine cannot bundle it. A pure-Rust audio
  backend behind the same trait is possible later and is out of scope for v1.
- **Provider terms.** Each game's author is responsible for the terms of the
  sources they use; the manifest's required provenance is what makes that
  checkable.
