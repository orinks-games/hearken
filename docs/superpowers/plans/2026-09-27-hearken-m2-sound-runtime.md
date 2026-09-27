# Hearken M2 Sound Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Hearken game makes sound with no sound files: a handle-and-pool audio API with engine-side positioning, a public backend trait with BASS as a default feature plus silent and recording backends, live synth voices, built-in earcons and category defaults, per-bus volumes, and gameplay speech pacing, verbosity and ducking.

**Architecture:** Two new crates. `hearken-synth` (fundsp only) renders the built-in sounds to WAV bytes and runs live-voice recipes. `hearken-audio` owns the `Audio` pool: generation-checked `Sound` handles, buses, positioning maths, the bed and music slots, clip lookup that falls back to category defaults, and an off-loop backend open; a backend is a handful of methods over channels and mixes. `hearken-speech` gains Freight Fate's pacer and a generic verbosity table; `hearken-app` joins speech and audio in `Ctx` (event delivery, the duck, `UiSounds`), plays earcons from its built-in screens and adds the sound rows to Settings.

**Tech Stack:** Rust 1.98.0 (edition 2021), `fundsp` 0.23 (`default-features = false, features = ["std"]`), `hearken-bass-sys` (BASS 2.4 and bass_fx's biquad), `sdl2` 0.38, `serde`, `log`.

**Spec:** `docs/superpowers/specs/2026-09-26-hearken-engine-design.md` — Goals (7), Non-goals, the 2026-09-27 decisions, `hearken-speech` (gameplay speech), `hearken-audio`, `hearken-synth` (M2), `hearken-app` ("Sound in the shell"), Testing, Milestones (M2). Sources, read-only, at pinned commits:
- Freight Fate (FF) `c5d3ef8d`, read with `git -C C:/Users/joshu/Freight-fate show c5d3ef8d:<path>`: `crates/ff-core/src/speech_pacing.rs` (pacer, lines 330-1099), `crates/ff-core/src/speech_pacing/tests.rs`, `crates/ff-core/src/speech_pacing/tests/settings.rs` (first test only), `crates/freight-fate/src/app/speech_delivery.rs` (`say_event_with` 583-715, cut helpers 410-470, pause 831-840), `crates/freight-fate/src/audio/engine.rs` (`new_deferred`, the off-loop probe).
- Lucky Break (LB) `a101970`, read with `git -C C:/Users/joshu/gh-projects/lucky-break show a101970:<path>`: `crates/lucky-break/src/audio/{mod,null,bass3d}.rs` (bus model, occlusion numbers, recording fake).
- Open follow-ups: `docs/superpowers/notes/m1-followups.md`.

## Global Constraints

- Toolchain pinned to `1.98.0`; `edition = "2021"`, `rust-version = "1.95"`. fundsp is edition 2024, which is fine as a dependency.
- The one new external dependency is `fundsp = { version = "0.23", default-features = false, features = ["std"] }`. New crates: `hearken-synth`, `hearken-audio`.
- The engine takes sound files and nothing else: no ElevenLabs, no provider or generator trait, no Faust, no Python, no network.
- Source repos are read-only; extract with `git show` at the pinned commit. Each ported module's doc names the commit.
- No trucking, driving, casino, `FREIGHT_FATE_*` or `LUCKY_BREAK_*` names in Hearken code or docs. Per-game environment variables are `<Game::ENV_PREFIX>_*`.
- Every built-in is a default a game can change (spec Goal 7): a value a game would reasonably tune is a public field, config struct or `Game` hook, never a private constant.
- Nothing may block the game loop. No network, IPC or thread joins in `Drop`; each `shutdown` is explicit, idempotent, bounded, and logs its steps.
- `unwrap`/indexing only for real invariants. Spoken text is for players: no jargon, and silence over redundant speech.
- Code files stay at or under 1000 lines.
- Gates, each separately: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`; the tests. No workflow changes are planned; `actionlint` if one happens.
- Exactly one `cargo test` run in flight at a time. Background any command expected to take over 30 seconds. Never pipe a test run to `tail`; redirect to a file and read it.
- Focused tests while iterating (`cargo test -p <crate> --test <file>`, `cargo test -p <crate> --lib <filter>`); the full `cargo test --workspace --locked` once, in Task 12.
- BASS-backed tests skip themselves with an `eprintln!` when BASS cannot load, run on BASS's no-sound device (0), and serialise through one `static Mutex` per test binary, as `crates/hearken-bass-sys/tests/smoke.rs` does with `bass()`.
- Work on branch `m2-sound` in `C:/Users/joshu/gh-projects/hearken` (already cut from `main`; the spec commit `870ae12` is on it). One commit per task. Every commit message ends with:
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`

## Deferred out of M2 (do not build)

- **M2b:** `sounds.toml`, `hearken-bake`, the pack format, credits, `--check`, measurement, the spike's recipes (modal impacts, babble, engine, water), layer helpers, Learn Sounds.
- **Not carried:** FF's "Transitions" rule, delivery receipts (`DeliveryStatus`), message review, `SpokenMessage` rendering inside delivery, sustain loops, hold watchdogs, radio and URL streams, tracker music, BASS's own 3D API and doppler, FF's trucking audio.
- **M4:** `<PREFIX>_NO_CONTROLLER`.

## Review Focus

1. **A NaN or infinite position or facing from game code.** Expected: the sound plays as if at the listener (full gain, centred), a NaN facing counts as north, and no NaN ever reaches a backend. Pinned in Task 2.
2. **The player quits while the sound device is still opening (a slow or hung driver).** Expected: the game closes within the shutdown bound (2 seconds' wait), never hangs. Pinned in Task 4.
3. **A handle kept after its sound ended and its slot went to a new sound.** Expected: calls on the old handle do nothing to the new sound. Pinned in Task 3.
4. **A missing clip played every frame (a footstep with no file).** Expected: its category default plays each time, `missing_clips()` names it once, and the log says so once, not per call. Pinned in Task 3.
5. **A hand-edited settings file with a volume over 100, a bus the game no longer has, or a verbosity level the game dropped.** Expected: the volume reads as 100, the unknown bus is ignored but kept in the file, the first verbosity level is used; nothing is said and nothing crashes. Pinned in Tasks 9 and 11.

---

### Task 1: `hearken-synth` — built-in sounds and live-voice recipes

**Files:**
- Modify: `Cargo.toml` (workspace)
- Create: `crates/hearken-synth/Cargo.toml`, `crates/hearken-synth/src/lib.rs`, `crates/hearken-synth/src/wav.rs`, `crates/hearken-synth/src/builtin.rs`, `crates/hearken-synth/src/voice.rs`
- Test: `crates/hearken-synth/tests/builtin.rs`, `crates/hearken-synth/tests/voice.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  ```rust
  pub use fundsp;                       // games write recipes against the same version
  pub const SAMPLE_RATE: u32 = 44_100;
  pub fn wav_bytes(samples: &[f32], sample_rate: u32) -> Vec<u8>;   // 16-bit PCM mono
  pub const UI_OPEN: &str = "ui/menu_open";   pub const UI_MOVE: &str = "ui/menu_move";
  pub const UI_SELECT: &str = "ui/menu_select"; pub const UI_BACK: &str = "ui/menu_back";
  pub const UI_TICK: &str = "ui/tick";        pub const UI_ERROR: &str = "ui/error";
  pub const UI_NOTIFY: &str = "ui/notify";
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
  pub enum SoundKind { Ui, Sfx, Ambience, Music, Other }
  pub fn kind(key: &str) -> SoundKind;           // "ui/", "sfx/", "amb/", "music/", else Other
  pub fn builtin(key: &str) -> Vec<f32>;         // mono at SAMPLE_RATE: the earcon for a UI_* key, else kind's default
  pub fn builtin_wav(key: &str) -> std::sync::Arc<[u8]>;  // cached per UI_* key and per kind
  pub type Recipe = fn(&Params) -> Box<dyn fundsp::audiounit::AudioUnit>;
  #[derive(Clone, Default)] pub struct Params { /* Vec<(&'static str, fundsp::shared::Shared)> */ }
  impl Params {
      pub fn new(initial: &[(&'static str, f32)]) -> Params;
      pub fn get(&self, name: &str) -> fundsp::shared::Shared; // unknown: warns, returns an unattached 0.0
      pub fn set(&self, name: &str, value: f32) -> bool;       // false (and a debug log) for an unknown name
      pub fn value(&self, name: &str) -> Option<f32>;
  }
  pub struct Voice { /* recipe, params, Box<dyn AudioUnit> */ }
  impl Voice {
      pub fn new(recipe: Recipe, initial: &[(&'static str, f32)]) -> Voice;
      pub fn with_params(recipe: Recipe, params: Params) -> Voice; // shares params' values
      pub fn recipe(&self) -> Recipe;
      pub fn params(&self) -> Params;                            // a clone sharing the same values
      pub fn set_sample_rate(&mut self, hz: f64);
      pub fn fill(&mut self, out: &mut [f32]);                   // mono samples
  }
  ```

- [ ] **Step 1: Add the crate to the workspace**

`Cargo.toml` `[workspace.dependencies]` gains:
```toml
fundsp = { version = "0.23", default-features = false, features = ["std"] }
hearken-synth = { path = "crates/hearken-synth" }
```
`crates/hearken-synth/Cargo.toml`: package `hearken-synth`, description "Synthesised sound for audio games: built-in earcons, category defaults and live-voice recipes", the workspace `version/edition/rust-version/license/repository/authors`, dependencies `fundsp.workspace = true` and `log.workspace = true`. `src/lib.rs` has a crate doc (what the crate is, that it uses fundsp and nothing else external), `mod wav; mod builtin; mod voice;` and re-exports every item listed under Produces.

- [ ] **Step 2: Write the failing tests**

`crates/hearken-synth/tests/builtin.rs`:
```rust
use hearken_synth::*;

const EARCONS: [&str; 7] = [UI_OPEN, UI_MOVE, UI_SELECT, UI_BACK, UI_TICK, UI_ERROR, UI_NOTIFY];

fn peak(v: &[f32]) -> f32 {
    v.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

#[test]
fn a_wav_has_a_riff_header_and_clamps_its_samples() {
    let bytes = wav_bytes(&[0.0, 2.0, -2.0], 44_100);
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..16], b"WAVEfmt ");
    assert_eq!(bytes.len(), 44 + 3 * 2);
    assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 44_100);
    assert_eq!(i16::from_le_bytes([bytes[46], bytes[47]]), 32767);
    assert_eq!(i16::from_le_bytes([bytes[48], bytes[49]]), -32767);
}

#[test]
fn keys_sort_into_kinds_by_their_first_folder() {
    assert_eq!(kind("ui/anything"), SoundKind::Ui);
    assert_eq!(kind("sfx/door"), SoundKind::Sfx);
    assert_eq!(kind("amb/harbour"), SoundKind::Ambience);
    assert_eq!(kind("music/title"), SoundKind::Music);
    assert_eq!(kind("voice/hello"), SoundKind::Other);
    assert_eq!(kind("beep"), SoundKind::Other);
}

#[test]
fn every_earcon_is_short_audible_unclipped_and_its_own() {
    let rendered: Vec<Vec<f32>> = EARCONS.iter().map(|k| builtin(k)).collect();
    for (key, v) in EARCONS.iter().zip(&rendered) {
        assert!(!v.is_empty(), "{key} is empty");
        assert!(v.len() < SAMPLE_RATE as usize / 2, "{key} is longer than half a second");
        let p = peak(v);
        assert!(p > 0.1 && p <= 1.0, "{key} peaks at {p}");
    }
    for i in 0..rendered.len() {
        for j in i + 1..rendered.len() {
            assert_ne!(rendered[i], rendered[j], "{} and {} sound the same", EARCONS[i], EARCONS[j]);
        }
    }
    assert_ne!(builtin("ui/something_else"), rendered[1], "the ui default is not the move earcon");
}

#[test]
fn a_kind_without_a_sound_of_its_own_gets_one_default_rendered_the_same_every_time() {
    assert_eq!(builtin("sfx/door"), builtin("sfx/window"));
    assert_eq!(builtin("amb/a"), builtin("amb/a"));
    assert!(peak(&builtin("sfx/door")) > 0.1);
}

#[test]
fn ambience_and_music_defaults_loop_without_a_click() {
    for (key, seconds) in [("amb/x", 2.0), ("music/x", 4.0)] {
        let v = builtin(key);
        assert_eq!(v.len(), (seconds * SAMPLE_RATE as f32) as usize, "{key} length");
        let seam = (v[0] - v[v.len() - 1]).abs();
        assert!(seam < 0.05, "{key} jumps by {seam} at the loop point");
    }
}

#[test]
fn defaults_are_cached_per_kind_not_per_key() {
    assert!(std::sync::Arc::ptr_eq(&builtin_wav("sfx/a"), &builtin_wav("sfx/b")));
    assert!(!std::sync::Arc::ptr_eq(&builtin_wav(UI_MOVE), &builtin_wav("ui/other")));
}
```
`crates/hearken-synth/tests/voice.rs`:
```rust
use hearken_synth::fundsp::prelude32::*;
use hearken_synth::{Params, Voice};

fn level(p: &Params) -> Box<dyn AudioUnit> {
    Box::new(var(&p.get("level")))
}

#[test]
fn a_voice_renders_its_parameter_and_follows_a_change_through_a_clone() {
    let mut voice = Voice::new(level, &[("level", 0.25)]);
    let mut out = [0.0f32; 8];
    voice.fill(&mut out);
    assert!(out.iter().all(|&s| (s - 0.25).abs() < 1e-6));
    let params = voice.params();
    assert!(params.set("level", 0.5));
    voice.fill(&mut out);
    assert!(out.iter().all(|&s| (s - 0.5).abs() < 1e-6));
    assert_eq!(params.value("level"), Some(0.5));
}

#[test]
fn an_unknown_parameter_is_refused_not_invented() {
    let voice = Voice::new(level, &[("level", 0.0)]);
    assert!(!voice.params().set("throttle", 1.0));
    assert_eq!(voice.params().value("throttle"), None);
}

#[test]
fn a_rebuilt_voice_shares_the_old_ones_values() {
    let first = Voice::new(level, &[("level", 0.1)]);
    let mut second = Voice::with_params(first.recipe(), first.params());
    first.params().set("level", 0.7);
    let mut out = [0.0f32; 2];
    second.fill(&mut out);
    assert!((out[0] - 0.7).abs() < 1e-6);
}

#[test]
fn a_recipe_that_needs_an_input_renders_silence_instead_of_panicking() {
    fn filter_only(_: &Params) -> Box<dyn AudioUnit> {
        Box::new(lowpass_hz(500.0, 0.7))
    }
    let mut voice = Voice::new(filter_only, &[]);
    let mut out = [1.0f32; 4];
    voice.fill(&mut out);
    assert_eq!(out, [0.0; 4]);
}
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `cargo test -p hearken-synth > target/t1.txt 2>&1` and read the file.
Expected: compile errors, the items do not exist yet.

- [ ] **Step 4: Implement**

`wav.rs`: `wav_bytes` writes the 44-byte PCM header (`RIFF`, size `36 + data`, `WAVEfmt `, 16, format 1, 1 channel, rate, rate × 2, block 2, 16 bits, `data`, size) then each sample as `(s.clamp(-1.0, 1.0) * 32767.0).round() as i16` little-endian.

`builtin.rs`. The seven earcons are short sine tones with a 3 ms attack and an exponential decay reaching -60 dB at their last sample, joined end to end:
```rust
const SR: f32 = SAMPLE_RATE as f32;

fn tone(hz: f32, seconds: f32, peak: f32) -> Vec<f32> {
    let n = (seconds * SR) as usize;
    let attack = 0.003 * SR;
    let decay = 1000.0f32.ln() / n as f32;
    (0..n)
        .map(|i| {
            let t = i as f32;
            peak * (t / attack).min(1.0) * (-decay * t).exp() * (std::f32::consts::TAU * hz * t / SR).sin()
        })
        .collect()
}
```
| Key | Sound |
|---|---|
| `UI_OPEN` | `tone(523, 0.040, 0.4)` then `tone(784, 0.060, 0.4)` |
| `UI_MOVE` | `tone(1200, 0.025, 0.35)` |
| `UI_SELECT` | `tone(660, 0.045, 0.4)` then `tone(990, 0.070, 0.4)` |
| `UI_BACK` | `tone(990, 0.045, 0.4)` then `tone(660, 0.070, 0.4)` |
| `UI_TICK` | `tone(2000, 0.012, 0.3)` |
| `UI_ERROR` | `tone(220, 0.090, 0.5)` then `tone(165, 0.120, 0.5)` |
| `UI_NOTIFY` | `tone(784, 0.060, 0.4)`, `tone(988, 0.060, 0.4)`, `tone(1319, 0.120, 0.4)` |
| other `Ui` | `tone(880, 0.060, 0.4)` |
| `Sfx` | 0.15 s of `noise() >> bandpass_hz(1500.0, 1.5)`, each sample × `exp(-i / (0.025 × SR))`, peak-normalised to 0.6 |
| `Ambience` | `pink() >> lowpass_hz(900.0, 0.7)` rendered 2.25 s, made seamless at 2.0 s, RMS-normalised to 0.1 |
| `Music` | `(saw_hz(220.0) + saw_hz(330.0)) * 0.5 >> lowpass_hz(1200.0, 0.7)` rendered 4.25 s, seamless at 4.0 s, RMS 0.1 |
| `Other` | `tone(660, 0.040, 0.4)` |

A fundsp graph is rendered by `set_sample_rate(SAMPLE_RATE as f64)`, then `get_mono()` once per sample. The seam blends the extra quarter second into the start with an equal-power crossfade, so the last sample is followed by the sample that followed it in the render:
```rust
fn seamless(rendered: &[f32], seconds: f32) -> Vec<f32> {
    let n = (seconds * SR) as usize;
    let x = (0.25 * SR) as usize;
    let mut out = rendered[..n].to_vec();
    for i in 0..x {
        let w = i as f32 / x as f32 * std::f32::consts::FRAC_PI_2;
        out[i] = rendered[i] * w.sin() + rendered[n + i] * w.cos();
    }
    out
}
```
`builtin_wav` keeps a `static OnceLock<Mutex<HashMap<&'static str, Arc<[u8]>>>>` keyed by the matching `UI_*` constant, or by the kind's name (`"ui"`, `"sfx"`, `"amb"`, `"music"`, `"other"`) for everything else, so a game with a thousand missing keys holds five defaults. A poisoned lock is recovered with `into_inner`.

`voice.rs`. `Params` wraps `Vec<(&'static str, Shared)>` (fundsp's `Shared` clones share one atomic, so a cloned `Params` controls the same voice). `Voice::with_params` calls the recipe; if the unit has inputs or has neither one nor two outputs, it logs `log::error!` once naming the problem and uses `Box::new(dc(0.0))` instead. `fill` writes `get_mono()` into each sample. The module doc gives a four-line recipe example using `var(&p.get("throttle"))`.

- [ ] **Step 5: Run the tests to see them pass**

Run: `cargo test -p hearken-synth > target/t1.txt 2>&1`
Expected: every test in `builtin.rs` and `voice.rs` passes.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/hearken-synth
git commit -m "feat(synth): built-in earcons, category defaults and live-voice recipes"
```

---

### Task 2: `hearken-audio` — positions, buses, mixes and the positioning model

**Files:**
- Modify: `Cargo.toml` (workspace)
- Create: `crates/hearken-audio/Cargo.toml`, `crates/hearken-audio/src/lib.rs`, `crates/hearken-audio/src/types.rs`, `crates/hearken-audio/src/spatial.rs`
- Test: `crates/hearken-audio/tests/spatial.rs`

**Interfaces:**
- Consumes: nothing yet (the crate depends on `hearken-synth` from Task 3 on).
- Produces:
  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Default)]
  pub struct Vec3 { pub x: f32, pub y: f32, pub z: f32 }   // x east, y north, z up
  impl Vec3 { pub const ZERO: Vec3; pub const fn new(x: f32, y: f32, z: f32) -> Vec3;
              pub fn length(self) -> f32; pub fn is_finite(self) -> bool; }
  impl std::ops::Sub for Vec3 { type Output = Vec3; }
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
  pub struct Bus(pub &'static str);
  impl Bus {
      pub const MASTER: Bus = Bus("master"); pub const UI: Bus = Bus("ui");
      pub const EFFECTS: Bus = Bus("effects"); pub const AMBIENCE: Bus = Bus("ambience");
      pub const MUSIC: Bus = Bus("music"); pub const VOICE: Bus = Bus("voice");
      pub const BUILT_IN: [Bus; 6] = [Bus::MASTER, Bus::UI, Bus::EFFECTS, Bus::AMBIENCE, Bus::MUSIC, Bus::VOICE];
      pub fn name(self) -> &'static str;
  }
  #[derive(Clone, Copy, Debug, PartialEq)]
  pub struct Mix { pub gain: f32, pub pan: f32, pub pitch: f32, pub lowpass_hz: Option<f32>, pub relative: Option<Vec3> }
  impl Mix { pub const UNITY: Mix; }   // gain 1, pan 0, pitch 1, no low-pass, no position
  #[derive(Clone, Copy, Debug, PartialEq, Default)]
  pub struct Listener { pub pos: Vec3, pub facing: f32 }   // degrees clockwise from north
  #[derive(Clone, Copy, Debug, PartialEq)]
  pub struct SpatialConfig {
      pub min_distance: f32,         // 1.5  full volume inside this
      pub max_distance: f32,         // 30.0 no quieter beyond this
      pub rolloff: f32,              // 1.0
      pub behind_lowpass_hz: f32,    // 3000.0 for a sound straight behind
      pub behind_gain: f32,          // 0.8
      pub occlusion_lowpass_hz: f32, // 800.0 (LB)
      pub occlusion_gain: f32,       // 0.5, -6 dB (LB)
  }
  impl Default for SpatialConfig { /* the values above */ }
  #[derive(Clone, Copy, Debug, PartialEq)]
  pub struct Placement { pub gain: f32, pub pan: f32, pub lowpass_hz: Option<f32>, pub relative: Vec3 }
  pub fn place(listener: &Listener, pos: Vec3, occluded: bool, cfg: &SpatialConfig) -> Placement;
  ```
  `relative` is the sound's position in the listener's frame: x to the right, y ahead, z up.

- [ ] **Step 1: Add the crate**

Workspace `[workspace.dependencies]` gains `hearken-audio = { path = "crates/hearken-audio", default-features = false }`. `crates/hearken-audio/Cargo.toml`: package `hearken-audio`, description "Sound for audio games: a pool of positioned sounds, buses and live voices over any audio library", workspace fields, `log.workspace = true`. `lib.rs`: crate doc (the handle-and-pool model, that positioning is the engine's, NVGT's axes), `mod types; mod spatial;` and re-exports.

- [ ] **Step 2: Write the failing tests**

`crates/hearken-audio/tests/spatial.rs`:
```rust
use hearken_audio::{place, Listener, SpatialConfig, Vec3};

fn at(x: f32, y: f32) -> Vec3 {
    Vec3::new(x, y, 0.0)
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn a_sound_on_the_listener_is_full_and_centred() {
    let p = place(&Listener::default(), Vec3::ZERO, false, &SpatialConfig::default());
    assert_eq!((p.gain, p.pan, p.lowpass_hz), (1.0, 0.0, None));
}

#[test]
fn facing_north_east_is_right_and_west_is_left() {
    let cfg = SpatialConfig::default();
    let l = Listener::default();
    assert!(near(place(&l, at(5.0, 0.0), false, &cfg).pan, 1.0));
    assert!(near(place(&l, at(-5.0, 0.0), false, &cfg).pan, -1.0));
    assert!(near(place(&l, at(0.0, 5.0), false, &cfg).pan, 0.0));
}

#[test]
fn turning_to_face_east_puts_north_on_the_left_and_ahead_becomes_east() {
    let cfg = SpatialConfig::default();
    let l = Listener { pos: Vec3::ZERO, facing: 90.0 };
    assert!(near(place(&l, at(0.0, 5.0), false, &cfg).pan, -1.0));
    let ahead = place(&l, at(5.0, 0.0), false, &cfg);
    assert!(near(ahead.pan, 0.0));
    assert!(near(ahead.relative.y, 5.0), "east is straight ahead: {:?}", ahead.relative);
}

#[test]
fn gain_falls_with_distance_and_holds_beyond_the_maximum() {
    let cfg = SpatialConfig::default(); // min 1.5, max 30, rolloff 1
    let l = Listener::default();
    assert!(near(place(&l, at(0.0, 1.5), false, &cfg).gain, 1.0));
    assert!(near(place(&l, at(0.0, 3.0), false, &cfg).gain, 0.5));
    let at_max = place(&l, at(0.0, 30.0), false, &cfg).gain;
    assert!(near(place(&l, at(0.0, 300.0), false, &cfg).gain, at_max));
}

#[test]
fn a_sound_behind_is_duller_and_a_little_quieter_but_one_beside_is_not() {
    let cfg = SpatialConfig::default();
    let l = Listener::default();
    let behind = place(&l, at(0.0, -1.0), false, &cfg);
    assert!(near(behind.gain, cfg.behind_gain));
    assert!(near(behind.lowpass_hz.unwrap(), cfg.behind_lowpass_hz));
    assert_eq!(place(&l, at(1.0, 0.0), false, &cfg).lowpass_hz, None);
}

#[test]
fn occlusion_halves_the_gain_and_muffles_to_the_lower_cutoff() {
    let cfg = SpatialConfig::default();
    let l = Listener::default();
    let p = place(&l, at(0.0, 1.0), true, &cfg);
    assert!(near(p.gain, 0.5));
    assert_eq!(p.lowpass_hz, Some(800.0));
    let behind = place(&l, at(0.0, -1.0), true, &cfg);
    assert_eq!(behind.lowpass_hz, Some(800.0), "the lower of the two cutoffs wins");
}

#[test]
fn nonsense_numbers_from_the_game_never_reach_a_backend() {
    let cfg = SpatialConfig::default();
    for pos in [Vec3::new(f32::NAN, 0.0, 0.0), Vec3::new(f32::INFINITY, 1.0, 0.0)] {
        let p = place(&Listener::default(), pos, true, &cfg);
        assert_eq!((p.gain, p.pan, p.lowpass_hz), (1.0, 0.0, None), "{pos:?}");
        assert_eq!(p.relative, Vec3::ZERO);
    }
    let lost = Listener { pos: Vec3::ZERO, facing: f32::NAN };
    assert!(near(place(&lost, at(5.0, 0.0), false, &cfg).pan, 1.0), "a NaN facing counts as north");
    let odd = SpatialConfig { min_distance: 0.0, max_distance: -1.0, ..cfg };
    let p = place(&Listener::default(), at(0.0, 2.0), false, &odd);
    assert!(p.gain.is_finite() && p.pan.is_finite());
}
```

- [ ] **Step 3: Run them to see them fail**

Run: `cargo test -p hearken-audio --test spatial > target/t2.txt 2>&1`
Expected: compile errors.

- [ ] **Step 4: Implement**

`types.rs` holds `Vec3`, `Bus`, `Mix`, `Listener`. `spatial.rs` holds `SpatialConfig`, `Placement` and `place`:
```rust
pub fn place(listener: &Listener, pos: Vec3, occluded: bool, cfg: &SpatialConfig) -> Placement {
    let rel = pos - listener.pos;
    if !rel.is_finite() {
        return Placement { gain: 1.0, pan: 0.0, lowpass_hz: None, relative: Vec3::ZERO };
    }
    let facing = if listener.facing.is_finite() { listener.facing } else { 0.0 };
    let (s, c) = facing.to_radians().sin_cos();
    let right = rel.x * c - rel.y * s;
    let ahead = rel.x * s + rel.y * c;
    let min = cfg.min_distance.max(f32::EPSILON);
    let d = rel.length().clamp(min, cfg.max_distance.max(min));
    let mut gain = min / (min + cfg.rolloff.max(0.0) * (d - min));
    let flat = right.hypot(ahead);
    let (pan, behind) = if flat > 1e-4 {
        ((right / flat).clamp(-1.0, 1.0), (-ahead / flat).max(0.0))
    } else {
        (0.0, 0.0)
    };
    let mut lowpass = None;
    if behind > 0.0 {
        gain *= 1.0 - (1.0 - cfg.behind_gain) * behind;
        lowpass = Some(20_000.0 * (cfg.behind_lowpass_hz / 20_000.0).powf(behind));
    }
    if occluded {
        gain *= cfg.occlusion_gain;
        lowpass = Some(lowpass.map_or(cfg.occlusion_lowpass_hz, |hz: f32| hz.min(cfg.occlusion_lowpass_hz)));
    }
    Placement { gain, pan, lowpass_hz: lowpass, relative: Vec3::new(right, ahead, rel.z) }
}
```
Doc comment on `SpatialConfig`: every number is the game's to change; the occlusion values are Lucky Break's.

- [ ] **Step 5: Run them to see them pass**

Run: `cargo test -p hearken-audio --test spatial > target/t2.txt 2>&1`
Expected: 7 passed.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/hearken-audio
git commit -m "feat(audio): positions, buses, mixes and the positioning model"
```

---

### Task 3: `hearken-audio` — the backend trait, clips, and the pool's handles

**Files:**
- Modify: `crates/hearken-audio/Cargo.toml`, `crates/hearken-audio/src/lib.rs`
- Create: `crates/hearken-audio/src/backend.rs`, `crates/hearken-audio/src/silent.rs`, `crates/hearken-audio/src/recording.rs`, `crates/hearken-audio/src/clips.rs`, `crates/hearken-audio/src/pool.rs`
- Test: `crates/hearken-audio/tests/pool.rs`, `crates/hearken-audio/tests/clips.rs`

**Interfaces:**
- Consumes: Task 1 `Voice`, `Recipe`, `Params`, `builtin_wav`; Task 2 `Vec3`, `Bus`, `Mix`, `Listener`, `SpatialConfig`, `place`.
- Produces:
  ```rust
  // backend.rs
  pub type Channel = u64;
  pub enum Source { Clip(std::sync::Arc<[u8]>), Voice(hearken_synth::Voice) }
  #[derive(Clone, Debug, PartialEq)] pub struct BackendError(pub String); // Display + Error
  pub trait Backend: Send {
      fn name(&self) -> String;
      fn play(&mut self, key: &str, source: Source, looping: bool, mix: Mix) -> Result<Channel, BackendError>;
      fn set_mix(&mut self, channel: Channel, mix: Mix, ramp_ms: u32);
      fn stop(&mut self, channel: Channel, fade_ms: u32);   // the backend fades, then frees
      fn pause(&mut self, channel: Channel, paused: bool);
      fn is_playing(&mut self, channel: Channel) -> bool;   // a paused channel is still playing
      fn update(&mut self) {}                               // once a frame: reap ended and faded channels
      fn shutdown(&mut self) {}
  }
  // silent.rs
  #[derive(Default)] pub struct SilentBackend { /* next id, looping ids */ }
  // recording.rs
  #[derive(Clone, Debug, PartialEq)]
  pub enum Call {
      Play { channel: Channel, key: String, looping: bool, clip: Option<std::sync::Arc<[u8]>>, mix: Mix },
      SetMix { channel: Channel, mix: Mix, ramp_ms: u32 },
      Stop { channel: Channel, fade_ms: u32 },
      Pause { channel: Channel, paused: bool },
  }
  #[derive(Clone, Default)] pub struct Recorder { /* Arc<Mutex<..>> */ }
  impl Recorder {
      pub fn calls(&self) -> Vec<Call>;
      pub fn played(&self) -> Vec<String>;          // keys of Play calls, in order
      pub fn last_mix(&self, channel: Channel) -> Option<Mix>;
      pub fn channel_of(&self, key: &str) -> Option<Channel>; // the latest Play of `key`
      pub fn finish(&self, channel: Channel);       // that channel stops playing
      pub fn fail_next_play(&self, reason: &str);   // the next Play returns Err
      pub fn clear(&self);                          // forgets calls, not channels
  }
  pub struct RecordingBackend { /* Recorder */ }
  impl RecordingBackend { pub fn new() -> (RecordingBackend, Recorder); }
  // clips.rs
  pub trait ClipSource: Send { fn load(&mut self, key: &str) -> Option<std::sync::Arc<[u8]>>; }
  pub struct LooseClips { /* root, cache incl. misses */ }
  impl LooseClips {
      pub const EXTENSIONS: [&'static str; 3] = ["ogg", "wav", "mp3"];
      pub fn at(root: impl Into<std::path::PathBuf>) -> LooseClips;
      pub fn none() -> LooseClips;
      pub fn discover(env_prefix: &str) -> LooseClips; // <PREFIX>_SOUNDS, <exe dir>/sounds, <cwd>/sounds
      pub fn root(&self) -> Option<&std::path::Path>;
  }
  #[derive(Default)] pub struct MemoryClips { /* HashMap */ }
  impl MemoryClips { pub fn with(self, key: &str, bytes: impl Into<std::sync::Arc<[u8]>>) -> MemoryClips; }
  // pool.rs
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
  pub struct Sound { /* index: u32, generation: u32 */ }
  impl Sound { pub const NONE: Sound; }
  #[derive(Clone, Debug)]
  pub struct LiveSound { pub sound: Sound, pub key: String, pub bus: Bus, pub pos: Option<Vec3>,
                         pub looping: bool, pub paused: bool, pub params: Option<hearken_synth::Params> }
  #[derive(Clone, Copy, Debug, PartialEq)]
  pub struct Fades { pub bed_ms: u32, pub music_ms: u32, pub duck_ms: u32, pub volume_ms: u32, pub move_ms: u32 }
  // Default: 800, 500, 150, 50, 30
  pub struct Audio { pub spatial: SpatialConfig, pub fades: Fades, /* private */ }
  impl Audio {
      pub fn new(backend: Box<dyn Backend>, clips: Box<dyn ClipSource>) -> Audio;
      pub fn silent() -> Audio;                       // SilentBackend, LooseClips::none()
      pub fn play(&mut self, key: &str, bus: Bus) -> Sound;
      pub fn play_at(&mut self, key: &str, pos: Vec3, bus: Bus) -> Sound;
      pub fn start_loop(&mut self, key: &str, pos: Option<Vec3>, bus: Bus) -> Sound;
      pub fn start_voice(&mut self, recipe: hearken_synth::Recipe, params: &[(&'static str, f32)],
                         pos: Option<Vec3>, bus: Bus) -> Sound;
      pub fn stop(&mut self, sound: Sound, fade_ms: u32);
      pub fn pause(&mut self, sound: Sound);
      pub fn resume(&mut self, sound: Sound);
      pub fn set_volume(&mut self, sound: Sound, volume: f32);   // clamped 0..=1
      pub fn set_pitch(&mut self, sound: Sound, pitch: f32);     // clamped 0.05..=4
      pub fn set_position(&mut self, sound: Sound, pos: Option<Vec3>);
      pub fn set_voice_param(&mut self, sound: Sound, name: &str, value: f32) -> bool;
      pub fn is_playing(&self, sound: Sound) -> bool;            // alive in the pool
      pub fn set_listener(&mut self, pos: Vec3, facing: f32);
      pub fn listener(&self) -> Listener;
      pub fn set_bus_volume(&mut self, bus: Bus, volume: f32);   // clamped 0..=1; MASTER scales every bus
      pub fn bus_volume(&self, bus: Bus) -> f32;                 // 1.0 until set
      pub fn update(&mut self, occluded: &dyn Fn(Vec3) -> bool);
      pub fn missing_clips(&self) -> Vec<String>;                // sorted
      pub fn live(&self) -> Vec<LiveSound>;
      pub fn backend_name(&self) -> String;
  }
  ```

- [ ] **Step 1: Write the failing tests**

`crates/hearken-audio/tests/clips.rs`:
```rust
use hearken_audio::{ClipSource, LooseClips};

#[test]
fn a_key_finds_its_file_by_folder_and_any_known_extension() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("ui")).unwrap();
    std::fs::write(dir.path().join("ui/menu_move.wav"), b"RIFF-ish").unwrap();
    std::fs::write(dir.path().join("door.ogg"), b"OggS").unwrap();
    let mut clips = LooseClips::at(dir.path());
    assert_eq!(clips.load("ui/menu_move").as_deref(), Some(&b"RIFF-ish"[..]));
    assert_eq!(clips.load("door").as_deref(), Some(&b"OggS"[..]));
    assert!(clips.load("ui/nothing").is_none());
}

#[test]
fn a_key_that_climbs_out_of_the_sounds_folder_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("secret.wav"), b"x").unwrap();
    let inner = dir.path().join("sounds");
    std::fs::create_dir_all(&inner).unwrap();
    let mut clips = LooseClips::at(&inner);
    for key in ["../secret", "/secret", "C:/secret", "a/../../secret"] {
        assert!(clips.load(key).is_none(), "{key}");
    }
}

#[test]
fn a_miss_is_remembered_so_the_disk_is_not_asked_every_frame() {
    let dir = tempfile::tempdir().unwrap();
    let mut clips = LooseClips::at(dir.path());
    assert!(clips.load("sfx/step").is_none());
    std::fs::create_dir_all(dir.path().join("sfx")).unwrap();
    std::fs::write(dir.path().join("sfx/step.wav"), b"x").unwrap();
    assert!(clips.load("sfx/step").is_none(), "cached as missing for the run");
}
```
`crates/hearken-audio/tests/pool.rs`:
```rust
use hearken_audio::*;

fn pool() -> (Audio, Recorder) {
    let (backend, rec) = RecordingBackend::new();
    let clips = MemoryClips::default().with("sfx/door", &b"door"[..]).with("ui/menu_move", &b"tick"[..]);
    (Audio::new(Box::new(backend), Box::new(clips)), rec)
}

fn no_occlusion(_: Vec3) -> bool {
    false
}

#[test]
fn a_one_shot_plays_its_clip_at_the_bus_volume() {
    let (mut audio, rec) = pool();
    audio.set_bus_volume(Bus::EFFECTS, 0.5);
    let door = audio.play("sfx/door", Bus::EFFECTS);
    assert!(audio.is_playing(door));
    match &rec.calls()[0] {
        Call::Play { key, looping, clip, mix, .. } => {
            assert_eq!((key.as_str(), *looping), ("sfx/door", false));
            assert_eq!(clip.as_deref(), Some(&b"door"[..]));
            assert_eq!((mix.gain, mix.pan, mix.pitch), (0.5, 0.0, 1.0));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_missing_clip_plays_its_default_and_is_named_once_however_often_it_plays() {
    let (mut audio, rec) = pool();
    for _ in 0..100 {
        audio.play("sfx/step", Bus::EFFECTS);
    }
    assert_eq!(audio.missing_clips(), ["sfx/step"]);
    let default = hearken_synth::builtin_wav("sfx/step");
    assert!(rec.calls().iter().all(|c| matches!(c, Call::Play { clip: Some(b), .. } if **b == *default)));
    assert_eq!(rec.played().len(), 100);
}

#[test]
fn a_clip_the_backend_cannot_play_falls_back_to_its_default() {
    let (mut audio, rec) = pool();
    rec.fail_next_play("not a sound file");
    audio.play("sfx/door", Bus::EFFECTS);
    assert_eq!(audio.missing_clips(), ["sfx/door"]);
    let default = hearken_synth::builtin_wav("sfx/door");
    assert!(matches!(rec.calls().last(), Some(Call::Play { clip: Some(b), .. }) if **b == *default));
}

#[test]
fn an_old_handle_does_nothing_to_the_sound_that_took_its_slot() {
    let (mut audio, rec) = pool();
    let old = audio.play("sfx/door", Bus::EFFECTS);
    rec.finish(rec.channel_of("sfx/door").unwrap());
    audio.update(&no_occlusion);
    assert!(!audio.is_playing(old));
    let new = audio.start_loop("ui/menu_move", None, Bus::UI);
    rec.clear();
    audio.set_pitch(old, 2.0);
    audio.stop(old, 0);
    assert!(rec.calls().is_empty(), "{:?}", rec.calls());
    assert!(audio.is_playing(new));
}

#[test]
fn a_burst_of_one_shots_leaves_nothing_behind_once_they_end() {
    let (mut audio, rec) = pool();
    for _ in 0..1000 {
        audio.play("ui/menu_move", Bus::UI);
    }
    for call in rec.calls() {
        if let Call::Play { channel, .. } = call {
            rec.finish(channel);
        }
    }
    audio.update(&no_occlusion);
    assert!(audio.live().is_empty());
}

#[test]
fn a_placed_sound_follows_the_listener_and_is_only_re_mixed_on_a_change() {
    let (mut audio, rec) = pool();
    audio.play_at("sfx/door", Vec3::new(5.0, 0.0, 0.0), Bus::EFFECTS);
    let ch = rec.channel_of("sfx/door").unwrap();
    assert!((rec.last_mix(ch).unwrap().pan - 1.0).abs() < 1e-4);
    audio.set_listener(Vec3::ZERO, 180.0);
    audio.update(&no_occlusion);
    assert!((rec.last_mix(ch).unwrap().pan + 1.0).abs() < 1e-4, "facing south, east is on the left");
    rec.clear();
    audio.update(&no_occlusion);
    assert!(rec.calls().is_empty(), "nothing changed, nothing sent");
}

#[test]
fn the_games_occlusion_answer_reaches_the_mix() {
    let (mut audio, rec) = pool();
    audio.play_at("sfx/door", Vec3::new(0.0, 1.0, 0.0), Bus::EFFECTS);
    audio.update(&|_| true);
    let mix = rec.last_mix(rec.channel_of("sfx/door").unwrap()).unwrap();
    assert_eq!((mix.gain, mix.lowpass_hz), (0.5, Some(800.0)));
}

#[test]
fn a_bus_volume_reaches_only_its_own_sounds_and_master_reaches_all() {
    let (mut audio, rec) = pool();
    audio.play("sfx/door", Bus::EFFECTS);
    audio.play("ui/menu_move", Bus::UI);
    let (door, tick) = (rec.channel_of("sfx/door").unwrap(), rec.channel_of("ui/menu_move").unwrap());
    audio.set_bus_volume(Bus::EFFECTS, 0.25);
    assert_eq!(rec.last_mix(door).unwrap().gain, 0.25);
    assert_eq!(rec.last_mix(tick).unwrap().gain, 1.0);
    audio.set_bus_volume(Bus::MASTER, 0.5);
    assert_eq!(rec.last_mix(door).unwrap().gain, 0.125);
    assert_eq!(rec.last_mix(tick).unwrap().gain, 0.5);
}

#[test]
fn a_voice_takes_its_parameters_by_name() {
    use hearken_synth::fundsp::prelude32::*;
    fn hum(p: &hearken_synth::Params) -> Box<dyn AudioUnit> {
        Box::new(var(&p.get("throttle")) * 100.0 >> sine())
    }
    let (mut audio, _rec) = pool();
    let v = audio.start_voice(hum, &[("throttle", 0.0)], None, Bus::EFFECTS);
    assert!(audio.set_voice_param(v, "throttle", 0.8));
    assert!(!audio.set_voice_param(v, "brake", 1.0));
    let live = audio.live();
    assert_eq!(live[0].params.as_ref().unwrap().value("throttle"), Some(0.8));
    assert!(live[0].looping);
}

#[test]
fn stopping_passes_the_fade_to_the_backend_and_frees_the_handle() {
    let (mut audio, rec) = pool();
    let s = audio.start_loop("sfx/door", None, Bus::EFFECTS);
    audio.stop(s, 300);
    assert!(matches!(rec.calls().last(), Some(Call::Stop { fade_ms: 300, .. })));
    assert!(!audio.is_playing(s));
}
```
Add `tempfile.workspace = true` to `[dev-dependencies]` of `hearken-audio`.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p hearken-audio --test pool --test clips > target/t3.txt 2>&1`
Expected: compile errors.

- [ ] **Step 3: Implement**

`crates/hearken-audio/Cargo.toml` adds `hearken-synth.workspace = true`.

`silent.rs`: `SilentBackend` hands out increasing channel ids; a looping play (clip or voice) stays "playing" until `stop`; a one-shot is not playing from the moment it starts. Every other method is a no-op. `name()` is `"silent"`.

`recording.rs`: `RecordingBackend` appends a `Call` for every method and answers `is_playing` from a set of started, unfinished, unstopped channels. `Source::Voice` records `clip: None`. `fail_next_play` makes the next `play` return `Err(BackendError(reason))` without recording a `Play`. `name()` is `"recording"`.

`clips.rs`: a key is refused (`None`, and `log::warn!` once per key) when it is empty, starts with `/` or `\`, contains `:` or a `..` component. `load` tries `root/<key>.<ext>` for each extension, reads the file with `std::fs::read`, and caches the result, a miss included, in a `HashMap<String, Option<Arc<[u8]>>>`. `discover` takes the first of `<PREFIX>_SOUNDS` (non-empty), `<exe dir>/sounds`, `<cwd>/sounds` that is a directory, logs which one (or that there is none), and is `none()` otherwise.

`pool.rs`, the core of the pool:
```rust
struct Slot { generation: u32, live: Option<Live> }

struct Live {
    key: String,
    channel: Channel,
    bus: Bus,
    pos: Option<Vec3>,
    volume: f32,
    pitch: f32,
    looping: bool,
    voice: Option<(Recipe, Params)>,
    paused: bool,
    paused_by_all: bool,   // Task 4
    occluded: bool,        // the last answer from `update`
    sent: Mix,             // the last mix given to the backend
}
```
- A `Sound` is `{ index, generation }`; lookups compare the slot's generation and that it is live, so a stale handle finds nothing. `Sound::NONE` is `{ index: u32::MAX, generation: 0 }`. Freed indices go on a free list; freeing bumps the generation (`wrapping_add(1)`).
- The mix of a live sound:
  ```rust
  fn mix_of(&self, live: &Live) -> Mix {
      let mut gain = live.volume * self.bus_gain(live.bus);
      let (mut pan, mut lowpass_hz, mut relative) = (0.0, None, None);
      if let Some(pos) = live.pos {
          let p = place(&self.listener, pos, live.occluded, &self.spatial);
          gain *= p.gain;
          (pan, lowpass_hz, relative) = (p.pan, p.lowpass_hz, Some(p.relative));
      }
      Mix { gain, pan, pitch: live.pitch, lowpass_hz, relative }
  }
  ```
  `bus_gain(bus)` is the master volume, times the bus's own volume unless the bus is `MASTER`, times the duck level when the bus is ducked (Task 4; 1.0 here).
- Starting a sound (`play`, `play_at`, `start_loop`, `start_voice` all go through one private `start`): take the clip from `ClipSource::load`, or, when it is `None`, note the key missing and use `hearken_synth::builtin_wav(key)`. If `Backend::play` fails, note the key missing, log the error with the key, and try the key's default once more; if that fails too, log it and return `Sound::NONE`. A voice has no clip: it is `Source::Voice(Voice::with_params(recipe, params.clone()))` and its key is `"voice"`. Noting a missing key inserts it into a `BTreeSet<String>` and logs `log::warn!("sound {key} is missing; playing its default")` only when the insert was new.
- `stop` calls `Backend::stop(channel, fade_ms)` and frees the slot at once, since the backend owns the fade. `pause` and `resume` set `paused` and call `Backend::pause`.
- `set_volume`, `set_pitch` and `set_position` change the field and re-send the mix with `fades.move_ms` if it differs from `sent`. `set_bus_volume` re-sends every affected sound with `fades.volume_ms`.
- `update`: `Backend::update()`; then for every live slot, free it when `!paused && !backend.is_playing(channel)`; otherwise, for a positioned sound, ask `occluded(pos)`, recompute the mix, and send it with `fades.move_ms` when it differs from `sent`.

- [ ] **Step 4: Run them to see them pass**

Run: `cargo test -p hearken-audio --test pool --test clips > target/t3.txt 2>&1`
Expected: every test passes.

- [ ] **Step 5: Commit**

```bash
git add Cargo.lock crates/hearken-audio
git commit -m "feat(audio): the backend trait, loose clips, and a pool of sound handles"
```

---

### Task 4: `hearken-audio` — bed, music, pause, duck, the off-loop open and shutdown

**Files:**
- Modify: `crates/hearken-audio/src/pool.rs`, `crates/hearken-audio/src/lib.rs`
- Create: `crates/hearken-audio/src/slots.rs` (`impl Audio` for the bed, music, pause-all and the duck), `crates/hearken-audio/src/open.rs` (`impl Audio` for the deferred open, the notice and shutdown)
- Test: `crates/hearken-audio/tests/slots.rs`, `crates/hearken-audio/tests/open.rs`

**Interfaces:**
- Consumes: Task 3's `Audio`, `Backend`, `RecordingBackend`, `Recorder`, `MemoryClips`.
- Produces:
  ```rust
  impl Audio {
      pub fn set_bed(&mut self, key: Option<&str>);            // AMBIENCE bus, crossfade fades.bed_ms
      pub fn bed(&self) -> Option<&str>;
      pub fn play_music(&mut self, key: Option<&str>, looping: bool); // MUSIC bus, fades.music_ms
      pub fn music(&self) -> Option<&str>;
      pub fn pause_all(&mut self);
      pub fn resume_all(&mut self);                            // only what pause_all paused
      pub fn set_duck_buses(&mut self, buses: &[Bus]);
      pub fn set_duck(&mut self, level: f32);                  // clamped 0..=1; 1.0 is no duck
      pub fn duck(&self) -> f32;
      pub fn deferred(open: impl FnOnce() -> Result<Box<dyn Backend>, String> + Send + 'static,
                      clips: Box<dyn ClipSource>) -> Audio;
      pub fn opening(&self) -> bool;
      pub fn take_notice(&mut self) -> Option<String>;
      pub fn shutdown(&mut self);                              // bounded by SHUTDOWN_WAIT; idempotent
  }
  pub const NO_SOUND_NOTICE: &str = "No sound device could be opened, so the game is playing without sound.";
  pub const SHUTDOWN_WAIT: std::time::Duration = std::time::Duration::from_secs(2);
  ```

- [ ] **Step 1: Write the failing tests**

`crates/hearken-audio/tests/slots.rs`:
```rust
use hearken_audio::*;

fn pool() -> (Audio, Recorder) {
    let (backend, rec) = RecordingBackend::new();
    (Audio::new(Box::new(backend), Box::new(MemoryClips::default())), rec)
}

#[test]
fn a_new_bed_fades_in_over_the_old_one_fading_out_and_the_same_bed_does_nothing() {
    let (mut audio, rec) = pool();
    audio.set_bed(Some("amb/harbour"));
    let first = rec.channel_of("amb/harbour").unwrap();
    rec.clear();
    audio.set_bed(Some("amb/harbour"));
    assert!(rec.calls().is_empty());
    audio.set_bed(Some("amb/market"));
    let calls = rec.calls();
    assert!(calls.contains(&Call::Stop { channel: first, fade_ms: 800 }));
    let second = rec.channel_of("amb/market").unwrap();
    assert!(matches!(calls.iter().find(|c| matches!(c, Call::Play { channel, .. } if *channel == second)),
        Some(Call::Play { looping: true, mix, .. }) if mix.gain == 0.0));
    assert!(calls.iter().any(|c| matches!(c, Call::SetMix { channel, ramp_ms: 800, mix } if *channel == second && mix.gain == 1.0)));
    assert_eq!(audio.bed(), Some("amb/market"));
    audio.set_bed(None);
    assert_eq!(audio.bed(), None);
}

#[test]
fn music_asked_for_again_carries_on_and_new_music_replaces_it() {
    let (mut audio, rec) = pool();
    audio.play_music(Some("music/title"), true);
    let title = rec.channel_of("music/title").unwrap();
    rec.clear();
    audio.play_music(Some("music/title"), true);
    assert!(rec.calls().is_empty());
    audio.play_music(Some("music/game"), false);
    assert!(rec.calls().contains(&Call::Stop { channel: title, fade_ms: 500 }));
    assert_eq!(audio.music(), Some("music/game"));
}

#[test]
fn pause_all_pauses_what_was_playing_and_resume_all_resumes_only_that() {
    let (mut audio, rec) = pool();
    let a = audio.start_loop("sfx/a", None, Bus::EFFECTS);
    let b = audio.start_loop("sfx/b", None, Bus::EFFECTS);
    audio.pause(b);
    audio.pause_all();
    let menu = audio.play("ui/menu_move", Bus::UI);
    audio.resume_all();
    let resumed: Vec<_> = rec.calls().into_iter()
        .filter(|c| matches!(c, Call::Pause { paused: false, .. })).collect();
    assert_eq!(resumed.len(), 1, "only a: b was paused by the game, the menu sound never was");
    assert!(audio.is_playing(a) && audio.is_playing(b) && audio.is_playing(menu));
}

#[test]
fn the_duck_lowers_only_its_buses() {
    let (mut audio, rec) = pool();
    audio.set_duck_buses(&[Bus::MUSIC]);
    audio.play_music(Some("music/x"), true);
    audio.play("ui/menu_move", Bus::UI);
    audio.set_duck(0.5);
    assert_eq!(rec.last_mix(rec.channel_of("music/x").unwrap()).unwrap().gain, 0.5);
    assert_eq!(rec.last_mix(rec.channel_of("ui/menu_move").unwrap()).unwrap().gain, 1.0);
    audio.set_duck(1.0);
    assert_eq!(rec.last_mix(rec.channel_of("music/x").unwrap()).unwrap().gain, 1.0);
}
```
`crates/hearken-audio/tests/open.rs`:
```rust
use std::sync::mpsc;
use std::time::{Duration, Instant};

use hearken_audio::*;

fn none(_: Vec3) -> bool {
    false
}

#[test]
fn the_game_runs_silent_until_the_device_opens_then_its_loops_and_volumes_move_over() {
    let (backend, rec) = RecordingBackend::new();
    let (go, wait) = mpsc::channel::<()>();
    let mut audio = Audio::deferred(
        move || {
            wait.recv().ok();
            Ok(Box::new(backend) as Box<dyn Backend>)
        },
        Box::new(MemoryClips::default()),
    );
    assert!(audio.opening());
    audio.set_bus_volume(Bus::MUSIC, 0.5);
    audio.play_music(Some("music/title"), true);
    let hum = audio.start_loop("sfx/hum", None, Bus::EFFECTS);
    let blip = audio.play("ui/menu_move", Bus::UI);
    go.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while audio.opening() && Instant::now() < deadline {
        audio.update(&none);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!audio.opening());
    assert_eq!(rec.played(), ["music/title", "sfx/hum"], "one-shots are not replayed");
    assert_eq!(rec.last_mix(rec.channel_of("music/title").unwrap()).unwrap().gain, 0.5);
    assert!(audio.is_playing(hum), "the handle survives the move");
    assert!(!audio.is_playing(blip));
    assert_eq!(audio.take_notice(), None);
}

#[test]
fn a_device_that_will_not_open_leaves_silence_and_says_so_once() {
    let mut audio = Audio::deferred(|| Err("no device".to_string()), Box::new(MemoryClips::default()));
    let deadline = Instant::now() + Duration::from_secs(5);
    while audio.opening() && Instant::now() < deadline {
        audio.update(&none);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(audio.take_notice().as_deref(), Some(NO_SOUND_NOTICE));
    assert_eq!(audio.take_notice(), None);
    assert_eq!(audio.backend_name(), "silent");
}

#[test]
fn quitting_while_the_device_is_still_opening_is_bounded() {
    let mut audio = Audio::deferred(
        || {
            std::thread::sleep(Duration::from_secs(10));
            Ok(Box::new(SilentBackend::default()) as Box<dyn Backend>)
        },
        Box::new(MemoryClips::default()),
    );
    let started = Instant::now();
    audio.shutdown();
    assert!(started.elapsed() < SHUTDOWN_WAIT + Duration::from_millis(500));
    audio.shutdown(); // idempotent
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p hearken-audio --test slots --test open > target/t4.txt 2>&1`
Expected: compile errors.

- [ ] **Step 3: Implement**

`slots.rs`:
- The pool keeps `bed: Option<(String, Sound)>` and `music: Option<(String, bool, Sound)>`.
- `set_bed(Some(k))` for the playing key does nothing. Otherwise the old bed gets `stop(old, fades.bed_ms)`, and the new one starts as a loop on `Bus::AMBIENCE` with `volume` 0, then has its volume set to 1 with a `set_mix` ramp of `fades.bed_ms`. `set_bed(None)` stops the bed with the same fade.
- `play_music(Some(k), looping)` for the same key and loop flag does nothing. Otherwise the old track gets `stop(old, fades.music_ms)` and the new one starts at full volume on `Bus::MUSIC` (`start_loop` when `looping`, a one-shot otherwise). `play_music(None, _)` stops it.
- `bed()` and `music()` return the key while its handle is still playing.
- `pause_all` pauses every live sound that is not paused, setting `paused_by_all`. `resume_all` resumes exactly those and clears the flag.
- `set_duck` stores the level and re-sends every sound on a ducked bus with `fades.duck_ms`.

`open.rs`:
- `deferred` starts with a `SilentBackend`, spawns a thread named `"audio-open"` that runs `open` and sends its result on `std::sync::mpsc::sync_channel(1)`, and keeps the receiver. The thread's `JoinHandle` is dropped (detached): nothing ever joins it.
- `update` (in `pool.rs`, before anything else) does `try_recv`:
  - **`Ok(Ok(backend))`** swaps the backend in and `log::info!`s its name. Every live slot that is looping or a voice is replayed on the new backend with its current mix, keeping its slot and generation so handles stay valid. A voice is rebuilt with `Voice::with_params`, so its parameters carry over. Every live one-shot slot is freed, and the silent backend is dropped.
  - **`Ok(Err(reason))`** logs the reason and sets the notice to `NO_SOUND_NOTICE`.
  - **`Disconnected`** (the opener panicked) is treated like `Err`.

  Each of these clears the receiver.
- `opening()` is whether the receiver is still there. `take_notice` returns the notice once.
- `shutdown`, when not already shut down:
  1. logs `"audio: shutting down"`;
  2. if still opening, waits `recv_timeout(SHUTDOWN_WAIT)`, shutting down a backend that arrives in time and logging that the open was abandoned otherwise;
  3. stops every live sound with no fade;
  4. calls `Backend::shutdown`;
  5. replaces the backend with a `SilentBackend`;
  6. logs `"audio: shut down"`.

- [ ] **Step 4: Run them to see them pass**

Run: `cargo test -p hearken-audio > target/t4.txt 2>&1`
Expected: every test in the crate passes.

- [ ] **Step 5: Commit**

```bash
git add crates/hearken-audio
git commit -m "feat(audio): bed and music slots, pause all, the duck, and opening off the loop"
```

---

### Task 5: The BASS backend, and the M1 follow-ups in `hearken-bass-sys`

**Files:**
- Modify: `Cargo.toml` (workspace), `crates/hearken-bass-sys/src/loader.rs`, `crates/hearken-bass-sys/src/lib.rs` (export), `crates/hearken-bass-sys/src/safe/callback.rs`, `crates/hearken-bass-sys/tests/smoke.rs`
- Modify: `crates/hearken-audio/Cargo.toml`, `crates/hearken-audio/src/lib.rs`
- Create: `crates/hearken-audio/src/bass.rs`
- Test: `crates/hearken-audio/tests/bass.rs`

**Interfaces:**
- Consumes: Task 3's `Backend`, `Source`, `Channel`, `BackendError`, `Mix`; Task 1's `Voice`, `wav_bytes`; `hearken-bass-sys`'s `init`, `stream_create_mem_shared`, `callback_stream`, `channel_*`, `channel_set_fx`, `fx_set_bqf`, `channel_remove_fx`, `load_bass_fx`, `free`, `Api`.
- Produces:
  ```rust
  // hearken-bass-sys, exported beside LIBRARY_PATH_ENV
  pub fn set_game_library_env(var: &str);   // checked before HEARKEN_BASS_PATH; first call wins;
                                            // has no effect once the library has loaded
  // hearken-audio, behind the default feature "bass"
  pub struct BassBackend { /* channels, fades, whether bass_fx loaded */ }
  impl BassBackend {
      pub fn open(env_prefix: &str) -> Result<BassBackend, String>;              // the default device
      pub fn open_device(env_prefix: &str, device: i32) -> Result<BassBackend, String>;
      pub fn has_filter(&self, channel: Channel) -> bool;                        // for tests and diagnostics
  }
  impl Backend for BassBackend { /* name() is "BASS" */ }
  ```

- [ ] **Step 1: Write the failing tests**

In `crates/hearken-bass-sys/tests/smoke.rs`, after the other callback-stream tests:
```rust
#[test]
fn a_playing_callback_stream_dropped_while_bass_pulls_it_frees_cleanly() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let Some(_g) = bass() else { return };
    let pulls = std::sync::Arc::new(AtomicUsize::new(0));
    let seen = std::sync::Arc::clone(&pulls);
    let stream = callback_stream(44100, 1, 0, move |out| {
        seen.fetch_add(1, Ordering::SeqCst);
        out.fill(0.1);
        out.len()
    })
    .expect("create a playing stream");
    let handle = stream.handle();
    channel_play(handle, false).expect("play");
    channel_update(handle, 0).expect("make BASS pull it now");
    assert!(pulls.load(Ordering::SeqCst) > 0, "BASS never pulled the stream");
    drop(stream);
    let after = pulls.load(Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(channel_is_active(handle), BASS_ACTIVE_STOPPED);
    assert!(pulls.load(Ordering::SeqCst) <= after + 1, "at most the pull already in flight");
}
```
In `crates/hearken-bass-sys/src/safe/callback.rs`, a `#[cfg(test)] mod tests` for the clamp (Step 3 names the helper):
```rust
#[test]
fn a_float_read_never_asks_for_more_than_basss_length_field_holds() {
    assert_eq!(float_request_bytes(10), 40);
    assert_eq!(float_request_bytes(usize::MAX / 8), 0x0FFF_FFFC);
}
```
In `crates/hearken-bass-sys/src/loader.rs`'s test module:
```rust
#[test]
fn a_games_own_path_variable_is_searched_first() {
    let dir = std::env::temp_dir().join("hearken-game-bass-path-test");
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("HEARKEN_LOADER_TEST_BASS_PATH", &dir);
    set_game_library_env("HEARKEN_LOADER_TEST_BASS_PATH");
    assert_eq!(search_paths()[0], dir.join(library_file_name()));
}
```
`crates/hearken-audio/tests/bass.rs`:
```rust
#![cfg(feature = "bass")]
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use bass_sys::safe::channel_get_attribute;
use bass_sys::{BASS_ATTRIB_FREQ, BASS_ATTRIB_PAN, BASS_ATTRIB_VOL};
use hearken_audio::*;

static LOCK: Mutex<()> = Mutex::new(());

fn bass() -> Option<(MutexGuard<'static, ()>, BassBackend)> {
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    match BassBackend::open_device("HEARKEN_AUDIO_TEST", 0) {
        Ok(b) => Some((guard, b)),
        Err(e) => {
            eprintln!("hearken-audio: BASS unavailable here ({e}); skipping");
            None
        }
    }
}

fn sine(seconds: f32) -> Source {
    let n = (seconds * 44_100.0) as usize;
    let samples: Vec<f32> = (0..n).map(|i| (i as f32 * 0.06).sin() * 0.3).collect();
    Source::Clip(hearken_synth::wav_bytes(&samples, 44_100).into())
}

#[test]
fn a_one_shot_plays_and_is_reaped_after_it_ends() {
    let Some((_g, mut b)) = bass() else { return };
    let ch = b.play("sfx/test", sine(0.05), false, Mix::UNITY).expect("play");
    assert!(b.is_playing(ch));
    std::thread::sleep(Duration::from_millis(400));
    b.update();
    assert!(!b.is_playing(ch));
    b.shutdown();
}

#[test]
fn a_loop_plays_until_stopped_and_a_fade_ends_it_after_the_fade() {
    let Some((_g, mut b)) = bass() else { return };
    let ch = b.play("amb/test", sine(0.05), true, Mix::UNITY).expect("play");
    std::thread::sleep(Duration::from_millis(150));
    assert!(b.is_playing(ch), "a loop outlasts its clip");
    b.stop(ch, 100);
    b.update();
    assert!(b.is_playing(ch), "still fading");
    std::thread::sleep(Duration::from_millis(250));
    b.update();
    assert!(!b.is_playing(ch));
    b.shutdown();
}

#[test]
fn the_mix_becomes_basss_volume_pan_and_frequency() {
    let Some((_g, mut b)) = bass() else { return };
    let mix = Mix { gain: 0.5, pan: -0.5, pitch: 2.0, lowpass_hz: None, relative: None };
    let ch = b.play("sfx/test", sine(1.0), false, mix).expect("play");
    let h = ch as u32;
    assert!((channel_get_attribute(h, BASS_ATTRIB_VOL).unwrap() - 0.5).abs() < 1e-3);
    assert!((channel_get_attribute(h, BASS_ATTRIB_PAN).unwrap() + 0.5).abs() < 1e-3);
    assert!((channel_get_attribute(h, BASS_ATTRIB_FREQ).unwrap() - 88_200.0).abs() < 1.0);
    b.shutdown();
}

#[test]
fn a_low_pass_comes_and_goes_with_the_mix() {
    let Some((_g, mut b)) = bass() else { return };
    let ch = b.play("sfx/test", sine(1.0), true, Mix::UNITY).expect("play");
    let muffled = Mix { lowpass_hz: Some(800.0), ..Mix::UNITY };
    b.set_mix(ch, muffled, 0);
    if !bass_sys::fx::bass_fx_loaded() {
        eprintln!("bass_fx not loaded; the filter half of this test is skipped");
        b.shutdown();
        return;
    }
    assert!(b.has_filter(ch));
    b.set_mix(ch, Mix::UNITY, 0);
    assert!(!b.has_filter(ch));
    b.shutdown();
}

#[test]
fn bytes_that_are_not_sound_are_an_error_not_a_panic() {
    let Some((_g, mut b)) = bass() else { return };
    let err = b.play("sfx/bad", Source::Clip((&b"not audio"[..]).into()), false, Mix::UNITY);
    assert!(err.is_err());
    b.shutdown();
}

#[test]
fn a_live_voice_plays_and_stops() {
    use hearken_synth::fundsp::prelude32::*;
    fn tone(p: &hearken_synth::Params) -> Box<dyn AudioUnit> {
        Box::new(var(&p.get("hz")) >> sine())
    }
    let Some((_g, mut b)) = bass() else { return };
    let voice = hearken_synth::Voice::new(tone, &[("hz", 220.0)]);
    let ch = b.play("voice", Source::Voice(voice), true, Mix::UNITY).expect("play");
    assert!(b.is_playing(ch));
    b.stop(ch, 0);
    assert!(!b.is_playing(ch));
    b.shutdown();
}
```
`crates/hearken-audio/Cargo.toml` `[dev-dependencies]` gains `hearken-bass-sys.workspace = true` (the tests read attributes directly).

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p hearken-bass-sys > target/t5a.txt 2>&1`, then `cargo test -p hearken-audio --test bass > target/t5b.txt 2>&1`.
Expected: compile errors for `float_request_bytes`, `set_game_library_env` and `BassBackend`. The smoke test may already pass; it pins the behaviour the follow-up asked for.

- [ ] **Step 3: Implement the `hearken-bass-sys` follow-ups**

- `loader.rs`: `static GAME_ENV: OnceLock<String>`; `set_game_library_env` stores the name the first time. `search_paths` reads the game's variable, when set, before `LIBRARY_PATH_ENV`, so its loop now runs over two names. Export it from `lib.rs` beside `LIBRARY_PATH_ENV`.
- `callback.rs`: `fn float_request_bytes(samples: usize) -> DWORD` returns `(samples * 4).min(0x0FFF_FFFC)`, saturating. `channel_get_data_float` uses it, so a buffer over 256 MB can never set `BASS_DATA_*` flag bits.

Workspace `[workspace.dependencies]` gains `hearken-bass-sys = { path = "crates/hearken-bass-sys" }`.

- [ ] **Step 4: Implement `BassBackend`**

`crates/hearken-audio/Cargo.toml`:
```toml
[features]
default = ["bass"]
bass = ["dep:hearken-bass-sys"]

[dependencies]
hearken-bass-sys = { workspace = true, optional = true }
```
`lib.rs`: `#[cfg(feature = "bass")] mod bass; #[cfg(feature = "bass")] pub use bass::BassBackend;`.

`bass.rs`, with the module doc naming what it borrows from LB `a101970` (`bass3d.rs`: the BQF occlusion filter and its Q) and FF `c5d3ef8d` (`audio/bass.rs`: memory streams kept alive until they end).

**Opening** (`open_device`):
1. `set_game_library_env(&format!("{env_prefix}_BASS_PATH"))`.
2. `Api::get()`, which on failure is `Err(format!("BASS could not be loaded: {e}"))`.
3. `init(device, 44_100, 0)`, where `BASS_ERROR_ALREADY` counts as success and any other error is `Err(format!("no sound device ({})", err.name()))`.
4. `load_bass_fx(api.library_dir())`, remembering whether it loaded.
5. `log::info!` the BASS version and device.

`open` is `open_device(env_prefix, -1)`.

**Channels** are kept in `HashMap<Channel, Entry>` with
`struct Entry { stream: Owned, base_freq: f32, fx: Option<HFX>, looping: bool }` and
`enum Owned { Clip(Stream), Voice(CallbackStream) }`. The channel id is the stream handle as `u64`.

**Playing:**
- A clip becomes `stream_create_mem_shared(bytes, if looping { BASS_SAMPLE_LOOP } else { 0 })`; an error is `BackendError(err.name().into())`.
- A voice gets `voice.set_sample_rate(44_100.0)`, then `callback_stream(44_100, 1, 0, move |out| { voice.fill(out); out.len() })`.
- `base_freq` is `channel_frequency(h)`.
- Apply the mix with no ramp, then `channel_play(h, false)`.

**Applying a mix** (`ramp_ms` 0 sets, otherwise slides):
- Volume: `mix.gain.max(0.0)`, times `0.708` (-3 dB) when a low-pass is wanted but bass_fx did not load.
- Pan: `mix.pan.clamp(-1.0, 1.0)`.
- Frequency: set to `base_freq * mix.pitch`.
- `Some(hz)` attaches the `BASS_FX_BFX_BQF` effect if the channel has none and sets it to `BASS_BFX_BQF_LOWPASS`, `f_center = hz.clamp(20.0, 20_000.0)`, `f_q = 0.707`, `f_bandwidth = 0.0`, `l_channel = BASS_BFX_CHANALL`.
- `None` removes the effect if there is one.

**Stopping:** `stop(ch, 0)` calls `channel_stop` and drops the entry. `stop(ch, fade)` slides the volume to 0 over `fade` and records `(ch, Instant::now() + fade)` in `fading`.

**Housekeeping and queries:**
- `update` drops every fading entry past its deadline and every non-looping entry whose `channel_is_active` is `BASS_ACTIVE_STOPPED`.
- `is_playing` is whether the entry exists and is not `BASS_ACTIVE_STOPPED` (a paused channel counts).
- `pause(ch, true)` is `channel_pause`, and `pause(ch, false)` is `channel_play(h, false)`.

**Shutting down:** drop every entry (a `Stream` frees its handle), call `free()`, and log both steps. Nothing in `Drop` beyond what `Stream` and `CallbackStream` already do.

- [ ] **Step 5: Run the tests to see them pass**

Run: `cargo test -p hearken-bass-sys > target/t5a.txt 2>&1`, then `cargo test -p hearken-audio > target/t5b.txt 2>&1`.
Expected: all pass. The BASS tests say they skipped only if BASS cannot load; on this machine it can (`crates/hearken-bass-sys/vendor/windows-x86_64/`), so check the output shows them running.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/hearken-bass-sys crates/hearken-audio
git commit -m "feat(audio): the BASS backend; a per-game BASS path and a clamped float read"
```

---

### Task 6: `hearken-speech` — the gameplay-speech pacer

**Files:**
- Create: `crates/hearken-speech/src/pacer.rs`, `crates/hearken-speech/src/pacer/tests.rs`
- Modify: `crates/hearken-speech/src/lib.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces (FF's API, minus what is dropped below):
  ```rust
  pub fn monotonic_seconds() -> f64;
  #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
  pub enum EventPriority { Ambient, Important, Critical }
  #[derive(Clone, Copy, Debug, PartialEq)]
  pub struct PacerConfig {
      pub ambient_wait_s: f64,        // 3.0  (FF STALE_WAIT_S)
      pub important_wait_s: f64,      // 0.8  (FF's Route budget)
      pub base_utterance_s: f64,      // 0.4
      pub chars_per_s: f64,           // 13.0
      pub repeat_window_s: f64,       // 2.5
      pub recent_limit: usize,        // 256
      pub recent_memory_s: f64,       // 300.0
      pub rescue_once_window_s: f64,  // 30.0
      pub mostly_heard_fraction: f64, // 0.5
  }
  impl Default for PacerConfig { /* FF's values, above */ }
  pub type Clock = Box<dyn FnMut() -> f64>;
  pub type Valid = Box<dyn Fn() -> bool>;
  pub type Cut = (String, EventPriority);
  pub struct EventSpeechPacer { /* FF's fields minus receipts */ }
  impl EventSpeechPacer {
      pub fn new(config: PacerConfig) -> Self;
      pub fn with_clock(config: PacerConfig, clock: Clock) -> Self;
      pub fn config(&self) -> &PacerConfig;
      pub fn set_clock(&mut self, clock: Clock);
      pub fn wait_budget_s(&self, priority: EventPriority) -> f64;
      pub fn is_repeat(&mut self, text: &str, key: Option<&str>, force: bool, window: Option<f64>) -> bool;
      pub fn note_spoken(&mut self, text: &str, key: Option<&str>);
      pub fn forget_condition(&mut self, key: &str);
      pub fn is_silenced_repeat(&mut self, text: &str, key: Option<&str>, window: Option<f64>) -> bool;
      pub fn note_silenced(&mut self, text: &str, key: Option<&str>);
      pub fn take_flush_cut(&mut self) -> Option<Cut>;
      pub fn take_mostly_heard(&mut self) -> Option<String>;
      pub fn note_interrupt(&mut self, text: &str, priority: EventPriority, valid: Option<Valid>) -> Option<Cut>;
      pub fn note_queued(&mut self, text: &str, priority: EventPriority, valid: Option<Valid>);
      pub fn note_ahead(&mut self, text: &str);
      pub fn note_channel_purged(&mut self) -> Option<Cut>;
      pub fn busy(&mut self) -> bool;
      pub fn speaking_critical(&mut self) -> bool;
      pub fn would_start_stale(&mut self, text: &str, priority: EventPriority) -> bool;
      pub fn should_flush(&mut self, text: &str, priority: EventPriority, valid: Option<Valid>) -> bool;
      pub fn reset(&mut self);
      pub fn pause(&mut self);
      pub fn resume(&mut self);
  }
  ```

- [ ] **Step 1: Port the tests first**

Extract `crates/ff-core/src/speech_pacing/tests.rs` and the first test of `crates/ff-core/src/speech_pacing/tests/settings.rs` (`test_busy_follows_the_projection`) into `crates/hearken-speech/src/pacer/tests.rs`, then make these edits:
- `Route` → `Important` everywhere.
- `EventSpeechPacer::with_clock(clock)` → `EventSpeechPacer::with_clock(PacerConfig::default(), clock)`.
- Each `EventSpeechPacer::CONSTANT` becomes the matching field on `PacerConfig::default()` (`STALE_WAIT_S` → `ambient_wait_s`, and so on). `wait_budget_s(p)` becomes a method call on the pacer.
- Remove the `category` argument from every `note_interrupt`/`note_queued`/`track` call.
- Delete the tests about receipts or delivery status (`track_delivery`, `complete_delivery`, `delivery_status`, `delivery_pending`, `resume_delivery`), and the tests of the `Confirmation` exception. Count what is deleted and name it in the commit message body.
- Delete the ladder tests; Task 7 has its own.

The file's doc says it was ported from FF `c5d3ef8d` and lists what was left out.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p hearken-speech --lib pacer > target/t6.txt 2>&1`
Expected: compile errors (no `pacer` module yet).

- [ ] **Step 3: Port the pacer**

Extract `crates/ff-core/src/speech_pacing.rs` lines 1-1099 into `crates/hearken-speech/src/pacer.rs` and edit:
- **Keep** the module doc's account of the three failures and how the pacer answers them, with the driving examples replaced by neutral ones (a line said three times in half a second; a standing warning read out forever; the line that mattered buried behind chatter). Keep the dated tester references out; keep the reasoning.
- **Remove** `SpeechCategory`, `Disposition`, `DRIVING_SPEECH_*`, `DispositionRow`, `disposition_row`, `row_disposition`, `ladder_earcon`, `disposition_for`, `LADDER_EARCONS`, the `receipts` module and every method that used it (`track_delivery`, `complete_delivery`, `delivery_status`, `delivery_pending`, `resume_delivery`, `interrupt_deliveries`, and their calls in `reset`/`pause`/`resume`/`note_channel_purged`).
- **Rename** `EventPriority::Route` to `Important` and rewrite its doc generically: "the line the player needs to act on soon; it waits a moment behind chatter, then goes ahead of it".
- **Change `track`** to `track(text, priority, valid)`: drop the category and the `Confirmation` early return. Only `Important` and `Critical` lines are protected, as before.
- **Make the constants config:** replace the associated constants with `self.config.<field>`. `duration_s` becomes a method using `base_utterance_s` and `chars_per_s`. `wait_budget_s(&self, priority)` returns `ambient_wait_s` for Ambient, `important_wait_s` for Important, and `0.0` for Critical.
- **Add the config to the constructors:** `new(config)` uses `monotonic_seconds` as its clock, and `with_clock(config, clock)`.

`lib.rs`: `pub mod pacer;` and `pub use pacer::{Clock, Cut, EventPriority, EventSpeechPacer, PacerConfig, Valid, monotonic_seconds};`.

- [ ] **Step 4: Run them to see them pass**

Run: `cargo test -p hearken-speech > target/t6.txt 2>&1`
Expected: the ported pacer tests pass, and every existing `hearken-speech` test still passes.

- [ ] **Step 5: Commit**

```bash
git add crates/hearken-speech
git commit -m "feat(speech): the event-speech pacer, ported from Freight Fate"
```

---

### Task 7: `hearken-speech` — the generic verbosity table

**Files:**
- Create: `crates/hearken-speech/src/verbosity.rs`
- Modify: `crates/hearken-speech/src/lib.rs` (module, re-exports, `SpeechSettings::verbosity`)
- Test: `crates/hearken-speech/tests/verbosity.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  ```rust
  pub trait Category: Copy + Eq + std::hash::Hash + std::fmt::Debug + 'static {
      fn all() -> &'static [Self];
      fn id(self) -> &'static str;
  }
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
  pub enum NoCategory {}
  impl Category for NoCategory { /* all() is &[] */ }
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum Rule { Full, Terse, FirstTime, Earcon(&'static str), Silent }
  #[derive(Debug)]
  pub struct Level<C: 'static> { pub id: &'static str, pub label: &'static str, pub rules: &'static [(C, Rule)] }
  #[derive(Debug)]
  pub struct Verbosity<C: 'static> { pub levels: &'static [Level<C>] }  // Clone + Copy by hand, no bound on C
  impl<C: Category> Verbosity<C> {
      pub const NONE: Verbosity<C> = Verbosity { levels: &[] };
      pub fn is_empty(&self) -> bool;
      pub fn level(&self, id: &str) -> Option<&'static Level<C>>;   // that level, else the first, else None
      pub fn rule(&self, level_id: &str, category: C) -> Rule;       // Full when unlisted or no levels
      pub fn problems(&self) -> Vec<String>;                         // for a start-up log
  }
  // SpeechSettings gains:
  pub verbosity: String,   // a level id; "" means the game's first level
  ```

- [ ] **Step 1: Write the failing tests**

`crates/hearken-speech/tests/verbosity.rs`:
```rust
use hearken_speech::{Category, Level, NoCategory, Rule, SpeechSettings, Verbosity};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum C {
    Safety,
    Chatter,
}

impl Category for C {
    fn all() -> &'static [Self] {
        &[C::Safety, C::Chatter]
    }
    fn id(self) -> &'static str {
        match self {
            C::Safety => "safety",
            C::Chatter => "chatter",
        }
    }
}

static LEVELS: [Level<C>; 3] = [
    Level { id: "full", label: "Full", rules: &[(C::Safety, Rule::Full), (C::Chatter, Rule::Full)] },
    Level { id: "quiet", label: "Quiet", rules: &[(C::Safety, Rule::Full), (C::Chatter, Rule::Earcon("ui/tick"))] },
    Level { id: "odd", label: "Odd", rules: &[(C::Safety, Rule::Terse), (C::Safety, Rule::Silent)] },
];

fn table() -> Verbosity<C> {
    Verbosity { levels: &LEVELS }
}

#[test]
fn a_level_says_what_it_does_with_each_category() {
    assert_eq!(table().rule("quiet", C::Chatter), Rule::Earcon("ui/tick"));
    assert_eq!(table().rule("quiet", C::Safety), Rule::Full);
}

#[test]
fn an_unknown_level_is_the_first_and_an_unlisted_category_is_full() {
    assert_eq!(table().level("gone").map(|l| l.id), Some("full"));
    assert_eq!(table().level("").map(|l| l.id), Some("full"));
    assert_eq!(table().rule("odd", C::Chatter), Rule::Full);
}

#[test]
fn problems_name_the_gaps_and_the_duplicates() {
    let problems = table().problems();
    assert!(problems.iter().any(|p| p.contains("odd") && p.contains("chatter")), "{problems:?}");
    assert!(problems.iter().any(|p| p.contains("odd") && p.contains("safety") && p.contains("twice")), "{problems:?}");
    assert_eq!(problems.len(), 2);
}

#[test]
fn a_game_without_categories_has_no_levels_and_everything_is_full() {
    let none = Verbosity::<NoCategory>::NONE;
    assert!(none.is_empty());
    assert!(none.level("anything").is_none());
    assert!(NoCategory::all().is_empty());
}

#[test]
fn old_settings_without_a_verbosity_read_as_the_first_level() {
    let s: SpeechSettings = serde_json::from_str(r#"{"rate": 0.5}"#).unwrap();
    assert_eq!(s.verbosity, "");
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p hearken-speech --test verbosity > target/t7.txt 2>&1`
Expected: compile errors.

- [ ] **Step 3: Implement**

`verbosity.rs` holds what Produces lists, and its module doc explains that a game names its categories and levels and the engine applies them (spec, "Ladder"). `problems()` reports, in this form:
- a duplicate level id: `"level {id} appears twice"`;
- a category listed twice in one level: `"level {id} lists {category} twice"`;
- a category missing from a level: `"level {id} has no rule for {category}"`.

`rule` returns the first matching rule. `SpeechSettings` gains `pub verbosity: String` (`Default` is empty), documented as "the verbosity level's id; empty for the game's first level".

- [ ] **Step 4: Run them to see them pass**

Run: `cargo test -p hearken-speech > target/t7.txt 2>&1`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/hearken-speech
git commit -m "feat(speech): a verbosity table a game fills with its own categories"
```

---

### Task 8: `hearken-app` — audio in the context, the loop and the game

**Files:**
- Modify: `crates/hearken-app/Cargo.toml`, `crates/hearken-app/src/lib.rs`, `crates/hearken-app/src/game.rs`, `crates/hearken-app/src/ctx.rs`, `crates/hearken-app/src/app.rs`, `crates/hearken-app/src/run.rs`, `crates/hearken-app/src/testing.rs`, `crates/hearken-app/src/settings.rs`
- Create: `crates/hearken-app/src/sound.rs`
- Modify (to compile): `crates/hearken-app/tests/common/mod.rs`, `template/src/lib.rs`
- Test: `crates/hearken-app/tests/sound.rs`

**Interfaces:**
- Consumes: Tasks 3-5's `Audio`, `Bus`, `Sound`, `Vec3`, `Backend`, `SilentBackend`, `RecordingBackend`, `Recorder`, `MemoryClips`, `LooseClips`, `BassBackend`, `SpatialConfig`; Task 6's `PacerConfig`; Task 7's `Category`, `NoCategory`, `Verbosity`; Task 1's `UI_*` keys.
- Produces:
  ```rust
  // sound.rs
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
  pub enum UiSound { Open, Move, Select, Back, Tick, Error, Notify }
  #[derive(Clone, Debug, PartialEq)] pub struct UiSounds { /* [Option<&'static str>; 7] */ }
  impl Default for UiSounds { /* hearken_synth's UI_* keys */ }
  impl UiSounds { pub fn get(&self, s: UiSound) -> Option<&'static str>;
                  pub fn set(&mut self, s: UiSound, key: Option<&'static str>); }
  pub const BUS_LABELS: [(Bus, &str); 6] = [(Bus::MASTER, "Master"), (Bus::UI, "Interface"),
      (Bus::EFFECTS, "Effects"), (Bus::AMBIENCE, "Ambience"), (Bus::MUSIC, "Music"), (Bus::VOICE, "Voices")];
  #[derive(Clone, Debug)]
  pub struct SoundConfig {
      pub buses: Vec<(Bus, &'static str)>,  // the game's own, listed after the built-in six
      pub duck_buses: Vec<Bus>,             // [MUSIC, AMBIENCE]
      pub duck_level: f32,                  // 0.5
      pub earcon_duck_s: f64,               // 0.25
      pub pacer: Option<PacerConfig>,       // Some(default); None switches the pacer off
      pub spatial: SpatialConfig,
      pub ui: UiSounds,
  }
  impl Default for SoundConfig { /* the values above */ }
  // game.rs, new items on Game
  type Category: hearken_speech::Category;
  fn sound() -> SoundConfig { SoundConfig::default() }
  fn verbosity() -> Verbosity<Self::Category> { Verbosity::NONE }
  fn audio_backend() -> Option<Box<dyn Backend>> { None }
  fn occluded(_data: &Self::Data, _listener: Vec3, _pos: Vec3) -> bool { false }
  // ctx.rs, new on Ctx<G>
  pub audio: Audio,
  pub ui_sounds: UiSounds,
  pub fn install_audio(&mut self, audio: Audio);  // applies spatial, duck buses and saved volumes
  pub fn apply_audio_settings(&mut self);
  pub fn buses(&self) -> Vec<(Bus, &'static str)>; // BUS_LABELS then the game's
  pub fn play_ui(&mut self, sound: UiSound) -> Sound; // UI bus; Sound::NONE when set to nothing
  pub fn sound_config(&self) -> &SoundConfig;
  // settings.rs
  #[derive(Serialize, Deserialize, Clone, Debug, PartialEq)] #[serde(default)]
  pub struct AudioSettings { pub volumes: BTreeMap<String, u8>, pub duck_for_speech: bool }
  impl AudioSettings { pub fn volume(&self, bus: Bus) -> u8;          // saved (capped at 100) or the default
                       pub fn set_volume(&mut self, bus: Bus, percent: u8); }
  pub fn default_volume(bus: Bus) -> u8;   // 100 for MASTER, 80 for the rest (LB)
  // Settings gains: #[serde(default)] pub audio: AudioSettings
  // testing.rs, new on Headless<G>
  pub sounds: Recorder,
  pub fn played(&self) -> Vec<String>;   // keys since the last clear(); clear() now clears these too
  ```

- [ ] **Step 1: Write the failing tests**

`crates/hearken-app/tests/common/mod.rs`: `TestData` gains `pub occluded: bool`; `impl Game for TestGame` gains `type Category = hearken_app::NoCategory;` (Task 9 gives it real categories) and
```rust
fn occluded(data: &TestData, _listener: hearken_app::Vec3, _pos: hearken_app::Vec3) -> bool {
    data.occluded
}
```
`crates/hearken-app/tests/sound.rs`:
```rust
mod common;

use common::*;
use hearken_app::{Bus, Headless, Sound, UiSound, Vec3};

fn start() -> Headless<TestGame> {
    Headless::new(TestData::default(), Probe::new("root"))
}

#[test]
fn a_headless_game_records_what_it_plays() {
    let mut h = start();
    h.app.ctx.play_ui(UiSound::Move);
    assert_eq!(h.played(), ["ui/menu_move"]);
    h.clear();
    assert!(h.played().is_empty());
}

#[test]
fn a_game_points_an_earcon_at_its_own_sound_or_at_nothing() {
    let mut h = start();
    h.app.ctx.ui_sounds.set(UiSound::Move, Some("ui/my_tick"));
    h.app.ctx.play_ui(UiSound::Move);
    h.app.ctx.ui_sounds.set(UiSound::Move, None);
    assert_eq!(h.app.ctx.play_ui(UiSound::Move), Sound::NONE);
    assert_eq!(h.played(), ["ui/my_tick"]);
}

#[test]
fn saved_volumes_reach_the_buses_and_an_unsaved_one_is_eighty_percent() {
    let mut h = start();
    h.app.ctx.settings.audio.volumes.insert("music".into(), 40);
    h.app.ctx.apply_audio_settings();
    let audio = &h.app.ctx.audio;
    assert_eq!(audio.bus_volume(Bus::MUSIC), 0.4);
    assert_eq!(audio.bus_volume(Bus::EFFECTS), 0.8);
    assert_eq!(audio.bus_volume(Bus::MASTER), 1.0);
}

#[test]
fn the_games_occlusion_hook_is_asked_every_frame() {
    let mut h = start();
    h.app.ctx.audio.play_at("sfx/door", Vec3::new(0.0, 1.0, 0.0), Bus::EFFECTS);
    h.app.ctx.game.occluded = true;
    h.frame(0.016);
    let ch = h.sounds.channel_of("sfx/door").unwrap();
    assert_eq!(h.sounds.last_mix(ch).unwrap().lowpass_hz, Some(800.0));
}

#[test]
fn ending_the_run_stops_every_sound() {
    let mut h = start();
    h.app.ctx.audio.start_loop("amb/hum", None, Bus::AMBIENCE);
    h.app.ctx.quit();
    h.app.run(&mut hearken_app::HeadlessShell::default());
    assert!(h.sounds.calls().iter().any(|c| matches!(c, hearken_app::Call::Stop { .. })));
    assert!(h.app.ctx.audio.live().is_empty());
}
```
(Check `HeadlessShell`'s constructor in `shell.rs` and use it.)

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p hearken-app --test sound > target/t8.txt 2>&1`
Expected: compile errors.

- [ ] **Step 3: Implement**

`crates/hearken-app/Cargo.toml`:
```toml
[features]
default = ["bass"]
bass = ["hearken-audio/bass"]

[dependencies]
hearken-audio.workspace = true
hearken-synth.workspace = true
```
`lib.rs`:
- `pub mod sound;`
- re-export `hearken_audio::{Audio, Backend, BackendError, Bus, Call, Channel, ClipSource, Fades, Listener, LiveSound, LooseClips, MemoryClips, Mix, Recorder, RecordingBackend, SilentBackend, Sound, Source, SpatialConfig, Vec3}`, plus `BassBackend` under `#[cfg(feature = "bass")]`;
- `pub use hearken_synth as synth;`
- `pub use hearken_speech::{Category, EventPriority, Level, NoCategory, PacerConfig, Rule, Verbosity};`
- `pub use sound::{SoundConfig, UiSound, UiSounds, BUS_LABELS};`
- `pub use settings::{AudioSettings, default_volume};`

`game.rs`:
- Add the new items listed under Produces, each with a doc comment. `audio_backend`'s doc says it runs off the game loop, while the device opens.
- `ENV_PREFIX`'s doc lists the new variables: `<PREFIX>_AUDIO_BACKEND` (`silent` turns sound off), `<PREFIX>_SOUNDS` (the loose sounds folder) and `<PREFIX>_BASS_PATH`.

`ctx.rs`:
- `Ctx::new` sets `sound: G::sound()` (a private field), `ui_sounds: sound.ui.clone()`, and `audio: Audio::silent()`, then calls `install_audio(Audio::silent())` so saved volumes apply from the first frame.
- `install_audio` sets `audio.spatial`, calls `set_duck_buses`, stores the pool and runs `apply_audio_settings`, which sets each of `buses()` to `settings.audio.volume(bus) as f32 / 100.0`.
- `pub(crate) fn update_audio(&mut self)`:
  ```rust
  let listener = self.audio.listener().pos;
  let game = &self.game;
  self.audio.update(&|pos| G::occluded(game, listener, pos));
  if let Some(notice) = self.audio.take_notice() {
      self.say_queued(notice);
  }
  ```

`app.rs`:
- `step` calls `self.ctx.update_audio()` right after `self.ctx.speech.poll(dt)`.
- `run` ends with, in order: `G::on_exit`, `controller.shutdown()`, `audio.shutdown()`, `save_settings()`, `speech.shutdown()`.

`run.rs`: after `Ctx::new`, `ctx.install_audio(Audio::deferred(open_backend::<G>, Box::new(LooseClips::discover(G::ENV_PREFIX))))`. `open_backend::<G>()` chooses:
1. `SilentBackend` when `<PREFIX>_AUDIO_BACKEND` is `silent` (any case) or `SDL_AUDIODRIVER` is `dummy`;
2. else `G::audio_backend()` when it returns `Some`;
3. else a private `default_backend(prefix)`, which has two `#[cfg]` bodies: `BassBackend::open(prefix)` boxed, or `SilentBackend` without the feature.

`log::info!` which one was chosen.

`testing.rs`: `Headless::new` makes a `RecordingBackend`, installs `Audio::new(Box::new(backend), Box::new(MemoryClips::default()))`, and keeps the `Recorder` as `sounds`. `played()` is `sounds.played()`, and `clear()` also calls `sounds.clear()`.

`settings.rs`: `AudioSettings` and `default_volume` as listed; `volume` caps a saved value at 100.

`template/src/lib.rs`: `type Category = hearken_app::NoCategory;`.

- [ ] **Step 4: Run the focused tests**

Run: `cargo test -p hearken-app > target/t8.txt 2>&1`, then `cargo test -p hello-hearken > target/t8b.txt 2>&1`.
Expected: the new `sound.rs` tests pass, and every existing `hearken-app` and template test still passes.

- [ ] **Step 5: Commit**

```bash
git add Cargo.lock crates/hearken-app template/src/lib.rs
git commit -m "feat(app): audio in the context, opened off the loop, with saved bus volumes"
```

---

### Task 9: `hearken-app` — gameplay speech delivery: pacer, verbosity and duck

**Files:**
- Create: `crates/hearken-app/src/speech_delivery.rs`
- Modify: `crates/hearken-app/src/ctx.rs`, `crates/hearken-app/src/app.rs`, `crates/hearken-app/src/lib.rs`
- Modify: `crates/hearken-app/tests/common/mod.rs` (TestGame gets categories)
- Test: `crates/hearken-app/tests/delivery.rs`

**Interfaces:**
- Consumes: Task 6's `EventSpeechPacer`, `EventPriority`, `PacerConfig`, `Cut`, `Valid`; Task 7's `Category`, `Rule`, `Verbosity`, `Level`; Task 8's `SoundConfig`, `Ctx::audio`, `AudioSettings::duck_for_speech`, `TRANSCRIPT_TARGET`.
- Produces:
  ```rust
  pub struct SayEvent<C> { /* interrupt, priority, category, key, terse, force, valid */ }
  impl<C> SayEvent<C> {
      pub fn new() -> Self;                               // queued, no category
      pub fn interrupt(self, on: bool) -> Self;
      pub fn priority(self, priority: EventPriority) -> Self; // default: Critical if interrupting, else Ambient
      pub fn category(self, category: C) -> Self;
      pub fn key(self, key: &str) -> Self;                // a standing condition's key
      pub fn terse(self, text: &str) -> Self;             // what a Terse rule says instead
      pub fn force(self) -> Self;                         // a line the player asked for: skips the verbosity rules and repeat checks
      pub fn valid(self, still_true: impl Fn() -> bool + 'static) -> Self;
  }
  impl<C> Default for SayEvent<C>;
  impl<G: Game> Ctx<G> {
      pub fn say_event_with(&mut self, text: impl Into<String>, opts: SayEvent<G::Category>);
      pub fn say_event(&mut self, text: impl Into<String>, interrupt: bool); // now say_event_with(.., new().interrupt(interrupt))
      pub fn clear_condition(&mut self, key: &str);
      pub fn reset_first_times(&mut self);
      pub fn pause_event_speech(&mut self);   // pacer.pause(), speech.stop_event()
      pub fn resume_event_speech(&mut self);  // pacer.resume(), speech.stop_event()
      pub fn verbosity(&self) -> Verbosity<G::Category>;
      pub fn verbosity_level(&self) -> Option<&'static Level<G::Category>>; // the saved one, or the first
      pub fn speech_ducked(&self) -> bool;
  }
  ```

- [ ] **Step 1: Give the test game categories**

In `crates/hearken-app/tests/common/mod.rs`:
```rust
use hearken_app::{Category, Level, Rule, Verbosity};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TestCategory {
    Safety,
    Chatter,
    Hint,
}

impl Category for TestCategory {
    fn all() -> &'static [Self] {
        &[TestCategory::Safety, TestCategory::Chatter, TestCategory::Hint]
    }
    fn id(self) -> &'static str {
        match self {
            TestCategory::Safety => "safety",
            TestCategory::Chatter => "chatter",
            TestCategory::Hint => "hint",
        }
    }
}

pub static TEST_LEVELS: [Level<TestCategory>; 3] = [
    Level { id: "full", label: "Full", rules: &[
        (TestCategory::Safety, Rule::Full), (TestCategory::Chatter, Rule::Full), (TestCategory::Hint, Rule::FirstTime)] },
    Level { id: "quiet", label: "Quiet", rules: &[
        (TestCategory::Safety, Rule::Full), (TestCategory::Chatter, Rule::Earcon("ui/tick")), (TestCategory::Hint, Rule::Silent)] },
    Level { id: "minimal", label: "Minimal", rules: &[
        (TestCategory::Safety, Rule::Terse), (TestCategory::Chatter, Rule::Silent), (TestCategory::Hint, Rule::Silent)] },
];
```
`impl Game for TestGame`: `type Category = TestCategory;` and `fn verbosity() -> Verbosity<TestCategory> { Verbosity { levels: &TEST_LEVELS } }`.

- [ ] **Step 2: Write the failing tests**

`crates/hearken-app/tests/delivery.rs`:
```rust
mod common;

use common::*;
use hearken_app::{Bus, EventPriority, Headless, SayEvent};

fn start() -> Headless<TestGame> {
    Headless::new(TestData::default(), Probe::new("root"))
}

fn events(h: &Headless<TestGame>) -> Vec<String> {
    h.speech.borrow().event_lines()
}

fn say(h: &mut Headless<TestGame>, text: &str, opts: SayEvent<TestCategory>) {
    h.app.ctx.say_event_with(text, opts);
}

#[test]
fn an_event_is_spoken_on_the_event_voice_and_kept_in_the_transcript() {
    let mut h = start();
    say(&mut h, "Wind rising.", SayEvent::new());
    assert_eq!(events(&h), ["Wind rising."]);
    assert_eq!(h.app.ctx.transcript.last().map(String::as_str), Some("[event] Wind rising."));
}

#[test]
fn the_same_line_twice_in_a_moment_is_said_once_and_again_later() {
    let mut h = start();
    say(&mut h, "Bump.", SayEvent::new());
    say(&mut h, "Bump.", SayEvent::new());
    assert_eq!(events(&h), ["Bump."]);
    h.frame(3.0);
    say(&mut h, "Bump.", SayEvent::new());
    assert_eq!(events(&h), ["Bump.", "Bump."]);
}

#[test]
fn a_standing_condition_speaks_again_only_when_it_changes_or_is_cleared() {
    let mut h = start();
    say(&mut h, "Hull damaged.", SayEvent::new().key("hull"));
    h.frame(10.0);
    say(&mut h, "Hull damaged.", SayEvent::new().key("hull"));
    say(&mut h, "Hull badly damaged.", SayEvent::new().key("hull"));
    h.app.ctx.clear_condition("hull");
    h.frame(10.0);
    say(&mut h, "Hull badly damaged.", SayEvent::new().key("hull"));
    assert_eq!(events(&h), ["Hull damaged.", "Hull badly damaged.", "Hull badly damaged."]);
}

#[test]
fn at_quiet_chatter_is_its_earcon_and_no_words() {
    let mut h = start();
    h.app.ctx.settings.speech.verbosity = "quiet".into();
    say(&mut h, "A gull calls.", SayEvent::new().category(TestCategory::Chatter));
    assert!(events(&h).is_empty());
    assert_eq!(h.played(), ["ui/tick"]);
}

#[test]
fn terse_says_the_short_line_and_silent_says_and_plays_nothing() {
    let mut h = start();
    h.app.ctx.settings.speech.verbosity = "minimal".into();
    say(&mut h, "Brake now, the pier is close.", SayEvent::new().category(TestCategory::Safety).terse("Brake!"));
    say(&mut h, "A gull calls.", SayEvent::new().category(TestCategory::Chatter));
    assert_eq!(events(&h), ["Brake!"]);
    assert!(h.played().is_empty());
}

#[test]
fn a_first_time_line_waits_for_reset_first_times() {
    let mut h = start();
    let hint = || SayEvent::new().category(TestCategory::Hint);
    say(&mut h, "Press J to jump.", hint());
    h.frame(10.0);
    say(&mut h, "Press J to jump.", hint());
    h.app.ctx.reset_first_times();
    say(&mut h, "Press J to jump.", hint());
    assert_eq!(events(&h), ["Press J to jump.", "Press J to jump."]);
}

#[test]
fn a_line_the_player_asked_for_skips_the_verbosity_rules() {
    let mut h = start();
    h.app.ctx.settings.speech.verbosity = "quiet".into();
    say(&mut h, "Tide is high.", SayEvent::new().category(TestCategory::Chatter).force());
    assert_eq!(events(&h), ["Tide is high."]);
}

#[test]
fn a_saved_level_the_game_no_longer_has_is_its_first_level() {
    let mut h = start();
    h.app.ctx.settings.speech.verbosity = "gone".into();
    say(&mut h, "A gull calls.", SayEvent::new().category(TestCategory::Chatter));
    assert_eq!(events(&h), ["A gull calls."]);
    assert_eq!(h.app.ctx.verbosity_level().map(|l| l.id), Some("full"));
}

#[test]
fn the_duck_lowers_music_while_event_speech_talks_and_lifts_after() {
    let mut h = start();
    h.app.ctx.settings.audio.duck_for_speech = true;
    h.app.ctx.audio.play_music(Some("music/x"), true);
    let ch = h.sounds.channel_of("music/x").unwrap();
    say(&mut h, "Critical warning, the engine is on fire.", SayEvent::new().priority(EventPriority::Critical).interrupt(true));
    assert!(h.app.ctx.speech_ducked());
    assert!((h.sounds.last_mix(ch).unwrap().gain - 0.8 * 0.5).abs() < 1e-4);
    for _ in 0..10 {
        h.frame(1.0);
    }
    assert!(!h.app.ctx.speech_ducked());
    assert!((h.sounds.last_mix(ch).unwrap().gain - 0.8).abs() < 1e-4);
}

#[test]
fn with_the_duck_off_speech_leaves_the_music_alone() {
    let mut h = start();
    h.app.ctx.audio.play_music(Some("music/x"), true);
    h.sounds.clear();
    say(&mut h, "Hello.", SayEvent::new());
    assert!(!h.app.ctx.speech_ducked());
    assert!(h.sounds.calls().is_empty());
}

#[test]
fn pausing_event_speech_silences_the_event_voice() {
    let mut h = start();
    let before = h.speech.borrow().stop_event_calls();
    h.app.ctx.pause_event_speech();
    assert_eq!(h.speech.borrow().stop_event_calls(), before + 1);
}
```
Also, in the same file, a game with the pacer switched off. It has its own minimal `Game` impl, with `Category = NoCategory` and `fn sound() -> SoundConfig { SoundConfig { pacer: None, ..SoundConfig::default() } }`. The test shows that it says the same line twice in one frame, twice.

- [ ] **Step 3: Run them to see them fail**

Run: `cargo test -p hearken-app --test delivery > target/t9.txt 2>&1`
Expected: compile errors.

- [ ] **Step 4: Implement**

`speech_delivery.rs` holds `SayEvent`, the crate-private `Delivery<C>`, and an `impl<G: Game> Ctx<G>` block. Its module doc names FF `c5d3ef8d` `speech_delivery.rs` as the source and lists what was left out (review, receipts, Transitions, `SpokenMessage`).

```rust
pub(crate) struct Delivery<C: 'static> {
    pub(crate) pacer: Option<EventSpeechPacer>,
    pub(crate) verbosity: Verbosity<C>,
    pub(crate) first_times: HashSet<(String, String)>,
    pub(crate) ducked: bool,
    pub(crate) duck_until: f64,          // ctx.clock seconds; 0.0 when unset
    pub(crate) clock: Rc<Cell<f64>>,     // what the pacer reads
}
```
- **Setup:** `Ctx::new` builds it from `G::sound().pacer` and `G::verbosity()`. The pacer's clock is `move || clock.get()`, and the cell starts at `1000.0`. `App::step` ends, after `ctx.clock` advances, by setting it to `ctx.clock + 1000.0`, so a line spoken between frames sees the time `ctx.clock` shows; the fixed offset keeps game time clear of the `0.0` the pacer uses to mean "idle". Each of `verbosity.problems()` is logged once, at `warn`.
- **Other methods:**
  - `ctx.stop_speech()` (F2) also calls `pacer.reset()`.
  - `clear_condition` calls `pacer.forget_condition`, and `reset_first_times` clears `first_times`.
  - `verbosity_level` is `verbosity.level(&settings.speech.verbosity)`.

`say_event_with` has this order; the pacer calls follow FF's lines 583-715:
```rust
pub fn say_event_with(&mut self, text: impl Into<String>, opts: SayEvent<G::Category>) {
    let text = text.into();
    if text.trim().is_empty() {
        return;
    }
    let SayEvent { mut interrupt, priority, category, key, terse, force, valid } = opts;
    let key = key.as_deref();
    let rule = match category {
        Some(c) if !force => self.delivery.verbosity.rule(&self.settings.speech.verbosity, c),
        _ => Rule::Full,
    };
    let text = match rule {
        Rule::Silent | Rule::Earcon(_) => {
            if let Some(pacer) = self.delivery.pacer.as_mut() {
                if pacer.is_silenced_repeat(&text, key, None) {
                    return;
                }
                pacer.note_silenced(&text, key);
            }
            if let Rule::Earcon(sound) = rule {
                self.audio.play(sound, Bus::UI);
                self.engage_duck(None);
            }
            log::info!(target: TRANSCRIPT_TARGET, "[verbosity] not spoken: {text}");
            return;
        }
        Rule::FirstTime => {
            if !self.delivery.first_times.insert((key.unwrap_or("").to_string(), text.clone())) {
                return;
            }
            text
        }
        Rule::Terse => terse.unwrap_or(text),
        Rule::Full => text,
    };
    // No pacer: straight to the event voice.
    // With one: is_repeat (skipped when `force`), the stale-ambient drop, note_spoken,
    // then FF's separate-voice / shared-voice branches: note_interrupt, or should_flush
    // with take_flush_cut delivered first, or note_queued with stop_main on the shared
    // voice. Then record "[event] {text}", engage the duck, log take_mostly_heard, and
    // requeue a cut behind this line.
}
```
Write the pacer half as that comment says: port FF's lines 650-715, replacing `self.settings.sapi_events` with `self.speech.has_separate_event_voice()` and dropping `review`, `receipt` and `message_log`. Arrange the pacer borrow so it ends before `deliver_cut`. `deliver_cut((text, priority), interrupt)` is FF's `deliver_cut_event` (lines 422-445) without `handed_back` or `resume_delivery`, and it records `[event] {text}` in the transcript, because the line is spoken again.

The duck:
- `engage_duck(line: Option<&str>)` does nothing unless `settings.audio.duck_for_speech`. Otherwise it sets `ducked`, sets `audio.set_duck(sound.duck_level)`, and moves `duck_until` to the later of its current value and one of:
  - `clock + sound.earcon_duck_s` for an earcon (`None`);
  - `clock + base_utterance_s + chars / chars_per_s` for a line when there is no pacer, using the default `PacerConfig` numbers, since nothing else says when the line ends.
- `pub(crate) fn update_duck(&mut self)`, called by `App::step` right after `update_audio`, restores `audio.set_duck(1.0)` and clears the state once `clock >= duck_until` and the pacer (if any) is not `busy()`.

`App::step` order after this task: input, the game's events, the top screen's update, `speech.poll`, `update_audio`, `update_duck`, the stack changes, `ctx.clock += dt`, then the pacer clock.

`lib.rs` re-exports `SayEvent`.

- [ ] **Step 5: Run the focused tests**

Run: `cargo test -p hearken-app > target/t9.txt 2>&1`
Expected: `delivery.rs` passes; every other `hearken-app` test still passes.

- [ ] **Step 6: Commit**

```bash
git add crates/hearken-app
git commit -m "feat(app): gameplay speech through the pacer, the game's verbosity levels and the duck"
```

---

### Task 10: Earcons in the built-in screens, and pause

**Files:**
- Modify: `crates/hearken-app/src/menu.rs`, `crates/hearken-app/src/screens/settings.rs` (`on_escape` only), `crates/hearken-app/src/screens/bindings.rs`, `crates/hearken-app/src/screens/text_entry.rs`, `crates/hearken-app/src/screens/timed_message.rs`, `crates/hearken-app/src/screens/pause.rs`
- Test: `crates/hearken-app/tests/earcons.rs`

**Interfaces:**
- Consumes: Task 8's `Ctx::play_ui`, `UiSound`, `Ctx::audio`; Task 9's `pause_event_speech`, `resume_event_speech`; Task 4's `pause_all`, `resume_all`.
- Produces:
  ```rust
  impl<S: 'static, G: Game> MenuItem<S, G> {
      pub fn select_sound(self, key: Option<&'static str>) -> Self; // None silences the row's select sound
      pub fn silent_adjust(self) -> Self;                           // Left/Right play nothing (the row plays its own)
  }
  pub fn menu_enter<S: Menu>(state: &mut S, ctx: &mut Ctx<S::Game>); // Menu::enter's default body
  ```

Where each sound plays:

| Screen | Event | Sound |
|---|---|---|
| Any menu | Entered (`Menu::enter` default, through `menu_enter`) | Open, before the title is spoken |
| Any menu | Up, Down, Home, End, or a first-letter jump that lands | Move |
| Any menu | Enter or Space on a row | The row's select sound (default Select), before its action runs |
| Any menu | Left or Right on an adjustable row | Move, unless `silent_adjust` |
| Any menu | Escape, default `on_escape` | Back, then `pop` |
| Settings | Escape | save, Back, pop |
| Shortcuts | A rebinding lands, or reset all | Select |
| Shortcuts | A key or button that is taken or reserved | Error |
| Text entry | A character typed | Tick |
| Text entry | The field is full (still says "The field is full.") | Error |
| Text entry | Backspace at the start, Left at the start, Right at the end, Home at the start, End at the end | Error, and nothing is said (today these are silent) |
| Text entry | Escape | Back, then pop |
| Timed message | Entered | Notify, before the message |
| Pause | Entered | `audio.pause_all()` and `pause_event_speech()`, then `menu_enter` |
| Pause | Exited | `audio.resume_all()` and `resume_event_speech()` |

- [ ] **Step 1: Write the failing tests**

`crates/hearken-app/tests/earcons.rs`:
```rust
mod common;

use common::*;
use hearken_app::menu::{Menu, MenuCore, MenuItem};
use hearken_app::screens::{PauseScreen, TextEntryScreen, TimedMessage};
use hearken_app::{Bus, Ctx, Headless, InputEvent, Key};

struct Rows {
    menu: MenuCore<Self, TestGame>,
    level: i32,
}

impl Menu for Rows {
    type Game = TestGame;
    fn name(&self) -> &'static str {
        "rows"
    }
    fn menu(&self) -> &MenuCore<Self, TestGame> {
        &self.menu
    }
    fn menu_mut(&mut self) -> &mut MenuCore<Self, TestGame> {
        &mut self.menu
    }
    fn build_items(&mut self, _ctx: &mut Ctx<TestGame>) -> Vec<MenuItem<Self, TestGame>> {
        vec![
            MenuItem::new("Alpha", |_s: &mut Self, _c: &mut Ctx<TestGame>| {}),
            MenuItem::new("Door", |_s: &mut Self, _c: &mut Ctx<TestGame>| {}).select_sound(Some("sfx/door")),
            MenuItem::new("Hush", |_s: &mut Self, _c: &mut Ctx<TestGame>| {}).select_sound(None),
            MenuItem::inert("Level").adjustable(|s: &mut Self, _c: &mut Ctx<TestGame>, d| s.level += d),
            MenuItem::inert("Quiet level").adjustable(|s: &mut Self, _c: &mut Ctx<TestGame>, d| s.level += d).silent_adjust(),
        ]
    }
}

fn rows() -> Headless<TestGame> {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.app.ctx.push(Rows { menu: MenuCore::new("Rows"), level: 0 });
    h.frame(0.0);
    h
}

#[test]
fn a_menu_opens_moves_selects_and_goes_back_with_its_sounds() {
    let mut h = rows();
    assert_eq!(h.played(), ["ui/menu_open"]);
    h.clear();
    h.press(Key::Down);
    h.press(Key::Up);
    h.press(Key::Return);
    h.press(Key::Escape);
    assert_eq!(h.played(), ["ui/menu_move", "ui/menu_move", "ui/menu_select", "ui/menu_back"]);
}

#[test]
fn a_row_can_have_its_own_select_sound_or_none() {
    let mut h = rows();
    h.clear();
    h.press(Key::Down);
    h.press(Key::Return);
    h.press(Key::Down);
    h.press(Key::Return);
    assert_eq!(h.played(), ["ui/menu_move", "sfx/door", "ui/menu_move"]);
}

#[test]
fn left_and_right_tick_unless_the_row_plays_its_own() {
    let mut h = rows();
    h.press(Key::End);
    h.press(Key::Up); // Level
    h.clear();
    h.press(Key::Right);
    assert_eq!(h.played(), ["ui/menu_move"]);
    h.press(Key::Down); // Quiet level
    h.clear();
    h.press(Key::Right);
    assert!(h.played().is_empty());
}

#[test]
fn a_first_letter_that_matches_nothing_is_silent() {
    let mut h = rows();
    h.clear();
    h.press(Key::Z);
    assert!(h.played().is_empty());
}

#[test]
fn typing_ticks_and_every_edge_is_an_error_sound() {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.app.ctx.push(TextEntryScreen::<TestGame>::new("Name", "Name", |_c: &mut Ctx<TestGame>, _t: &str| {}).max_len(1));
    h.frame(0.0);
    h.clear();
    h.press(Key::Backspace);
    h.press(Key::Left);
    h.press(Key::Home);
    assert_eq!(h.played(), ["ui/error", "ui/error", "ui/error"]);
    assert!(h.spoken().is_empty(), "the sound says it; words would repeat it");
    h.clear();
    h.send(InputEvent::text_key(Key::A, 'a'));
    h.send(InputEvent::text_key(Key::B, 'b'));
    assert_eq!(h.played(), ["ui/tick", "ui/error"]);
    assert_eq!(h.spoken().last().map(String::as_str), Some("The field is full."));
    h.clear();
    h.press(Key::Right);
    h.press(Key::End);
    h.press(Key::Escape);
    assert_eq!(h.played(), ["ui/error", "ui/error", "ui/menu_back"]);
}

#[test]
fn a_timed_message_arrives_with_its_notify_sound() {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.clear();
    h.app.ctx.push(TimedMessage::<TestGame>::new("Ready", "Get ready.", "Starting soon.", 1.0, |_c: &mut Ctx<TestGame>| {}));
    h.frame(0.0);
    assert_eq!(h.played(), ["ui/notify"]);
}

#[test]
fn pause_holds_the_games_sounds_and_event_speech_and_resume_brings_the_sounds_back() {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.app.ctx.audio.start_loop("amb/hum", None, Bus::AMBIENCE);
    let hum = h.sounds.channel_of("amb/hum").unwrap();
    let stops = h.speech.borrow().stop_event_calls();
    h.app.ctx.push(PauseScreen::<TestGame>::new("# Help", |_c: &mut Ctx<TestGame>| {}));
    h.frame(0.0);
    assert!(h.sounds.calls().contains(&hearken_app::Call::Pause { channel: hum, paused: true }));
    assert_eq!(h.speech.borrow().stop_event_calls(), stops + 1);
    h.press(Key::Return); // Resume
    assert!(h.sounds.calls().contains(&hearken_app::Call::Pause { channel: hum, paused: false }));
}
```
Use the real constructor signatures from `text_entry.rs` and `timed_message.rs`, and the test helper this crate already uses to send a key that types a character (see `tests/entry.rs`); the names above (`InputEvent::text_key`) stand for whatever that helper is.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p hearken-app --test earcons > target/t10.txt 2>&1`
Expected: compile errors, then failures.

- [ ] **Step 3: Implement**

The table above is the specification.
- In `menu.rs`, `MenuItem` gains `select: Option<Option<&'static str>>` (outer `None`: the `UiSounds` default) and `adjust_sound: bool` (default `true`).
- `activate` plays the select sound through `ctx.audio.play(key, Bus::UI)` for an override, or `ctx.play_ui(UiSound::Select)` for the default.
- `menu_enter` is `ctx.play_ui(UiSound::Open); state.refresh(ctx, false); state.announce(ctx);`, and `Menu::enter`'s default calls it.
- `PauseScreen` overrides `Menu::enter` and `Menu::exit` as the table says.
- `timed_message.rs`'s module doc drops "Its arrival sound comes with audio in M2".
- Existing tests assert speech, not sound, so they should not move. Run them all to be sure.

- [ ] **Step 4: Run the focused tests**

Run: `cargo test -p hearken-app > target/t10.txt 2>&1`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/hearken-app
git commit -m "feat(app): earcons in menus, text entry, shortcuts and timed messages; pause holds sound"
```

---

### Task 11: Settings rows for volume, verbosity and the duck, and the game's own rows

**Files:**
- Modify: `crates/hearken-app/src/screens/settings.rs`, `crates/hearken-app/src/game.rs`, `crates/hearken-app/src/lib.rs`
- Modify: existing tests that reach a Settings row by counting Down presses (`crates/hearken-app/tests/screens.rs`, `tests/bindings.rs`, `tests/controller.rs`, `template/tests/walkthrough.rs`; grep for `SettingsScreen` and `Settings` navigation)
- Modify: `crates/hearken-app/tests/common/mod.rs` (TestGame gets an `engine` bus)
- Test: `crates/hearken-app/tests/sound_settings.rs`

**Interfaces:**
- Consumes: Task 8's `AudioSettings`, `Ctx::buses`, `apply_audio_settings`, `UiSounds`; Task 9's `verbosity`, `verbosity_level`; Task 10's `silent_adjust`.
- Produces:
  ```rust
  pub struct SettingsRow<G: Game> { pub id: String, pub item: MenuItem<SettingsScreen<G>, G> }
  impl<G: Game> SettingsRow<G> { pub fn new(id: impl Into<String>, item: MenuItem<SettingsScreen<G>, G>) -> Self; }
  // on Game:
  fn settings_rows(_ctx: &Ctx<Self>, rows: Vec<SettingsRow<Self>>) -> Vec<SettingsRow<Self>> { rows }
  ```
  Built-in row ids, in order: `speech_rate` (when the voice can change rate), `verbosity` (when the game has categories and levels), `duck`, `volume.master`, `volume.ui`, `volume.effects`, `volume.ambience`, `volume.music`, `volume.voice`, `volume.<game bus>` for each of `SoundConfig::buses`, `menu_positions`, `keyboard`, `controller_buttons`, `controller`.

Row texts:

| Row | Label | Left/Right | Enter |
|---|---|---|---|
| Volume | `"{label} volume: {n} percent"` | ±5, clamped 0..=100; saved to `settings.audio`, applied live; plays the Move earcon on that bus (on UI for master) | nothing |
| Verbosity | `"Speech detail: {level label}"` | previous or next level, wrapping; saves the level's id | next level |
| Duck | `"Lower game sound under speech: on"` or `"...: off"` | toggles | toggles |

Help lines:
- Volume: `"Left and Right change how loud {label lowercased} sounds are."` Master's reads `"Left and Right change how loud the whole game is."`
- Verbosity: `"How much the game says while you play. Left and Right change it."`
- Duck: `"Whether music and background sound get quieter while the game speaks. Enter, Left or Right changes it."`

- [ ] **Step 1: Write the failing tests**

`TestGame` gains `fn sound() -> SoundConfig { SoundConfig { buses: vec![(Bus("engine"), "Engine")], ..SoundConfig::default() } }`.

`crates/hearken-app/tests/sound_settings.rs`:
```rust
mod common;

use common::*;
use hearken_app::screens::SettingsScreen;
use hearken_app::{Bus, Headless, Key, Settings};

fn settings() -> Headless<TestGame> {
    let mut h = Headless::new(TestData::default(), Probe::new("root"));
    h.app.ctx.push(SettingsScreen::<TestGame>::new());
    h.frame(0.0);
    h.clear();
    h
}

fn row_labels(h: &Headless<TestGame>) -> Vec<String> {
    h.app.top_lines().into_iter().skip(1).map(|l| l[2..].to_string()).collect()
}

#[test]
fn the_sound_rows_come_after_speech_and_include_the_games_own_bus() {
    let h = settings();
    let rows = row_labels(&h);
    let start = rows.iter().position(|r| r.starts_with("Speech detail")).expect("a verbosity row");
    assert_eq!(&rows[start..start + 9], [
        "Speech detail: Full",
        "Lower game sound under speech: off",
        "Master volume: 100 percent",
        "Interface volume: 80 percent",
        "Effects volume: 80 percent",
        "Ambience volume: 80 percent",
        "Music volume: 80 percent",
        "Voices volume: 80 percent",
        "Engine volume: 80 percent",
    ]);
}

#[test]
fn a_volume_row_moves_by_five_clamps_and_is_heard_on_its_own_bus() {
    let mut h = settings();
    h.press(Key::M); // Master volume
    h.press(Key::M); // Music volume
    h.clear();
    h.press(Key::Right);
    assert_eq!(h.app.ctx.settings.audio.volume(Bus::MUSIC), 85);
    assert!((h.app.ctx.audio.bus_volume(Bus::MUSIC) - 0.85).abs() < 1e-4);
    assert_eq!(h.played(), ["ui/menu_move"]);
    let ch = h.sounds.channel_of("ui/menu_move").unwrap();
    assert!((h.sounds.last_mix(ch).unwrap().gain - 0.85).abs() < 1e-4, "played through the music bus");
    assert!(h.spoken()[0].starts_with("Music volume: 85 percent."));
    for _ in 0..5 {
        h.press(Key::Right);
    }
    assert_eq!(h.app.ctx.settings.audio.volume(Bus::MUSIC), 100);
}

#[test]
fn the_verbosity_row_steps_through_the_games_levels_and_wraps() {
    let mut h = settings();
    h.press(Key::S); // from Speech rate, the first row, to Speech detail
    assert!(h.spoken().last().unwrap().starts_with("Speech detail: Full"));
    h.press(Key::Right);
    assert_eq!(h.app.ctx.settings.speech.verbosity, "quiet");
    h.press(Key::Left);
    h.press(Key::Left);
    assert_eq!(h.app.ctx.settings.speech.verbosity, "minimal");
}

#[test]
fn the_duck_row_toggles() {
    let mut h = settings();
    h.press(Key::L);
    h.press(Key::Return);
    assert!(h.app.ctx.settings.audio.duck_for_speech);
    assert!(h.spoken().last().unwrap().starts_with("Lower game sound under speech: on"));
}

#[test]
fn a_hand_edited_file_with_odd_sound_values_loads_quietly_and_keeps_what_it_does_not_know() {
    let dir = tempfile::tempdir().unwrap();
    let paths = hearken_app::Paths::at(dir.path());
    std::fs::write(paths.settings_file(),
        r#"{"audio": {"volumes": {"music": 250, "gone": 30}}, "speech": {"verbosity": "gone"}}"#).unwrap();
    let (loaded, notice) = Settings::<()>::load(&paths);
    assert_eq!(notice, None);
    assert_eq!(loaded.audio.volume(Bus::MUSIC), 100);
    loaded.save(&paths).unwrap();
    let text = std::fs::read_to_string(paths.settings_file()).unwrap();
    assert!(text.contains("\"gone\": 30"), "a bus the game dropped stays in the file");
}
```
Also in `sound_settings.rs`, a second game type whose `settings_rows` removes `menu_positions` and appends its own `SettingsRow::new("difficulty", MenuItem::inert("Difficulty: normal"))`. Assert that the rows no longer contain "Menu positions", that the last row is "Difficulty: normal", and that moving the speech rate row to the end still reads it back through the voice (`h.speech.borrow().previews()` gains an entry).

A `NoCategory` game shows no "Speech detail" row. Assert it with the same second game type.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p hearken-app --test sound_settings > target/t11.txt 2>&1`
Expected: failures.

- [ ] **Step 3: Implement**

- `SettingsScreen::build_items` builds `Vec<SettingsRow<G>>` in the order above, passes it through `G::settings_rows(&*ctx, rows)`, keeps the ids in `row_ids: Vec<String>`, and returns the items.
- `speak_adjusted` previews through the voice when `row_ids[menu.index] == "speech_rate"`, replacing the `index == 0` check.
- The volume closures capture `(bus, label)` by value.
- The verbosity row reads `ctx.verbosity()` and `ctx.verbosity_level()`.
- `SettingsScreen::new()` is unchanged. `SettingsRow` is re-exported from `hearken_app::screens`.
- The rows moved, so update the tests that reach a row by counting Down presses to jump by first letter instead (K for Keyboard shortcuts, C for Controller buttons and Controller, M for Menu positions and the volumes). The template's `count_can_move_to_another_key` is one of them.

- [ ] **Step 4: Run the focused tests**

Run: `cargo test -p hearken-app > target/t11.txt 2>&1`, then `cargo test -p hello-hearken > target/t11b.txt 2>&1`.
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/hearken-app template/tests
git commit -m "feat(app): volume, verbosity and duck rows in Settings, and rows a game adds or hides"
```

---

### Task 12: The template makes sound; follow-ups; the full gates

**Files:**
- Modify: `template/src/lib.rs`, `template/src/practice.rs`, `template/help.md`, `template/tests/walkthrough.rs`
- Create: `template/src/sound.rs`
- Modify: `docs/superpowers/notes/m1-followups.md`, `docs/superpowers/specs/2026-09-26-hearken-engine-design.md` (status line)

**Interfaces:**
- Consumes: everything above through `hearken_app` (`Audio` via `ctx.audio`, `Bus`, `Vec3`, `Sound`, `synth::fundsp`, `synth::Params`).
- Produces:
  ```rust
  pub enum HelloAction { Count, Throttle }  // Throttle: the Up arrow; pad X; id "throttle", label "Engine"
  pub fn engine(p: &hearken_app::synth::Params) -> Box<dyn hearken_app::synth::fundsp::audiounit::AudioUnit>;
  ```

- [ ] **Step 1: Write the failing tests**

Add to `template/tests/walkthrough.rs`:
```rust
use hearken_app::{InputEvent, Key};

fn into_practice() -> Headless<HelloGame> {
    let mut h = start();
    h.press(Key::Return); // Play
    h.clear();
    h
}

#[test]
fn the_title_ticks_as_you_move() {
    let mut h = start();
    h.clear();
    h.press(Key::Down);
    assert_eq!(h.played(), ["ui/menu_move"]);
}

#[test]
fn the_practice_room_runs_an_engine_and_a_beacon_pings_as_it_circles() {
    let mut h = into_practice();
    assert!(h.app.ctx.audio.live().iter().any(|s| s.params.is_some()), "the engine voice runs");
    let mut places = Vec::new();
    for _ in 0..40 {
        h.frame(0.05);
    }
    for call in h.sounds.calls() {
        if let hearken_app::Call::Play { key, mix, .. } = call {
            if key == "sfx/beacon" {
                places.push(mix.relative.expect("a placed sound"));
            }
        }
    }
    assert!(places.len() >= 2, "a ping every 0.8 seconds: {places:?}");
    assert!((places[0] - places[1]).length() > 1.0, "the beacon moved between pings: {places:?}");
}

#[test]
fn holding_the_engine_key_revs_it_and_letting_go_idles_it() {
    let mut h = into_practice();
    h.send(InputEvent::key(Key::Up));
    for _ in 0..30 {
        h.frame(0.05);
    }
    let throttle = |h: &Headless<HelloGame>| {
        h.app.ctx.audio.live().iter().find_map(|s| s.params.as_ref()?.value("throttle")).unwrap()
    };
    assert!(throttle(&h) > 0.5);
    h.send(InputEvent::key_up(Key::Up));
    for _ in 0..30 {
        h.frame(0.05);
    }
    assert!(throttle(&h) < 0.2);
}

#[test]
fn leaving_the_practice_room_stops_its_engine() {
    let mut h = into_practice();
    h.press(Key::Escape);
    h.press(Key::End);
    h.press(Key::Up); // Return to the title
    h.press(Key::Return);
    assert!(h.app.ctx.audio.live().iter().all(|s| s.params.is_none()));
}
```
Update the two tests that assert the practice room's announcement to the new line:
`"The practice room. A beacon circles you. Space counts, hold the Up arrow to rev the engine, Escape pauses."`
After rebinding Count to C it reads `"The practice room. A beacon circles you. C counts, hold the Up arrow to rev the engine, Escape pauses."`
Use the real key-up constructor from `input.rs`; `InputEvent::key_up` stands for it.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p hello-hearken > target/t12.txt 2>&1`
Expected: failures.

- [ ] **Step 3: Implement**

- `sound.rs` is the template's own recipe, so a new game sees how to write one:
  ```rust
  use hearken_app::synth::fundsp::prelude32::*;
  use hearken_app::synth::Params;

  /// A low engine hum whose pitch follows `throttle`, 0 to 1.
  pub fn engine(p: &Params) -> Box<dyn AudioUnit> {
      Box::new((var(&p.get("throttle")) * 110.0 + 55.0) >> saw() >> lowpass_hz(700.0, 0.8) * 0.3)
  }
  ```
- `Practice` gains the following fields: `engine: Sound`, `throttle: f32`, `beacon_angle: f32` and `ping_in: f64`.
  - `enter` starts `ctx.audio.start_voice(engine, &[("throttle", 0.0)], None, Bus::EFFECTS)`, sets `ping_in = 0.8` and says the new line.
  - `update(dt)`:
    - The angle advances by `dt * 0.2 * TAU` (one lap in five seconds).
    - The throttle eases toward 1 while `ctx.action_held(HelloAction::Throttle)` and toward 0 otherwise, at `(target - t) * (dt * 3.0).min(1.0)`, and goes to `ctx.audio.set_voice_param(engine, "throttle", t)`.
    - Every 0.8 s it plays `ctx.audio.play_at("sfx/beacon", Vec3::new(4.0 * a.sin(), 4.0 * a.cos(), 0.0), Bus::EFFECTS)`.
  - `exit` stops the engine with a 200 ms fade.
  - `resume` keeps its line and stays silent about sound: pause already resumed it.
- `help.md` gains a short "Sound" section describing the practice room's beacon and engine, and that missing sounds play a stand-in.
- `m1-followups.md`: remove the four "Carry into M2 (audio)" items, which this milestone did. Add under "Deliberately left": "Pause does not pause one-shots started after it (menu earcons play normally); a game that needs a frozen world stops its own loops."
- The spec's status line: `M1 and M1b built from this; M2 approved 2026-09-27`.

- [ ] **Step 4: Run the template's tests**

Run: `cargo test -p hello-hearken > target/t12.txt 2>&1`
Expected: all pass.

- [ ] **Step 5: The full gates, once each**

Run, one at a time, each redirected to a file and read:
- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test --workspace --locked` (in the background; expect several minutes)

Expected: all three clean. A failure far from this milestone's files is exactly what this run is for: fix it at its cause and rerun only the failing test file, then this full run once more.

- [ ] **Step 6: Commit**

```bash
git add template docs
git commit -m "feat(template): a practice room you can hear: an engine voice and a circling beacon"
```
