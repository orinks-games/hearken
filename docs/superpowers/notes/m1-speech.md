# M1 research: extracting `hearken-speech` from Freight Fate

Source: `C:\Users\joshu\Freight-fate` at commit `c5d3ef8d` (branch `dev`).
Comparison: `C:\Users\joshu\gh-projects\lucky-break` at `a101970` (branch `main`).
All paths below are relative to `C:\Users\joshu\Freight-fate\crates\` unless
stated otherwise. `FF` = Freight Fate, `LB` = Lucky Break.

Status: COMPLETE.

---

## 1. Files involved

### 1a. The speech stack proper (`freight-fate/src/`)

| Path | Lines | Purpose |
|---|---|---|
| `freight-fate/src/speech.rs` | 306 | Module root. `SpeechSink` trait, constants (`REFRESH_INTERVAL_S`, `EVENT_BACKEND`, `UIA_LAST_RESORT_PRIORITY`, `NO_SPEECH_ENV`, `SPEECH_BACKEND_ENV`, `PREVIEW_FEATURES`), `PreviewFeature`, `apply_speech_settings` (the only FF-`Settings` coupling). Re-exports. |
| `freight-fate/src/speech/backend.rs` | 585 | `VoiceBackend` / `VoiceRegistry` traits, `VoiceFeatures`, Prism implementations (`PrismRegistry`, `PrismVoice`, private `PrismShared`, `BackendInstances`), selection policy (`pick_backend`, `pick_backend_gated`, `pick_event_backend`, `usable`, `narrator_running`, `preserve_backend_default_pitch`). Contains both `unsafe` blocks. |
| `freight-fate/src/speech/live.rs` | 678 | `Speech`: the Prism-backed `SpeechSink` (`!Send`), `SpeechConfig`. Reads env vars. Backend re-detection, braille-only, retry-on-failure. |
| `freight-fate/src/speech/threaded.rs` | 869 | `ThreadedSpeech`: worker-thread `SpeechSink` wrapping any `Box<dyn SpeechSink>` built by a factory on the worker. Bounded queue, coalescing, snapshot, watchdog, respawn. |
| `freight-fate/src/speech/capture.rs` | 538 | Test doubles: `CaptureSpeech` (+ `CaptureProfile`, `SpokenEntry`, `SpeechChannel`, `ConfigureCall`) and `NullSpeech`. |
| `freight-fate/src/speech/fakes.rs` | 252 | Backend-level fakes, compiled into the lib (not `cfg(test)`): `FakeVoice`, `FakeVoiceState`, `FakeRegistry`. |
| `freight-fate/src/speech/backend/recovery_tests.rs` | 128 | `#[cfg(test)]` unit tests of `BackendInstances` (4 tests). |
| `freight-fate/src/speech/threaded/recovery_tests.rs` | 306 | `#[cfg(test)]` respawn/abandon tests (2 tests + rig). |
| `freight-fate/src/speech/threaded/tests.rs` | 598 | `#[cfg(test)]` worker tests with a `StubSink` (10 tests). |

### 1b. The delivery layer (FF app shell)

| Path | Lines | Purpose |
|---|---|---|
| `freight-fate/src/app/speech_delivery.rs` | 930 | `impl GameContext { say, say_with, say_event, say_event_with, ... }`: rung/ladder gate, anti-backlog pacer driving, cut-line requeue, audio duck, transcript logging, message-log review. `Say`, `SayEvent`, `Spoken`, `IntoSpoken`, `TRANSCRIPT_TARGET`. Entirely `impl GameContext`. |

### 1c. `ff-core` modules the stack depends on

| Path | Lines | Used by | Notes |
|---|---|---|---|
| `ff-core/src/speech_pacing.rs` | 1101 | `speech.rs:74` (re-export), `speech_delivery.rs:15-17` | `EventSpeechPacer`, `EventPriority`, `SpeechCategory`, `Disposition`, rung table, `LADDER_EARCONS`, `monotonic_seconds`, `Clock`, `Valid`, `Cut`. std-only. |
| `ff-core/src/speech_pacing/receipts.rs` | 92 | `speech_pacing.rs:62-64` | `DeliveryStatus` (pub), `DeliveryReceipts` (pub(super)). std-only, generic. |
| `ff-core/src/speech_pacing/tests.rs` | 895 | cfg(test) | Pure pacer tests with a `FakeClock`. |
| `ff-core/src/speech_pacing/tests/settings.rs` | 221 | cfg(test) | Rung-table / settings tests (`tests.rs:895` `mod settings;`). |
| `ff-core/src/speech_text.rs` | 1292 | `capture.rs:10`, `speech_delivery.rs:18` | Only `SpokenMessage` (lines 68-171, ~105 lines) + `terse_silent` (173-177) are generic. Everything from line 179 on is trucking copy (hazard calls, stop callouts, tolls, cruise, achievements, chatter). Line 32 imports `crate::sim::trip_models::OpenSide` (trucking). |
| `ff-core/src/settings.rs` | 1030 | `speech.rs:63`, `speech_delivery.rs` via `GameContext.settings` | `Settings` (macro-generated, `settings_fields!` at 265). Speech fields at 532-582; methods 781-800. |
| `ff-core/src/message_log.rs` | 689 | `speech_delivery.rs:13` | `MessageCategory::{General, Event}`; FF review log. Coupling. |
| `ff-core/src/sound_catalog.rs` | 941 | `speech_delivery.rs:14` | `entry_by_name` for ladder earcons. Coupling. |
| `ff-core/src/spoken_advice.rs` | 112 | **not used by the speech stack** | Hint retirement keyed on control bindings; depends on `crate::achievements`. Trucking/FF. Leave behind. |
| `ff-core/src/pyfmt.rs` | 461 | **not used by the speech stack** | Python-compatible number formatting. Not needed. |

`grep` confirmed: no file in `speech.rs`, `speech/`, `speech_delivery.rs`,
`speech_pacing*`, or `speech_text.rs` references `spoken_advice` or `pyfmt`.

### 1d. Other FF files that touch the stack (consumers, not extracted)

- `freight-fate/src/audio.rs` -- `trait Audio` (520), `set_speech_duck` (566), `play_with` (547), `SPEECH_DUCK_LEVEL` (221), `EARCON_DUCK_S` (227).
- `freight-fate/src/app/context.rs` -- `GameContext` struct (130-222), `top_paces_main_speech` (459-463), `apply_speech` (807-809), `use crate::speech::{apply_speech_settings, SpeechSink};` (55).
- `freight-fate/src/app.rs` -- `ThreadedSpeech::spawn()` (374), `new_headless` (386), `request_refresh` on focus (748), `speech.poll(dt)` per frame (796), `update_speech_duck()` per frame (835), `speech.shutdown()` (1000), env check (360), `pub use speech_delivery::{IntoSpoken, Say, SayEvent, Spoken, TRANSCRIPT_TARGET};` (50).
- `freight-fate/src/app/testing.rs` -- `TestApp`, `SharedCapture(pub Rc<RefCell<CaptureSpeech>>)` (357) adapter, `main_calls`/`event_calls` (301-308), sets `FREIGHT_FATE_NO_SPEECH=1` (158).
- `freight-fate/src/agent_server/ears.rs` -- `TeeSpeech` impl of `SpeechSink` (82-86), `NullSpeech` (505, 730).
- `freight-fate/src/playtest/harness.rs:45` -- `use crate::speech::{SpeechChannel, SpokenEntry};`
- `freight-fate/src/main.rs` -- `CaptureSpeech` (49, 674), diagnostic using `narrator_running, usable, PrismRegistry, VoiceRegistry, pick_backend` (387-396), env set (640, 670).
- `freight-fate/src/states/main_menu/settings_actions.rs` (541-598), `settings.rs:413`, `settings_items.rs:284,305` -- settings menu uses `event_backend_options`, `supports_braille`, `voice_names`, `say_adjustment_preview`, `supports_rate`.
- `freight-fate/build.rs` (149 lines) -- Prism delay-load and static-link plumbing; see section 10.

---

## 2. Public traits and types

### 2a. `SpeechSink` (verbatim, `freight-fate/src/speech.rs:153-266`)

```rust
/// What `GameContext` needs from a speech object: the public surface of the
/// Python `Speech` class, method for method.
///
/// Two channels: `say` is the main voice (menus, info keys, everything the
/// player asked for), `say_event` the dedicated event voice (road events,
/// warnings) that falls back to the main channel when no separate voice is
/// bound. `interrupt` cuts off the previous utterance on that channel.
///
/// The "Speech is now using X." line a backend switch produces is spoken by
/// the object itself, straight through the new main voice and bypassing the
/// message log -- exactly what `Speech.refresh(announce=True)` did in Python,
/// where it called its own `say`. No hook and no returned list, because the
/// caller never saw that line in Python either; a test double that wants to
/// see it records what `say` received.
pub trait SpeechSink {
    /// Speak (and braille, where supported) `text` on the main channel.
    fn say(&mut self, text: &str, interrupt: bool);

    /// Speak on the dedicated event voice (SAPI), so the player's screen
    /// reader cannot talk over it; falls back to the main channel.
    fn say_event(&mut self, text: &str, interrupt: bool);

    /// Silence in-progress main speech without cutting off event speech.
    fn stop_main(&mut self);

    /// Silence in-progress event speech without cutting off main speech.
    fn stop_event(&mut self);

    /// Silence any in-progress speech on both channels.
    fn stop(&mut self);

    /// Periodic health check, driven every frame by the game loop.
    fn poll(&mut self, dt: f64);

    /// Make the next `poll` re-detect immediately (the window regained focus).
    fn request_refresh(&mut self);

    /// Whether a main voice is bound right now.
    fn available(&self) -> bool;

    /// Registry name of the main voice; `"none"` when there is none.
    fn backend_name(&self) -> String;

    /// Whether events currently have their own backend. Without one,
    /// `say_event` falls back to the main channel, so events and menu speech
    /// share a single voice and can cut each other.
    fn has_separate_event_voice(&self) -> bool;

    /// Registry name of the event voice; `"none"` when events share the main
    /// voice.
    fn event_backend_name(&self) -> String;

    /// Whether any bound voice honors the in-game rate setting.
    fn supports_rate(&self) -> bool;

    /// Whether any bound voice honors the in-game pitch setting.
    fn supports_pitch(&self) -> bool;

    /// Whether any bound voice honors the in-game volume setting.
    fn supports_volume(&self) -> bool;

    /// Whether the dedicated event voice honors the in-game rate setting.
    fn event_supports_rate(&self) -> bool;

    /// Usable backends that can serve as a separate event voice (the
    /// controllable software voices other than the main one), in registry
    /// order.
    fn event_backend_options(&self) -> Vec<String>;

    /// Choose which backend speaks driving events (`None` = the main voice).
    /// The name is a preference: when it is not on this machine the best
    /// available separate software voice is used instead.
    fn select_event_backend(&mut self, name: Option<&str>);

    /// Installed voice names from the first backend that lets us pick one;
    /// empty when none does.
    fn voice_names(&self) -> Vec<String>;

    /// Push speech parameters to every backend that supports them; `None`
    /// leaves that parameter alone. Values are remembered so a backend swap
    /// mid-session can re-apply them to the new voice.
    fn configure(
        &mut self,
        rate: Option<f64>,
        pitch: Option<f64>,
        volume: Option<f64>,
        voice: Option<&str>,
    );

    /// Send every line to the main voice's braille display and speak nothing
    /// (Settings > Speech, "Output: braille only"): menus, readouts, and the
    /// driving events that would otherwise go to the separate event voice.
    /// Honoured only while the bound main voice can braille
    /// (`supports_braille`); with any other voice speech carries on
    /// unchanged, so the switch can never leave a player with nothing.
    fn set_braille_only(&mut self, on: bool);

    /// Whether the main voice can put text on a braille display by itself
    /// (NVDA and JAWS can; SAPI, OneCore and Narrator cannot).
    fn supports_braille(&self) -> bool;

    /// Speak a settings preview through the voice affected by `setting`
    /// (`"speech_rate"`, `"speech_pitch"`, `"speech_volume"`,
    /// `"speech_voice"`). `false` when no bound voice can be adjusted that way
    /// or the setting is not voice-adjustable.
    fn say_adjustment_preview(&mut self, setting: &str, text: &str, interrupt: bool) -> bool;

    /// Re-detect which screen reader or voice should be speaking. `true` when
    /// the main voice changed; with `announce` the new voice says so.
    fn refresh(&mut self, announce: bool) -> bool;

    /// Release the backends and context. Safe to call more than once.
    fn shutdown(&mut self);
}
```

Notes: 22 methods, no default impls, **no `Send` supertrait** (required:
`live::Speech` is `!Send` and lives in the worker as `Box<dyn SpeechSink>`).
The trait signature itself has no trucking types; only doc comments say
"driving events", "road events", "SAPI". `say_adjustment_preview`'s
`setting` strings are FF settings keys (`"speech_rate"` etc.; see
`PREVIEW_FEATURES`).

### 2b. Other public items in `speech.rs`

```rust
pub mod backend;                         // 65
pub mod capture;                         // 66
pub mod fakes;                           // 67
pub mod live;                            // 68
pub mod threaded;                        // 69
pub use ff_core::speech_pacing::{EventPriority, EventSpeechPacer};   // 74
pub use backend::{
    narrator_running, pick_backend, pick_backend_gated, pick_event_backend,
    preserve_backend_default_pitch, usable, BackendId, PrismRegistry, PrismVoice, VoiceBackend,
    VoiceFeatures, VoiceRegistry,
};                                                                   // 76-80
pub use capture::{
    CaptureProfile, CaptureSpeech, ConfigureCall, NullSpeech, SpeechChannel, SpokenEntry,
};                                                                   // 81-83
pub use live::{Speech, SpeechConfig};                                // 84
pub use threaded::ThreadedSpeech;                                    // 85

pub const REFRESH_INTERVAL_S: f64 = 3.0;                             // 90
pub const EVENT_BACKEND: &str = "SAPI";                              // 96
pub const UIA_LAST_RESORT_PRIORITY: i32 = -1;                        // 105
pub const NO_SPEECH_ENV: &str = "FREIGHT_FATE_NO_SPEECH";            // 108
pub const SPEECH_BACKEND_ENV: &str = "FREIGHT_FATE_SPEECH_BACKEND";  // 111
pub const PREVIEW_FEATURES: [(&str, PreviewFeature); 4] = [          // 115-120
    ("speech_rate", PreviewFeature::Rate),
    ("speech_pitch", PreviewFeature::Pitch),
    ("speech_volume", PreviewFeature::Volume),
    ("speech_voice", PreviewFeature::Voice),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewFeature { Rate, Pitch, Volume, Voice }               // 124-130
impl PreviewFeature {
    pub fn for_setting(setting: &str) -> Option<Self>                // 135
    pub fn supported_by(self, features: VoiceFeatures) -> bool       // 143
}

pub fn apply_speech_settings(sink: &mut dyn SpeechSink, settings: &mut Settings)  // 284-306
```

`apply_speech_settings` body (verbatim, 284-306) -- the one place settings meet the sink:

```rust
pub fn apply_speech_settings(sink: &mut dyn SpeechSink, settings: &mut Settings) {
    let preference = settings.sapi_events.then(|| settings.event_backend.clone());
    sink.select_event_backend(preference.as_deref());
    if settings.sapi_events {
        let options = sink.event_backend_options();
        if !options.is_empty() && !options.contains(&settings.event_backend) {
            log::info!(
                "Event speech backend {} is not on this machine; using {}",
                settings.event_backend,
                options[0]
            );
            settings.event_backend = options[0].clone();
        }
    }
    let voice = (!settings.speech_voice.is_empty()).then(|| settings.speech_voice.clone());
    sink.configure(
        Some(settings.speech_rate),
        Some(settings.speech_pitch),
        Some(settings.speech_volume),
        voice.as_deref(),
    );
    sink.set_braille_only(settings.braille_only);
}
```

### 2c. `speech/backend.rs`

```rust
pub type BackendId = u64;                                   // 20
pub type SpeechError = prismer::Error;                      // 23

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VoiceFeatures {                                  // 31-46
    pub is_supported_at_runtime: bool,
    pub supports_output: bool,
    pub supports_speak: bool,
    pub supports_braille: bool,
    pub supports_stop: bool,
    pub supports_set_rate: bool,
    pub supports_set_pitch: bool,
    pub supports_set_volume: bool,
    pub supports_set_voice: bool,
    pub supports_count_voices: bool,
    pub supports_get_voice_name: bool,
}
impl VoiceFeatures {
    pub const SPEAKING: VoiceFeatures = ...;                // 51
    pub const ADJUSTABLE: VoiceFeatures = ...;              // 67
    pub const BRAILLING: VoiceFeatures = ...;               // 80
    pub const fn selects_voices(self) -> bool               // 86
}
impl From<prismer::Features> for VoiceFeatures              // 91-109

pub trait VoiceBackend {                                    // 115-133
    fn name(&self) -> String;
    fn features(&self) -> VoiceFeatures;
    fn output(&mut self, text: &str, interrupt: bool) -> Result<(), SpeechError>;
    fn speak(&mut self, text: &str, interrupt: bool) -> Result<(), SpeechError>;
    fn braille(&mut self, text: &str) -> Result<(), SpeechError>;
    fn stop(&mut self) -> Result<(), SpeechError>;
    fn set_rate(&mut self, rate: f64) -> Result<(), SpeechError>;
    fn set_pitch(&mut self, pitch: f64) -> Result<(), SpeechError>;
    fn set_volume(&mut self, volume: f64) -> Result<(), SpeechError>;
    fn voices_count(&self) -> Result<usize, SpeechError>;
    fn voice_name(&self, index: usize) -> Result<String, SpeechError>;
    fn set_voice(&mut self, index: usize) -> Result<(), SpeechError>;
}

pub trait VoiceRegistry {                                   // 137-151
    fn backend_count(&self) -> usize;
    fn id_at(&self, index: usize) -> Option<BackendId>;
    fn id_by_name(&self, name: &str) -> Option<BackendId>;
    fn name_of(&self, id: BackendId) -> Option<String>;
    fn priority_of(&self, id: BackendId) -> i32;
    fn acquire(&self, id: BackendId) -> Result<Box<dyn VoiceBackend>, SpeechError>;
    fn settle(&self) {}
}
impl fmt::Debug for dyn VoiceBackend                        // 153-159

pub struct PrismRegistry { shared: Rc<PrismShared> }        // 165-167
impl PrismRegistry {
    pub fn new() -> Result<Self, SpeechError>               // 260
    pub fn new_fresh() -> Result<Self, SpeechError>         // 277
}
impl VoiceRegistry for PrismRegistry                        // 291-328
pub struct PrismVoice { backend: Rc<RefCell<prismer::Backend<'static>>>, _owner: Rc<PrismShared> }  // 331-336
impl VoiceBackend for PrismVoice                            // 338-389

pub fn usable(backend: &dyn VoiceBackend) -> bool                                  // 404
#[cfg(windows)] pub fn narrator_running() -> bool                                  // 416-452
#[cfg(not(windows))] pub fn narrator_running() -> bool                             // 455-458
pub fn pick_backend(
    ctx: &dyn VoiceRegistry,
    override_name: Option<&str>,
) -> Option<Box<dyn VoiceBackend>>                                                 // 476-481
pub fn pick_backend_gated(
    ctx: &dyn VoiceRegistry,
    override_name: Option<&str>,
    narrator_probe: fn() -> bool,
) -> Option<Box<dyn VoiceBackend>>                                                 // 485-543
pub fn pick_event_backend(
    ctx: &dyn VoiceRegistry,
    main_backend: Option<&dyn VoiceBackend>,
    name: &str,
) -> Option<Box<dyn VoiceBackend>>                                                 // 554-575
pub fn preserve_backend_default_pitch(backend: &dyn VoiceBackend, value: f64) -> bool  // 582-585
```

Private but important: `struct PrismShared { instances: BackendInstances<prismer::Backend<'static>>, ctx: Box<prismer::Prism> }` (175-180, field order = drop order); `struct BackendInstances<T>` (183-229, one native instance per backend id per registry; works around a Prism 0.18.2 OneCore handle leak); `PrismShared::open` (235-254, the `transmute`); `fn non_empty` (393-399, empty text -> `prismer::Error::InvalidParam`).

Hard-coded backend names in policy: `"UIA"` (521, 599 in live.rs), `"narrator.exe"` (443), `"onecore"`/`"one_core"` (584).

### 2d. `speech/live.rs`

```rust
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpeechConfig {                                   // 24-30
    pub rate: Option<f64>,
    pub pitch: Option<f64>,
    pub volume: Option<f64>,
    pub voice: Option<String>,
}
pub struct Speech { /* private: backend, event_backend, ctx, override_name, event_pref,
                       config, braille_only, refresh_timer, narrator_probe */ }  // 43-59
impl Default for Speech                                     // 61-65
impl Speech {
    pub fn new() -> Self                                    // 73
    pub fn new_after_wedge() -> Self                        // 81
    pub fn disabled() -> Self                               // 115
    pub fn with_registry(ctx: Box<dyn VoiceRegistry>, override_name: Option<String>) -> Self   // 135
    pub fn from_parts(
        ctx: Option<Box<dyn VoiceRegistry>>,
        backend: Option<Box<dyn VoiceBackend>>,
        event_backend: Option<Box<dyn VoiceBackend>>,
    ) -> Self                                               // 164-168
    pub fn set_narrator_probe(&mut self, probe: fn() -> bool)                 // 184
    pub fn set_main_backend(&mut self, backend: Option<Box<dyn VoiceBackend>>)   // 190
    pub fn set_event_backend(&mut self, backend: Option<Box<dyn VoiceBackend>>)  // 195
    pub fn config(&self) -> &SpeechConfig                   // 200
}
impl SpeechSink for Speech                                  // 331-678
```

Private free fn `stream_player_speech(text: &str)` (16-20) prints to stdout when `FREIGHT_FATE_STREAM_TRANSCRIPT` is non-empty; called from `say` (504) and `say_event` (532) on success.

### 2e. `speech/threaded.rs`

```rust
pub struct ThreadedSpeech { /* private: commands, snapshot, heartbeat, abandoned, ready,
    factory, replay, respawn_after_s, respawns, max_respawns, shutting_down, wedged,
    wedge_after_s, dropped_lines */ }                        // 305-332
impl ThreadedSpeech {
    pub fn spawn() -> Self                                   // 337-345
    pub fn spawn_with<F>(factory: F) -> Self
    where
        F: Fn(bool) -> Box<dyn SpeechSink> + Send + Sync + 'static,   // 351-380
}
impl SpeechSink for ThreadedSpeech                           // 675-861
```

Private: `enum Command` (80-115), `struct Snapshot` (119-132), `fn coalesce` (139-169), `macro_rules! query` (173-184), `fn publish` (186-206), `fn publish_status` (210-237), `fn publish_after_probe` (249-275), `type SinkFactory = dyn Fn(bool) -> Box<dyn SpeechSink> + Send + Sync;` (280), `struct Worker` (283-290), `type ConfigureArgs` (293), `struct Replay` (297-302). `#[cfg(test)] fn set_wedge_after_s` (630-633), `#[cfg(test)] fn set_respawn` (637-641).

`ThreadedSpeech::spawn` hard-wires `super::live::Speech::new()` / `new_after_wedge()` (338-344). No `Drop` impl; no `JoinHandle` kept (thread is detached).

### 2f. `speech/capture.rs`

```rust
pub type ConfigureCall = (Option<f64>, Option<f64>, Option<f64>, Option<String>);   // 16

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpeechChannel { Main, Event }                        // 20-23
impl SpeechChannel { pub const fn tag(self) -> &'static str } // 27  ("main" / "event")

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpokenEntry {                                      // 37-44
    pub sequence: u64,
    pub channel: SpeechChannel,
    pub text: String,
    pub interrupt: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CaptureProfile {                                   // 58-70
    pub available: bool,
    pub backend_name: String,
    pub event_backend_name: Option<String>,
    pub supports_rate: bool,
    pub supports_pitch: bool,
    pub supports_volume: bool,
    pub event_supports_rate: bool,
    pub event_backend_options: Vec<String>,
    pub voice_names: Vec<String>,
}
impl CaptureProfile {
    pub fn headless() -> Self                                 // 74  (available=false, "none", no options)
    pub fn full_voice() -> Self                               // 89  ("NVDA" + "SAPI", options ["OneCore","SAPI"], voices ["David","Zira"])
}
impl Default for CaptureProfile                               // 104 (= headless)

#[derive(Debug)]
pub struct CaptureSpeech { /* private: entries, terse, prefix, profile, event_pref,
    stop_main_calls, stop_event_calls, stop_calls, previews, configure_calls,
    refresh_requests, shutdown_calls */ }                     // 111-127
impl Default for CaptureSpeech                                // 129
impl CaptureSpeech {
    pub fn new() -> Self                                      // 138
    pub fn full_voice() -> Self                               // 144 (event_pref = Some(EVENT_BACKEND))
    pub fn with_profile(profile: CaptureProfile) -> Self      // 150
    pub fn terse(mut self) -> Self                            // 169
    pub fn with_prefix(mut self, prefix: &str) -> Self        // 175
    pub fn set_terse(&mut self, terse: bool)                  // 180
    pub fn is_terse(&self) -> bool                            // 184
    pub fn profile(&self) -> &CaptureProfile                  // 188
    pub fn profile_mut(&mut self) -> &mut CaptureProfile      // 192
    pub fn say_message(&mut self, message: &SpokenMessage, interrupt: bool)        // 213  <-- ff_core::speech_text
    pub fn say_event_message(&mut self, message: &SpokenMessage, interrupt: bool)  // 221  <-- ff_core::speech_text
    pub fn entries(&self) -> &[SpokenEntry]                   // 229
    pub fn lines(&self) -> Vec<String>                        // 234
    pub fn channel_lines(&self, channel: SpeechChannel) -> Vec<String>   // 242
    pub fn main_lines(&self) -> Vec<String>                   // 251
    pub fn event_lines(&self) -> Vec<String>                  // 256
    pub fn calls(&self, channel: SpeechChannel) -> Vec<(String, bool)>   // 262
    pub fn tagged(&self) -> Vec<(&'static str, String)>       // 272
    pub fn transcript(&self) -> String                        // 281 (event lines prefixed "[event] ", joined "\n")
    pub fn transcript_lines(&self) -> Vec<String>             // 286
    pub fn contains(&self, needle: &str) -> bool              // 297
    pub fn clear(&mut self)                                   // 302 (NB: does not reset shutdown_calls)
    pub fn drain(&mut self) -> Vec<SpokenEntry>               // 313
    pub fn stop_main_calls(&self) -> u32                      // 317
    pub fn stop_event_calls(&self) -> u32                     // 321
    pub fn stop_calls(&self) -> u32                           // 326
    pub fn previews(&self) -> &[(String, String, bool)]       // 331
    pub fn configure_calls(&self) -> &[ConfigureCall]         // 336
    pub fn refresh_requests(&self) -> u32                     // 340
    pub fn shutdown_calls(&self) -> u32                       // 344
    pub fn event_preference(&self) -> Option<&str>            // 349
}
impl SpeechSink for CaptureSpeech                             // 354-472

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NullSpeech;                                        // 477-478
impl SpeechSink for NullSpeech                                // 480-538
```

Capture behaviour to preserve: empty text is dropped (356, 362); `has_separate_event_voice` = `event_pref.is_some() && profile.event_backend_name.is_some()` (393-395); `supports_braille` always `false` (437-439); `set_braille_only` no-op (432-435); `say_adjustment_preview` returns `false` for a non-preview key or empty text, else records and returns `true` (456-463); `refresh` always `false`.

### 2g. `speech/fakes.rs`

```rust
#[derive(Debug, Default)]
pub struct FakeVoiceState {                                   // 18-39
    pub name: String,
    pub priority: i32,
    pub features: VoiceFeatures,
    pub voices: Vec<String>,
    pub rate: Option<f64>,
    pub pitch: Option<f64>,
    pub volume: Option<f64>,
    pub voice: Option<usize>,
    pub spoken: Vec<(String, bool)>,
    pub brailled: Vec<String>,
    pub stop_calls: u32,
    pub fail_output: bool,
    pub fail_braille: bool,
}

#[derive(Clone, Debug)]
pub struct FakeVoice { state: Rc<RefCell<FakeVoiceState>> }   // 43-46
impl FakeVoice {
    pub fn new(name: &str, priority: i32, features: VoiceFeatures) -> Self              // 49
    pub fn with_voices(name: &str, priority: i32, features: VoiceFeatures, voices: &[&str]) -> Self  // 61-66
    pub fn state(&self) -> std::cell::Ref<'_, FakeVoiceState>                          // 73
    pub fn state_mut(&self) -> std::cell::RefMut<'_, FakeVoiceState>                   // 79
    pub fn set_runtime_supported(&self, supported: bool)                               // 83
    pub fn set_fail_output(&self, fail: bool)                                          // 87
    pub fn set_fail_braille(&self, fail: bool)                                         // 91
    pub fn spoken(&self) -> Vec<(String, bool)>                                        // 96
    pub fn brailled(&self) -> Vec<String>                                              // 101
    pub fn stop_calls(&self) -> u32                                                    // 105
    pub fn priority(&self) -> i32                                                      // 109
    pub fn boxed(&self) -> Box<dyn VoiceBackend>                                       // 114
}
impl VoiceBackend for FakeVoice                                                        // 123-191
#[derive(Clone, Debug, Default)]
pub struct FakeRegistry { order: Vec<FakeVoice> }                                      // 196-199
impl FakeRegistry {
    pub fn new(voices: Vec<FakeVoice>) -> Self                                         // 204
    pub fn voice(&self, name: &str) -> Option<&FakeVoice>                              // 211
}
impl VoiceRegistry for FakeRegistry                                                    // 221-252
```

Fakes use `prismer::Error::SpeakFailure` (119) and `prismer::Error::BackendNotAvailable` (250), so they keep a hard `prismer` dependency through `SpeechError = prismer::Error`.

### 2h. `app/speech_delivery.rs` public items

```rust
pub const TRANSCRIPT_TARGET: &str = "freight_fate.transcript";   // 27
const HANDED_BACK_KEPT: usize = 512;                             // 37 (private)

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Say { pub interrupt: bool, pub review: bool, pub category: Option<SpeechCategory> }   // 40-45
impl Say { pub const fn new() -> Self; pub const fn queued() -> Self;
           pub const fn interrupt(mut self, interrupt: bool) -> Self;
           pub const fn review(mut self, review: bool) -> Self;
           pub const fn category(mut self, category: SpeechCategory) -> Self; }            // 53-86

#[derive(Default)]
pub struct SayEvent {                                                                    // 89-102
    pub interrupt: bool,
    pub review: bool,
    pub priority: Option<EventPriority>,
    pub key: Option<String>,
    pub force: bool,
    pub category: Option<SpeechCategory>,
    pub receipt: bool,
    pub valid: Option<Valid>,
}
impl SayEvent { pub fn new() -> Self; pub fn queued() -> Self; pub fn interrupt(..); pub fn review(..);
                pub fn priority(..); pub fn key(mut self, key: &str) -> Self; pub fn force(..);
                pub fn category(..); pub fn receipt(mut self) -> Self;
                pub fn valid(mut self, valid: impl Fn() -> bool + 'static) -> Self; }     // 104-158

pub struct Spoken { pub message: SpokenMessage, pub is_pair: bool }                      // 162-167
pub trait IntoSpoken { fn into_spoken(self) -> Spoken; }                                 // 169-171
// impls for &str, String, &String (is_pair=false), SpokenMessage, &SpokenMessage (is_pair=true)  // 173-216
```

`impl GameContext` public methods: `ladder_applies` (231), `say` (266), `say_with` (271), `event_voice_shares_main` (399), `handed_back_count` (452), `reset_ladder_leg_memory` (475), `say_event` (531), `say_event_with` (583), `update_speech_duck` (788), `speech_ducked` (804), `set_earcon_duck_until` (809), `reset_event_condition` (814), `pause_event_speech` (831), `resume_event_speech` (837), `player_asked<R>` (854), `player_asked_begin` (864), `player_asked_end` (870), `speech_requested` (875), `ladder_said` (880), `ladder_last` (885), `event_voice_busy` (894), `event_voice_critical` (899), `event_delivery_status` (903-908), `event_delivery_pending` (910), `stop_event_speech` (914), `stop_speech` (925). Private: `play_ladder_earcon` (249), `render_silenced` (368), `render_spoken` (382), `requeue_cut_event` (415), `deliver_cut_event` (422), `log_mostly_heard_cut` (465), `ladder_repeats` (501), `engage_earcon_duck` (759), `engage_speech_duck` (776).

### 2i. `ff-core/src/speech_pacing.rs` public items

```rust
pub fn monotonic_seconds() -> f64                                                        // 68
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventPriority { Ambient = 0, Route = 1, Critical = 2 }                          // 84-89
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpeechCategory { Safety, Navigation, NavigationAdvisory, Money, Coaching, Confirmation, Status }  // 113-130
impl SpeechCategory { pub const ALL: [SpeechCategory; 7]; pub fn value(self) -> &'static str;
                      pub fn from_value(value: &str) -> Option<SpeechCategory>; }        // 132-163
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Disposition { Full, Terse, FirstOccurrence, Transitions, Earcon, Silent }       // 170-184
impl Disposition { pub const ALL: [Disposition; 6]; pub fn value(self) -> &'static str; }   // 186-207
pub const DRIVING_SPEECH_MODES: [&str; 3] = ["standard", "quiet", "urgent_only"];       // 220
pub type DispositionRow = [(SpeechCategory, Disposition); 7];                            // 223
pub const DRIVING_SPEECH_DISPOSITIONS: [(&str, DispositionRow); 3]                       // 228-265
pub const DEFAULT_DRIVING_SPEECH: &str = "standard";                                     // 267
pub fn disposition_row(mode: &str) -> Option<&'static DispositionRow>                    // 270
pub fn row_disposition(row: &DispositionRow, category: SpeechCategory) -> Option<Disposition>  // 278
pub const LADDER_EARCONS: [(SpeechCategory, &str); 4]                                    // 297-302
pub fn ladder_earcon(category: SpeechCategory) -> Option<&'static str>                   // 305
pub fn disposition_for(mode: &str, category: Option<SpeechCategory>) -> Disposition      // 318
pub type Clock = Box<dyn FnMut() -> f64>;                                                // 330
pub type Valid = Box<dyn Fn() -> bool>;                                                  // 334
pub type Cut = (String, EventPriority);                                                  // 337
pub use receipts::DeliveryStatus;  // enum DeliveryStatus { Pending, Completed, Interrupted }  (receipts.rs:7-12)
pub struct EventSpeechPacer { /* private */ }                                            // 369-414
impl EventSpeechPacer {
    pub const STALE_WAIT_S / BASE_UTTERANCE_S / CHARS_PER_S / REPEAT_WINDOW_S / RECENT_LIMIT /
              RECENT_MEMORY_S / RESCUE_ONCE_WINDOW_S / MOSTLY_HEARD_FRACTION           // 424-463 (values in section 4)
    pub fn wait_budget_s(priority: EventPriority) -> f64                                 // 469
    pub fn new() -> Self                                                                 // 478
    pub fn with_clock(clock: Clock) -> Self                                              // 482
    pub fn set_clock(&mut self, clock: Clock)                                            // 532
    pub fn is_repeat(&mut self, text: &str, key: Option<&str>, force: bool, window: Option<f64>) -> bool   // 555-561
    pub fn note_spoken(&mut self, text: &str, key: Option<&str>)                         // 581
    pub fn forget_condition(&mut self, key: &str)                                        // 597
    pub fn is_silenced_repeat(&mut self, text: &str, key: Option<&str>, window: Option<f64>) -> bool  // 619-624
    pub fn note_silenced(&mut self, text: &str, key: Option<&str>)                       // 648
    pub fn track(&mut self, text: &str, priority: EventPriority, category: Option<SpeechCategory>, valid: Option<Valid>)  // 683-689
    pub fn take_flush_cut(&mut self) -> Option<Cut>                                      // 709
    pub fn take_mostly_heard(&mut self) -> Option<String>                                // 720
    pub fn note_interrupt(&mut self, text: &str, priority: EventPriority, category: Option<SpeechCategory>, valid: Option<Valid>) -> Option<Cut>  // 820-826
    pub fn note_queued(&mut self, text: &str, priority: EventPriority, category: Option<SpeechCategory>, valid: Option<Valid>)  // 842-848
    pub fn track_delivery(&mut self, key: &str, text: &str)                              // 855
    pub fn complete_delivery(&mut self, key: &str, text: &str)                           // 860
    pub fn delivery_status(&mut self, key: &str) -> Option<DeliveryStatus>               // 866
    pub fn delivery_pending(&mut self) -> bool                                           // 872
    pub fn note_ahead(&mut self, text: &str)                                             // 881
    pub fn resume_delivery(&mut self, text: &str)                                        // 886
    pub fn note_channel_purged(&mut self) -> Option<Cut>                                 // 902
    pub fn busy(&mut self) -> bool                                                       // 917
    pub fn speaking_critical(&mut self) -> bool                                          // 922
    pub fn would_start_stale(&mut self, _text: &str, priority: EventPriority) -> bool     // 938
    pub fn should_flush(&mut self, text: &str, priority: EventPriority, valid: Option<Valid>) -> bool  // 958-963
    pub fn reset(&mut self)                                                              // 1070
    pub fn pause(&mut self)                                                              // 1084
    pub fn resume(&mut self)                                                             // 1092
}
```

### 2j. `ff-core/src/speech_text.rs` generic part

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SpokenMessage { pub normal: String, pub terse: Option<String> }   // 79-83
impl SpokenMessage {
    pub fn new(normal: impl Into<String>) -> Self                            // 87
    pub fn with_terse(normal: impl Into<String>, terse: impl Into<String>) -> Self   // 95
    pub fn pair(normal: impl Into<String>, terse: Option<String>) -> Self    // 103
    pub fn render(&self, terse: bool) -> &str                                // 110
    pub fn plus(&self, suffix: &str) -> SpokenMessage                        // 124
}
impl fmt::Display / From<&str> / From<String> / PartialEq<str> / PartialEq<&str> / AsRef<str> for SpokenMessage  // 137-171
pub fn terse_silent(normal: impl Into<String>) -> SpokenMessage              // 175
```

---

## 3. Dependency map

Legend: **(a)** inside the speech stack; **(b)** generic helper that must come along; **(c)** FF/trucking coupling to cut; **ext** external crate; **std** standard library.

### `freight-fate/src/speech.rs`
| Import (verbatim) | Class | Notes |
|---|---|---|
| `use ff_core::settings::Settings;` (63) | **(c)** | Only for `apply_speech_settings` (284). Reads `sapi_events`, `event_backend`, `speech_voice`, `speech_rate`, `speech_pitch`, `speech_volume`, `braille_only`; **writes** `settings.event_backend` (295). Replace with a hearken-owned settings struct. |
| `pub use ff_core::speech_pacing::{EventPriority, EventSpeechPacer};` (74) | **(b)**/(c) | Re-export only; nothing in `speech/` uses the pacer. Pacer is 1101+92 lines, std-only; see section 4 for the driving-specific parts. Drop the re-export from the M1 crate unless the pacer moves too. |
| `pub use backend::{...}`, `pub use capture::{...}`, `pub use live::{...}`, `pub use threaded::ThreadedSpeech;` (76-85) | (a) | |
| `log::info!` (290) | ext `log` | |

### `freight-fate/src/speech/backend.rs`
| Import | Class | Notes |
|---|---|---|
| `use std::cell::{Cell, RefCell};` (10) | std | |
| `use std::collections::HashMap;` (11) | std | |
| `use std::fmt;` (12) | std | |
| `use std::rc::Rc;` (13) | std | |
| `#[cfg(test)] #[path = "backend/recovery_tests.rs"] mod recovery_tests;` (15-17) | (a) | |
| `prismer::{Error, Features, BackendId, Backend, Prism}` (paths used inline: 23, 93, 178-179, 236-253, 285, 297-309, 334, 395) | ext `prismer` | `use prismer::Features as F;` at 93 is the only `use`. |
| `use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};` (418) | ext `windows-sys` (cfg windows) | Features `Win32_Foundation`, `Win32_System_Diagnostics_ToolHelp`. |
| `use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};` (419-422) | ext `windows-sys` (cfg windows) | |
| `super::UIA_LAST_RESORT_PRIORITY` (525) | (a) | |
| `log::warn!` / `log::info!` (496-508, 566, 570) | ext `log` | |

No FF coupling.

### `freight-fate/src/speech/live.rs`
| Import | Class | Notes |
|---|---|---|
| `use std::env;` (5) | std | |
| `use super::backend::{narrator_running, pick_backend_gated, pick_event_backend, preserve_backend_default_pitch, usable, PrismRegistry, VoiceBackend, VoiceFeatures, VoiceRegistry};` (7-10) | (a) | |
| `use super::{PreviewFeature, SpeechSink, EVENT_BACKEND, REFRESH_INTERVAL_S};` (11) | (a) | |
| `super::SPEECH_BACKEND_ENV` (86), `super::NO_SPEECH_ENV` (89) | (a) but **(c)** in value | Env names are `FREIGHT_FATE_*`. |
| literal `"FREIGHT_FATE_STREAM_TRANSCRIPT"` (17) | **(c)** | Not a const; hard-coded string. |
| literal `"Speech disabled via FREIGHT_FATE_NO_SPEECH"` (90) | **(c)** | Log text. |
| `prismer::Error` (262, 432) | ext | |
| `log::*` (90, 107, 145-153, 253, 272, 290, 308, 568, 587, 630) | ext `log` | |

### `freight-fate/src/speech/threaded.rs`
| Import | Class |
|---|---|
| `use std::sync::atomic::{AtomicBool, Ordering};` (57) | std |
| `use std::sync::mpsc::{self, RecvTimeoutError, TrySendError};` (58) | std |
| `use std::sync::{Arc, Mutex};` (59) | std |
| `use std::time::{Duration, Instant};` (60) | std |
| `use super::SpeechSink;` (62) | (a) |
| `super::live::Speech::new_after_wedge()` / `super::live::Speech::new()` (340, 342) | (a) |
| `#[cfg(test)] #[path = "threaded/recovery_tests.rs"] mod recovery_tests;` (863-865), `#[cfg(test)] #[path = "threaded/tests.rs"] mod tests;` (867-869) | (a) |
| `log::error!` / `log::warn!` (586, 655, 715, 725) | ext `log` |

No FF coupling (comments mention testers and "I-77").

### `freight-fate/src/speech/capture.rs`
| Import | Class | Notes |
|---|---|---|
| `use ff_core::speech_text::SpokenMessage;` (10) | **(b)** | Used only by `say_message` (213) and `say_event_message` (221). `SpokenMessage` + `terse_silent` = `speech_text.rs:68-177` (~110 lines incl. docs), std-only apart from `use std::fmt;`. The rest of `speech_text.rs` (and its `use crate::sim::trip_models::OpenSide;` at 32) is trucking and stays in ff-core. |
| `use super::SpeechSink;` (12) | (a) | |
| `super::EVENT_BACKEND` (146), `super::PreviewFeature::for_setting` (457) | (a) | |

### `freight-fate/src/speech/fakes.rs`
| Import | Class |
|---|---|
| `use std::cell::RefCell;` (9) | std |
| `use std::rc::Rc;` (10) | std |
| `use super::backend::{BackendId, VoiceBackend, VoiceFeatures, VoiceRegistry};` (12) | (a) |
| `prismer::Error` (118-120, 132-190, 247-251) | ext |

### `freight-fate/src/speech/backend/recovery_tests.rs`
`use super::*;` (1) -- (a).

### `freight-fate/src/speech/threaded/recovery_tests.rs`
`use super::*;` (1), `use std::sync::atomic::{AtomicBool, Ordering};` (2), `use std::sync::Condvar;` (3) -- (a)/std.

### `freight-fate/src/speech/threaded/tests.rs`
`use super::*;` (1), `use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};` (2) -- (a)/std. Defines its own `StubSink` (5-121); no fakes/capture.

### `freight-fate/src/app/speech_delivery.rs`
| Import (verbatim) | Class | What / how used |
|---|---|---|
| `use ff_core::message_log::MessageCategory;` (13) | **(c)** | Review log: `self.message_log.add(&text, MessageCategory::General)` (362), `MessageCategory::Event` (679, 734). |
| `use ff_core::sound_catalog::entry_by_name;` (14) | **(c)** | `play_ladder_earcon` (256): catalog name -> `entry.plays.first()` -> `self.audio.play_with(cue.key, cue.volume, cue.pan)` (262). |
| `use ff_core::speech_pacing::{ladder_earcon, monotonic_seconds, Cut, Disposition, EventPriority, SpeechCategory, Valid};` (15-17) | (b) pacer types; **(c)** `ladder_earcon`, `Disposition`, `SpeechCategory` | Rung/ladder is FF verbosity policy. |
| `use ff_core::speech_text::SpokenMessage;` (18) | (b) | `Spoken`/`IntoSpoken`. |
| `use crate::audio::{EARCON_DUCK_S, SPEECH_DUCK_LEVEL};` (20) | **(c)** | Duck constants live in FF `audio.rs` (221, 227). |
| `use super::GameContext;` (22) | **(c)** | The whole file is `impl GameContext`. |
| `ff_core::speech_pacing::DeliveryStatus` (906) | (b) | |
| `log::info!(target: TRANSCRIPT_TARGET, ...)` via `transcript!` (29-33) | ext `log` | Target string is FF-named. |

`GameContext` coupling inside `speech_delivery.rs` (all **(c)**):
- `self.settings.speaks(category)` (278, 596), `.speech_disposition(category)` (308, 507, 623), `.renders_terse()` (370, 384), `.driving_speech` (317, 637, 649), `.sapi_events` (400, 443, 667, 686), `.duck_audio_for_speech` (760, 777).
- `self.profile.as_ref().is_none_or(|profile| profile.tutorial_done)` (232-234) -- FF `Profile`.
- `self.top_paces_main_speech()` (329) -- reads FF state stack (`context.rs:459-463`, from each state's `paces_main_speech()`).
- `self.speech_requested` (278, 329, 865-876).
- `self.event_pacer` (throughout), `self.message_log` (362, 679, 734), `self.audio` (262, 765, 781, 800), `self.speech` (356, 403, 444, 446, 710, 716, 721, 833, 839, 916, 927-928).
- `self.handed_back` (433-436, 453), `self.ladder_said` (476, 510-513, 881), `self.ladder_last` (520-523, 821, 886), `self.speech_ducked` (763, 780, 789, 798, 805), `self.earcon_duck_until` (764, 792, 799, 810).

### `ff-core/src/speech_pacing.rs`
`use std::collections::HashMap;` (58), `use std::sync::OnceLock;` (59), `use std::time::Instant;` (60), `mod receipts; use receipts::DeliveryReceipts; pub use receipts::DeliveryStatus;` (62-64). std-only.
### `ff-core/src/speech_pacing/receipts.rs`
`use std::collections::HashMap;` (3). std-only.

### Crate-level dependencies the new crate needs
From `freight-fate/Cargo.toml` (40-75) and workspace `Cargo.toml`:
- `prismer = { version = "0.1.3", features = ["static"] }` (freight-fate/Cargo.toml:47). prismer 0.1.3 has `links = "prismer"`; its `build.rs` re-publishes prism-sys's `DEP_PRISM_DELAY_LOAD_DLLS` as `cargo:delay_load_dlls=...`. prism-sys 0.1.3 builds Prism from source with the `cmake` crate (build-dependency).
- `log = "0.4"` (workspace).
- `[target.'cfg(windows)'.dependencies] windows-sys = "0.59"` with at least `Win32_Foundation`, `Win32_System_Diagnostics_ToolHelp` (workspace line 56 lists more).
- Workspace `edition = "2021"`, `rust-version = "1.95"`. Code uses `Option::is_none_or` (threaded.rs:162-163, speech_delivery.rs:233, speech_pacing.rs:527, 808).

---

## 4. `speech_delivery.rs`: ladder, pacer, duck

### 4a. Two "ladders"

**Event priority** (`ff-core/src/speech_pacing.rs:84-89`) -- how long a line waits:

```rust
pub enum EventPriority {
    Ambient = 0,
    Route = 1,
    Critical = 2,
}
```

Default when `SayEvent.priority` is `None` (`speech_delivery.rs:660-664`):
```rust
let priority = priority.unwrap_or(if interrupt {
    EventPriority::Critical
} else {
    EventPriority::Ambient
});
```

**Driving-speech rung** (verbosity; `speech_pacing.rs:220-265`), chosen by `Settings.driving_speech`:

| Category \ rung | `standard` | `quiet` | `urgent_only` |
|---|---|---|---|
| Safety | Full | Terse | Terse |
| Money | Full | Terse | Silent |
| Navigation | Full | Terse | Terse |
| NavigationAdvisory | Full | Terse | Earcon |
| Coaching | FirstOccurrence | Earcon | Silent |
| Confirmation | Full | Terse | Earcon |
| Status | Transitions | Terse | Silent |

`DEFAULT_DRIVING_SPEECH = "standard"`. `disposition_for(mode, None)` returns the row's Safety disposition (an unclassified line always speaks). `Settings::renders_terse()` is `driving_speech == "quiet" || driving_speech == "urgent_only"` (settings.rs:798-800). `LADDER_EARCONS` (297-302): NavigationAdvisory -> `"Road ahead note"`, Coaching -> `"Coaching note"`, Confirmation -> `"Confirmation note"`, Status -> `"Status note"` (FF sound-catalog names).

The rung only applies when `ladder_applies()` (tutorial done or no profile, 231-235) and, for `say_with`, when `!self.speech_requested`; for `say_event_with`, when `!force`.

### 4b. The pacer (`EventSpeechPacer`) constants, verbatim (`speech_pacing.rs:423-475`)

```rust
    /// a queued line may start at most this far in the past
    pub const STALE_WAIT_S: f64 = 3.0;
    /// per-utterance pause before the voice gets going
    pub const BASE_UTTERANCE_S: f64 = 0.4;
    /// the default Windows voice at its default rate
    pub const CHARS_PER_S: f64 = 13.0;
    pub const REPEAT_WINDOW_S: f64 = 2.5;
    pub const RECENT_LIMIT: usize = 256;
    pub const RECENT_MEMORY_S: f64 = 300.0;
    pub const RESCUE_ONCE_WINDOW_S: f64 = 30.0;
    pub const MOSTLY_HEARD_FRACTION: f64 = 0.5;

    pub fn wait_budget_s(priority: EventPriority) -> f64 {
        match priority {
            EventPriority::Ambient => Self::STALE_WAIT_S,
            EventPriority::Route => 0.8,
            EventPriority::Critical => Self::STALE_WAIT_S,
        }
    }
```
Plus `receipts.rs:5`: `const RECEIPT_LIMIT: usize = 32;`, and `HANDED_BACK_KEPT: usize = 512` (speech_delivery.rs:37).

Duration model (540-542): `BASE_UTTERANCE_S + text.chars().count() as f64 / CHARS_PER_S`.

Driving-specific vs generic:
- Generic mechanism: projection (`clear_at`, `extend_projection`, `restart_projection`), repeat window, condition keys, silenced namespace, receipts, protected-line rescue, mostly-heard drop, pause/resume purge, `busy()`.
- Tuned to a Windows default voice (generic, but a knob): `CHARS_PER_S = 13.0`, `BASE_UTTERANCE_S = 0.4`.
- Driving-flavoured: the name `Route` and its `0.8` s budget ("a planned stop that arrives after its exit"); `SpeechCategory` variants (`Navigation`, `NavigationAdvisory`, `Money`, ...); the rung table, `DRIVING_SPEECH_*`, `LADDER_EARCONS`, `disposition_for`; `track()` special-cases `SpeechCategory::Confirmation` (690-693) so confirmations never take the rescue slot; the whole rationale text (weigh stations, exits, troopers).
- `RESCUE_ONCE_WINDOW_S = 30.0` comment cites "the trooper escalation" but the value is generic.

### 4c. How `say_event` reaches the sink

Call chain: state code -> `ctx.say_event(text)` (531-533) -> `say_event_with(text, SayEvent::new())` (583-736):

1. `text.into_spoken()` (584).
2. **Rung gate** (596-641): `if !self.settings.speaks(category) && !force && self.ladder_applies()` -> render silenced text; if `event_pacer.is_silenced_repeat(&text, key, None)` return; if disposition is `Earcon` -> `play_ladder_earcon(category)` + `engage_earcon_duck()`; `event_pacer.note_silenced`; optional `complete_delivery`; `transcript!("[ladder] {} silenced: {}", ...)`; return. Nothing reaches the sink.
3. `render_spoken` (642) -- `None` (terse drops pair) returns.
4. **Ladder repeat** (645-653): `ladder_repeats` (FirstOccurrence / Transitions memory) -> `transcript!("[ladder] {} already said: {}")`, return.
5. **Pacer repeat** (654-659): `event_pacer.is_repeat(&text, key, force, None)` -> return silently.
6. Priority default (660-664).
7. **Stale ambient drop** (665-682): `!interrupt && priority == Ambient && settings.sapi_events && event_pacer.would_start_stale(..)` -> `transcript!("[pacer] stale ambient dropped: {}")`, review-log it, return.
8. `transcript!("[event] {}", text)` (683); `event_pacer.note_spoken(&text, key)` (684).
9. **Delivery** (686-722):
   - `settings.sapi_events == true`: interrupt -> `cut = event_pacer.note_interrupt(..)`; queued -> `event_pacer.should_flush(..)`; if flushing, `transcript!("[pacer] stale event backlog flushed")`, `cut = take_flush_cut()`, `interrupt = true`, and a flushed cut is delivered FIRST via `deliver_cut_event(first, true)` then `interrupt = false`. Then **`self.speech.say_event(&text, interrupt)`** (710).
   - `sapi_events == false`: interrupt -> `note_interrupt` + **`self.speech.stop_main()`** (716); queued -> `note_queued`. Then **`self.speech.say(&text, false)`** (721).
10. `receipt` -> `event_pacer.track_delivery` (723-727); `engage_speech_duck()` (728); `log_mostly_heard_cut()` (729); `requeue_cut_event(cut)` (730-732) -> `deliver_cut_event(cut, false)` which logs `transcript!("[pacer] cut line requeued: {}")` and calls `self.speech.say_event(&text, interrupt)` or `self.speech.say(&text, false)` (443-447); review log `MessageCategory::Event` (733-735).

Then `ThreadedSpeech::say_event` (683-688) -> `send_lossy(Command::SayEvent{..})` -> worker `coalesce` -> `inner.say_event` -> `live::Speech::say_event` (508-540): braille-only routes to `say`; no event voice -> (interrupt ? `stop_main`) + `say(text, false)`; else `speak_with_backend(event_backend, text, interrupt)`, falling back to main on failure.

`say_with` (271-364) is the main-channel twin: rung gate (278-321, exempt when `speech_requested`), `render_spoken`, `transcript!("{}", text)` (328), at-the-wheel queueing `if interrupt && !self.speech_requested && self.top_paces_main_speech() { interrupt = false; }` (329-346), shared-voice rescue `cut = self.event_pacer.note_channel_purged()` when `interrupt && self.event_voice_shares_main()` (348-355), **`self.speech.say(&text, interrupt)`** (356), `log_mostly_heard_cut`, requeue, review log `MessageCategory::General`.

`event_voice_shares_main()` (399-404): `!settings.sapi_events || !self.speech.has_separate_event_voice()`.

### 4d. The duck

Talks to audio through FF's `trait Audio` (`freight-fate/src/audio.rs:520`):
```rust
    fn play_with(&mut self, key: &str, volume: f64, pan: f64);   // audio.rs:547
    fn set_speech_duck(&mut self, duck: f64);                    // audio.rs:566
```
Constants (`audio.rs:221`, `227`):
```rust
pub const SPEECH_DUCK_LEVEL: f64 = 0.5;
pub const EARCON_DUCK_S: f64 = 0.25;
```
- `engage_speech_duck` (776-782): if `settings.duck_audio_for_speech`, set `speech_ducked = true`, `audio.set_speech_duck(SPEECH_DUCK_LEVEL)`. Called after every spoken event line (728).
- `engage_earcon_duck` (759-766): same plus `earcon_duck_until = monotonic_seconds() + EARCON_DUCK_S`.
- `update_speech_duck` (788-801), called per frame from `app.rs:835`: returns while `earcon_duck_until` is in the future or `event_pacer.busy()`; else `speech_ducked = false`, `earcon_duck_until = 0.0`, `audio.set_speech_duck(1.0)`. Restoration keys off the pacer's projection, not the voice.

Note: main-channel `say_with` never ducks; only event lines and earcons do.

### 4e. `GameContext` fields read/written by `speech_delivery.rs`

Declared in `app/context.rs:130-222`:
- `pub speech: Box<dyn SpeechSink>` (131)
- `pub audio: Box<dyn Audio>` (132)
- `pub settings: Settings` (134) -- fields `driving_speech`, `sapi_events`, `duck_audio_for_speech`; methods `speaks`, `speech_disposition`, `renders_terse`
- `pub profile: Option<Profile>` (140) -- `tutorial_done`
- `pub message_log: MessageLog` (144)
- `pub event_pacer: EventSpeechPacer` (165)
- `pub(crate) speech_ducked: bool` (175)
- `pub(crate) earcon_duck_until: f64` (178)
- `pub(crate) speech_requested: bool` (182)
- `pub(crate) ladder_said: HashSet<(String, String)>` (188)
- `pub(crate) ladder_last: HashMap<String, String>` (189)
- `pub(crate) handed_back: VecDeque<String>` (195)
- private `stack: Vec<StackEntry>` via `top_paces_main_speech()` (459-463)

Other pacer-facing API used by FF states: `pause_event_speech`, `resume_event_speech`, `stop_event_speech`, `stop_speech`, `reset_event_condition`, `reset_ladder_leg_memory`, `event_voice_busy`, `event_voice_critical`, `event_delivery_status`, `event_delivery_pending`, `player_asked*`.

---

## 5. Settings that affect speech

Struct: `ff_core::settings::Settings`, generated by `macro_rules! settings_fields` (`ff-core/src/settings.rs:265`, struct at 269). Speech fields, verbatim macro rows:

```rust
    duck_audio_for_speech: bool = false => bool_strict,          // 532
    driving_speech: String = DEFAULT_DRIVING_SPEECH => str_checked,   // 537
    sapi_events: bool = true => bool_truthy,                     // 564  "driving events on a separate voice"
    event_backend: String = "SAPI" => str_checked,               // 566  "which voice that is (e.g. SAPI/OneCore)"
    braille_only: bool = false => bool_strict,                   // 574
    speech_rate: f64 = 0.5 => level,                             // 576  "voice speed, 0..1 (backend default ~0.5)"
    speech_pitch: f64 = 0.5 => level,                            // 578
    speech_volume: f64 = 1.0 => level,                           // 580
    speech_voice: String = "" => str_plain,                      // 582  "installed voice name; "" = backend default"
```
Related but not speech-engine: `announce_menu_position` (559), chatter switches, `place_callouts`. Settings-version history: `settings.rs:67` "3: speech_verbosity (0 terse / 1 normal) became the driving_speech ladder." -- there is no `speech_verbosity` field any more; verbosity == `driving_speech`.

Methods (`settings.rs:781-800`):
```rust
    pub fn speech_disposition(&self, category: Option<SpeechCategory>) -> Disposition {
        disposition_for(&self.driving_speech, category)
    }
    pub fn speaks(&self, category: Option<SpeechCategory>) -> bool {
        !matches!(
            self.speech_disposition(category),
            Disposition::Earcon | Disposition::Silent
        )
    }
    pub fn renders_terse(&self) -> bool {
        self.driving_speech == "quiet" || self.driving_speech == "urgent_only"
    }
```

Where each is read:
- **Voice, rate, pitch, volume, braille-only, event voice**: only in `apply_speech_settings` (`speech.rs:284-306`), invoked by `GameContext::apply_speech` (`app/context.rs:807-809`) at startup (`app.rs:509`) and after every settings change (`states/main_menu/settings_actions.rs:557, 564, 581, 598`). The sink remembers them (`live::Speech.config`, `ThreadedSpeech.replay`) and re-applies after a backend swap / worker respawn.
- **Screen reader vs SAPI**: the MAIN voice is never a setting. `pick_backend_gated` (`backend.rs:485-543`) picks the highest-priority backend that is usable now (Prism static priority; screen readers outrank SAPI/OneCore; UIA only if Narrator runs, then last). Only override: env `FREIGHT_FATE_SPEECH_BACKEND`. The EVENT voice is the setting: `sapi_events` (separate voice on/off) + `event_backend` (name). `EVENT_BACKEND = "SAPI"` is the startup default inside `Speech::with_registry` (151). The settings menu builds its choice list from `sink.event_backend_options()` (`settings_actions.rs:541`) and writes `sapi_events`/`event_backend` (549-554).
- **Verbosity** (`driving_speech`): `speech_delivery.rs` only (sections 4a, 4c). Not seen by any sink.
- **`sapi_events`** is also read directly in `speech_delivery.rs` (400, 443, 667, 686) and by states to decide whether the event voice's rate is the relevant one (`states/driving_events/update_exit.rs:610`, `driving_facility_gate.rs:128`, `driving_turns.rs:258`, `driving_updates/enforcement.rs:278`: `if ctx.settings.sapi_events && ctx.speech.event_supports_rate()`).
- **`duck_audio_for_speech`**: `speech_delivery.rs:760, 777`.
- Previews: `states/main_menu/settings.rs:413` `ctx.speech.say_adjustment_preview(setting, &text, true)` with setting keys matching `PREVIEW_FEATURES`.

---

## 6. Threading model (`speech/threaded.rs`)

Constants, verbatim (64-78):
```rust
/// How long a silent worker is allowed before the watchdog calls it wedged.
const WEDGE_AFTER_S: f64 = 8.0;
/// How long a wedged worker is given to come back before it is abandoned
/// and replaced. Longer than a screen reader's own freeze recovery (NVDA's
/// watchdog gives its core about ten seconds), so a stall that will clear
/// on its own is not answered with a second voice.
const RESPAWN_AFTER_S: f64 = 20.0;
/// Replacement workers per session, at most.
const MAX_RESPAWNS: u32 = 3;
/// Command queue depth; beyond it, new say lines are dropped, not queued.
const QUEUE_DEPTH: usize = 256;
/// How often the worker asks Prism to re-check the live speech backends.
const HEALTH_POLL: Duration = Duration::from_secs(3);
/// Bounded wait for the two calls that need an answer.
const REPLY_WAIT: Duration = Duration::from_secs(2);
```

Worker thread and channel (383-399):
```rust
    fn start_worker(factory: &Arc<SinkFactory>, after_wedge: bool) -> Worker {
        let (commands, rx) = mpsc::sync_channel::<Command>(QUEUE_DEPTH);
        ...
        std::thread::Builder::new()
            .name("speech".to_string())
            .spawn(move || {
                let mut inner = factory(after_wedge);
```
- Command channel: **bounded** `mpsc::sync_channel`, capacity **256**. All sends are `try_send` (never block).
- Reply channels: unbounded `mpsc::channel()` per request (`bounded_reply` 668-672, `refresh` 831, `shutdown` 849).
- Shared state: `Arc<Mutex<Snapshot>>` (query answers), `Arc<Mutex<Instant>>` heartbeat, `Arc<AtomicBool>` `shutting_down`, `abandoned`, `ready`.
- The sink is built ON the worker by the factory (so `!Send` `live::Speech` never crosses threads). The thread is detached (no `JoinHandle` stored).

Main loop (428-565), key excerpts:
```rust
                    let until_health_poll = HEALTH_POLL.saturating_sub(last_health_poll.elapsed());
                    let first = match rx.recv_timeout(until_health_poll) {
                        Ok(command) => command,
                        Err(RecvTimeoutError::Timeout) => {
                            let elapsed = last_health_poll.elapsed();
                            beat();
                            call!(inner.poll(elapsed.as_secs_f64()));
                            call!(publish_after_probe(
                                &worker_snapshot,
                                inner.as_ref(),
                                &worker_abandoned
                            ));
                            last_health_poll = Instant::now();
                            continue;
                        }
                        Err(RecvTimeoutError::Disconnected) => {
                            call!(inner.shutdown());
                            return;
                        }
                    };
                    let mut batch = vec![first];
                    while let Ok(command) = rx.try_recv() {
                        batch.push(command);
                    }
                    coalesce(&mut batch);
```
Says are skipped while `shutting_down` is set (`let draining = worker_shutting_down.load(Ordering::Acquire);` 476, `if !draining` 481/491). After each say, `publish_status`; after configure/select/refresh, full `publish`; after health probe, `publish_after_probe` (enumerates options/voices only if a voice name changed -- works around the OneCore leak).

Coalescing (139-169): an interrupting `Say` or `StopMain` drops earlier queued `Say`s; an interrupting `SayEvent` or `StopEvent` drops earlier `SayEvent`s; `Stop` drops both. Only says are ever dropped.

Abandon guard (403-418): `active!()` checks `worker_abandoned` before and after every native call; if set, `std::mem::forget(inner); return;`.

Lossy send (650-666):
```rust
    fn send_lossy(&mut self, command: Command) {
        if let Err(TrySendError::Full(command)) = self.commands.try_send(command) {
            if matches!(command, Command::Say { .. } | Command::SayEvent { .. }) {
                self.dropped_lines += 1;
                if self.dropped_lines.is_power_of_two() {
                    log::warn!(
                        "speech queue full: {} line(s) dropped while the backend is stalled",
                        self.dropped_lines
                    );
                }
            }
        }
    }
```

Bounded answers:
- `say_adjustment_preview` -> `bounded_reply` -> `reply_rx.recv_timeout(REPLY_WAIT).unwrap_or(false)` (**2 s**).
- `refresh` -> `reply_rx.recv_timeout(Duration::from_secs(4)).unwrap_or(false)` (**4 s**, 838-840).
- All other queries answer from `Snapshot` without touching the worker.

Watchdog (`poll`, 706-732): heartbeat stale > `wedge_after_s` (8 s) -> log once `"speech backend stopped responding {stale:.0}s ago (a wedged screen reader or SAPI call); the game continues without speech until it returns"`; recovered only when `ready && snapshot.available` -> `"speech backend recovered"`; stale > `respawn_after_s` (20 s) and `respawns < max_respawns` (3) -> `respawn`. `available()` (738-749) = `ready && snapshot.available && stale <= wedge_after_s`.

Respawn (583-625): sets old `shutting_down` and `abandoned`, starts a new worker with `after_wedge = true` (factory -> `Speech::new_after_wedge()` -> `PrismRegistry::new_fresh()`), replays `SelectEventBackend`, `Configure`, `SetBrailleOnly`, then `ReplayComplete` (which sets `ready`).

Shutdown, verbatim (843-860):
```rust
    fn shutdown(&mut self) {
        // The flag first, then the command: the worker skips every say
        // still queued ahead of it, so the wait below covers one in-flight
        // utterance at most, not the whole backlog.
        self.shutting_down
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let (done_tx, done_rx) = mpsc::channel();
        if self
            .commands
            .try_send(Command::Shutdown { done: done_tx })
            .is_ok()
        {
            // Give the backend a bounded chance to release cleanly; a
            // wedged one is abandoned, which is exactly what quitting a
            // frozen game by hand used to do -- minus freezing the game.
            let _ = done_rx.recv_timeout(Duration::from_secs(3));
        }
    }
```
Shutdown timeout: **3 s**. There is no `Drop` impl: dropping a `ThreadedSpeech` without `shutdown()` disconnects the channel, and the worker calls `inner.shutdown()` on its next `recv_timeout` (454-457), unjoined. FF calls `shutdown()` explicitly at `app.rs:1000`.

`live::Speech` is `!Send`/`!Sync` by construction (`Rc` in `PrismRegistry`/`PrismVoice`); module docs `speech.rs:33-50` and `threaded.rs:15-21` state the invariant.

---

## 7. Capture sink, fakes, and tests

API: see 2f (capture) and 2g (fakes). FF's test app shares a capture with the boxed sink through an adapter, `freight-fate/src/app/testing.rs:357`:
```rust
pub struct SharedCapture(pub Rc<RefCell<CaptureSpeech>>);
impl SpeechSink for SharedCapture { /* forwards every method to self.0.borrow_mut() */ }
```
with `TestApp::speech()` / `speech_mut()` / `clear_speech()` / `main_calls()` / `event_calls()` / `main_lines()` (testing.rs 287-312).

### Representative test 1 -- capture (`freight-fate/tests/it/speech.rs:677-710`)
```rust
#[test]
fn capture_records_both_channels_in_order_with_interrupt() {
    let mut capture = CaptureSpeech::new();
    capture.say("Main menu.", true);
    capture.say_event("Brake now.", true);
    capture.say("Settings.", false);
    assert_eq!(
        capture.lines(),
        vec!["Main menu.", "Brake now.", "Settings."]
    );
    assert_eq!(capture.main_lines(), vec!["Main menu.", "Settings."]);
    assert_eq!(capture.event_lines(), vec!["Brake now."]);
    assert_eq!(
        capture.calls(SpeechChannel::Main),
        spoken(&[("Main menu.", true), ("Settings.", false)])
    );
    assert_eq!(
        capture.tagged(),
        vec![
            ("main", "Main menu.".to_string()),
            ("event", "Brake now.".to_string()),
            ("main", "Settings.".to_string())
        ]
    );
    let sequences: Vec<u64> = capture.entries().iter().map(|e| e.sequence).collect();
    assert_eq!(sequences, vec![0, 1, 2]);
    assert_eq!(
        capture.transcript(),
        "Main menu.\n[event] Brake now.\nSettings."
    );
    capture.clear();
    assert!(capture.lines().is_empty());
    assert!(capture.transcript().is_empty());
}
```
(`spoken` helper at speech.rs:34-39.)

### Representative test 2 -- fakes + live `Speech` (`freight-fate/tests/it/speech.rs:471-497`)
```rust
fn live_registry(nvda_running: bool) -> (Speech, FakeVoice, FakeVoice) {
    let nvda = FakeVoice::new(
        "NVDA",
        103,
        VoiceFeatures {
            is_supported_at_runtime: nvda_running,
            supports_stop: true,
            ..SPEAKING
        },
    );
    let sapi = FakeVoice::with_voices("SAPI", 97, ADJUSTABLE, &["David", "Zira"]);
    let ctx = FakeRegistry::new(vec![nvda.clone(), sapi.clone()]);
    let speech = Speech::from_parts(Some(Box::new(ctx)), None, None);
    (speech, nvda, sapi)
}

#[test]
fn test_say_failure_switches_to_a_live_voice_and_retries() {
    // NVDA quits mid-game: the utterance that fails must come out of the
    // fallback voice instead of muting the game forever.
    let (mut s, nvda, sapi) = live_registry(false);
    s.set_main_backend(Some(nvda.boxed()));
    nvda.set_fail_output(true);
    s.say("Turn left ahead.", true);
    assert_eq!(s.backend_name(), "SAPI");
    assert_eq!(sapi.spoken(), spoken(&[("Turn left ahead.", true)]));
}
```

### Representative test 3 -- delivery layer through the capture (`freight-fate/tests/it/app_event_speech_pacer.rs:30-34, 49-80`)
```rust
fn sapi_app() -> TestApp {
    let mut app = TestApp::new();
    app.ctx.settings.sapi_events = true;
    app
}

const APPROACH: [&str; 4] = [
    "Slow down for the dock, twenty five miles per hour through the yard.",
    "Passing the fuel island, dock doors ahead on the left.",
    "At the dock. Line up square and ease it back.",
    "Delivering. The forklift crew is unloading the trailer.",
];

#[test]
fn test_say_event_queues_a_burst_of_route_lines_whole_end_to_end() {
    // (comment elided)
    let mut app = sapi_app();
    for line in APPROACH {
        app.ctx
            .say_event_with(line, SayEvent::queued().priority(EventPriority::Route));
    }
    assert_eq!(
        app.event_calls(),
        APPROACH
            .iter()
            .map(|line| (line.to_string(), false))
            .collect::<Vec<_>>()
    );
    assert_eq!(app.ctx.handed_back_count(""), 0);
    app.shutdown();
}
```

(For the worker, the pattern is a hand-rolled `StubSink` + `ThreadedSpeech::spawn_with`: `speech/threaded/tests.rs:5-193`, e.g. `queued_says_are_purged_by_a_later_interrupt_before_they_speak` at 322-358.)

### Tests exercising the speech stack

`freight-fate/src` (unit, cfg(test)):
- `speech/backend/recovery_tests.rs` (4 tests)
- `speech/threaded/recovery_tests.rs` (2 tests)
- `speech/threaded/tests.rs` (10 tests)
- `ff-core/src/speech_pacing/tests.rs`, `ff-core/src/speech_pacing/tests/settings.rs` (pure pacer / rung)

`freight-fate/tests/it/` that import `freight_fate::speech::` directly:
`account_achievements.rs`, `app_event_speech_pacer.rs`, `app_speech_audio.rs`, `playtest_harness.rs`, `speech.rs`, `speech_live.rs`, `states_driving_multilane_speech.rs`, `states_main_menu_settings.rs`, `updater.rs`.

`freight-fate/tests/it/` focused on speech/delivery behaviour (via `TestApp` capture):
`speech.rs` (1020 lines, backend policy + capture + apply_speech_settings + braille), `speech_live.rs` (239, real Prism; skips under `FREIGHT_FATE_NO_SPEECH`), `app_speech_audio.rs`, `audio_speech_audio.rs`, `app_event_speech_pacer.rs`, `app_main_channel_pacing.rs`, `app_driving_speech_ladder.rs`, `states_driving_speech_ladder.rs`, `app_speech_ducking.rs`, `audio_speech_ducking.rs`, `app_message_review.rs`, `transcript_menu_stop_speech.rs`, `transcript_tutorial_verbosity.rs`.

Nearly every other `tests/it/states_*.rs` and `transcript_*.rs` reads speech through `TestApp`'s capture (`main_lines`/`event_calls`/`transcript`), so any change to `CaptureSpeech`'s query API or default `CaptureProfile` reaches them all.

`tests/it/speech.rs` tests coupled to FF `Settings`: `apply_speech_settings_records_the_voice_actually_bound` (651), `capture_apply_speech_settings_keeps_the_saved_voice_name` (835), `apply_speech_settings_keeps_a_chosen_voice_the_sink_has_not_bound_yet` (854), `apply_speech_settings_pushes_braille_only` (1005) -- they build `ff_core::settings::Settings { sapi_events, event_backend, speech_rate, speech_voice, .. }`. Coupled to `SpokenMessage`: `capture_resolves_spoken_messages_by_its_terse_flag` (713).

---

## 8. Environment variables

In the speech stack:
| Var | Where | Effect |
|---|---|---|
| `FREIGHT_FATE_NO_SPEECH` | const `speech.rs:108`; read `speech/live.rs:89` (`env::var_os(super::NO_SPEECH_ENV).is_some_and(|value| !value.is_empty())`); log text `live.rs:90` | Any non-empty value: `Speech::start` returns disabled (no Prism loaded). |
| `FREIGHT_FATE_SPEECH_BACKEND` | const `speech.rs:111`; read `speech/live.rs:86-88` | Non-empty: forced main-backend name, tried first by `pick_backend_gated`; falls back to automatic if unusable/missing. Kept in `Speech.override_name` for every `refresh`. |
| `FREIGHT_FATE_STREAM_TRANSCRIPT` | literal `speech/live.rs:17` | Non-empty: `println!("{text}")` for each line that reached a voice (main and event), including the "Speech is now using X." announcement. |

Elsewhere in FF (callers, stay in FF): `app.rs:360` (reads `FREIGHT_FATE_NO_SPEECH` to force SDL dummy drivers), `app/testing.rs:158` (sets it), `main.rs:640` (sets it for a child env), `main.rs:670` (sets `FREIGHT_FATE_STREAM_TRANSCRIPT`), `app/sdl_shell.rs:135` (comment), `tests/it/playtest_harness.rs:113`, `tests/it/speech_live.rs:19, 101, 106`. `app/logging.rs` reads `FREIGHT_FATE_LOG` (not speech).

No env reads in `speech_delivery.rs`, `speech_pacing*`, `threaded.rs`, `backend.rs`, `capture.rs`, `fakes.rs`.

---

## 9. Transcript log target

`freight-fate/src/app/speech_delivery.rs:24-33`:
```rust
/// The logger every spoken line lands in too, so a logged playtest reads as
/// a transcript of what the player heard -- the most faithful record for an
/// audio-first game. Same name as the Python logger so the lines grep alike.
pub const TRANSCRIPT_TARGET: &str = "freight_fate.transcript";

macro_rules! transcript {
    ($($arg:tt)*) => {
        log::info!(target: TRANSCRIPT_TARGET, $($arg)*)
    };
}
```
Emission sites (all in `speech_delivery.rs`):
- 315-319 `"[ladder] {} silenced: {}"` (say_with)
- 328 `"{}"` (main line spoken)
- 432 `"[pacer] cut line requeued: {}"`
- 467 `"[pacer] cut line mostly heard, dropped: {}"`
- 635-639 `"[ladder] {} silenced: {}"` (say_event_with)
- 647-651 `"[ladder] {} already said: {}"`
- 677 `"[pacer] stale ambient dropped: {}"`
- 683 `"[event] {}"` (event line spoken)
- 697 `"[pacer] stale event backlog flushed"`

Also emitted outside: `states/driving_updates/frame.rs:97-104` (`"[jake] stage ..."`, trucking). Re-exported at `app.rs:50`. `app/logging.rs` does not special-case the target (it just interleaves all targets in one file; doc line 4).

Transcript lines are written BEFORE the sink call, in the delivery layer; the sinks themselves never log to this target. (`live.rs` only `println!`s under `FREIGHT_FATE_STREAM_TRANSCRIPT`.)

---

## 10. `unsafe`, platform cfgs, Windows-specific code

`unsafe`:
1. `speech/backend.rs:246-253` -- lifetime extension of a prismer backend:
   ```rust
        // SAFETY: the borrow is of `*self.ctx`, a boxed context that never
        // moves and is dropped only with this `PrismShared`, after the
        // `instances` field that owns every backend (fields drop in
        // declaration order). Each `PrismVoice` holds this owner by `Rc`, so
        // no backend can be reached after its context is gone.
        Ok(unsafe {
            std::mem::transmute::<prismer::Backend<'_>, prismer::Backend<'static>>(backend)
        })
   ```
2. `speech/backend.rs:424-451` -- Toolhelp32 process walk in `narrator_running` (Windows only).

Not `unsafe` but deliberate leaks: `std::mem::forget(inner)` on an abandoned worker (`threaded.rs:406`).

cfgs:
- `#[cfg(windows)] pub fn narrator_running()` (`backend.rs:416`), `#[cfg(not(windows))]` stub returning `false` (455-458).
- `#[cfg(test)]` modules: `backend.rs:15`, `threaded.rs:630, 637, 863, 867`; `speech_pacing.rs:1100`.
- No `cfg` in `live.rs`, `capture.rs`, `fakes.rs`, `speech.rs`, `speech_delivery.rs`.

Windows-specific knowledge in logic (not cfg-gated): backend names `"UIA"` (`backend.rs:521`, `live.rs:599` -> announced as `"Narrator"`), `"narrator.exe"` (`backend.rs:443`), `EVENT_BACKEND = "SAPI"` (`speech.rs:96`), OneCore pitch-preserve `name == "onecore" || name == "one_core"` and `value == 0.5` (`backend.rs:582-585`), `UIA_LAST_RESORT_PRIORITY = -1`. `CaptureProfile::full_voice` hard-codes NVDA/SAPI/OneCore/David/Zira.

Build-time (`freight-fate/build.rs`):
- `delay_load_prism_backends()` (52-63): MSVC only; reads `DEP_PRISMER_DELAY_LOAD_DLLS` and emits `cargo:rustc-link-arg=/DELAYLOAD:{dll}`. `rustc-link-arg` only affects the package's own final link, and `DEP_PRISMER_*` is only visible to DIRECT dependents of prismer. A game that depends on `hearken-speech` but not on `prismer` gets neither.
- `link_prism_system_libraries()` (72-119): Linux/macOS `cargo:rustc-link-lib=static:+whole-archive,-bundle=prism` (backends self-register from static initialisers); macOS frameworks Foundation, AVFoundation, AppKit, IOKit, CoreFoundation + `objc`; Linux pkg-config for `speech-dispatcher`, `glibmm-2.68`, `giomm-2.68`.
- `link_libstdcxx_statically()` (128-149): Linux.
- prismer 0.1.3 `build.rs` re-publishes `DEP_PRISM_DELAY_LOAD_DLLS` as `cargo:delay_load_dlls=` (it has `links = "prismer"`). prism-sys 0.1.3 builds Prism with the `cmake` crate.

---

## 11. Lucky Break comparison

LB repo: `C:\Users\joshu\gh-projects\lucky-break` @ `a101970`.
Files: `crates/lucky-break/src/speech/mod.rs` (153 lines), `crates/lucky-break/src/speech/prism.rs` (237 lines). LB depends on its own `prism = { path = "../prism" }` (`crates/lucky-break/Cargo.toml:25`), a hand-written wrapper (`crates/prism/src/lib.rs`, 941 lines) over `crates/prism-sys`, which **loads a vendored prismatoid 0.17.3 `prism.dll` at runtime via `libloading`** (prism-sys/Cargo.toml:13). `speech/prism.rs` is the only user of `prism::` in LB. LB's `build.rs` has no Prism delay-load or static-link code.

### LB `SpeechSink` (verbatim, `speech/mod.rs:14-30`)
```rust
/// Where interface speech goes. `say`/`stop`/`set_rate` never block the game
/// loop; `poll` drains whatever a worker-backed sink reports about itself.
pub trait SpeechSink: Send {
    /// Speak `text`. `interrupt` cuts off the current utterance; `false`
    /// queues behind it.
    fn say(&mut self, text: &str, interrupt: bool);
    /// Stop any speech in progress.
    fn stop(&mut self);
    /// Called once per frame; drains worker status for sinks that have one.
    fn poll(&mut self);
    /// Whether a real screen reader or TTS engine answered.
    fn available(&self) -> bool;
    /// The active backend's name (`"NVDA"`, `"SAPI"`, `"console"`, `"capture"`).
    fn name(&self) -> String;
    /// Speech rate as a multiplier: `0.5..=2.0`, `1.0` is normal.
    fn set_rate(&mut self, rate: f64);
}
```
Impls: `ConsoleSpeech` (37-55; `available` false, `name` `"console"`), `CaptureSpeech { pub lines: Transcript }` (59-100; `new() -> (Self, Transcript)` where `type Transcript = Arc<Mutex<Vec<(String, bool)>>>` using `parking_lot::Mutex`; `available` **true**, `name` `"capture"`), `PrismSpeech` (prism.rs:42-183). `choose_speech()` (105-119): `LUCKY_BREAK_NO_SPEECH == "1"` exactly -> console; else `PrismSpeech::spawn()` or console.

LB `PrismSpeech`: `ctx.acquire_best()` (prism.rs:63; the exact call FF rejects, see `backend.rs:464-475`); unbounded `mpsc::channel()` (74); thread `"speech"` (77); coalescing only for main-channel says (95-129); `impl Drop` sends `Shutdown` and **joins with no timeout** (176-183); rate is a multiplier mapped relative to the backend's reported base rate: `(f64::from(base) * multiplier).clamp(0.0, 1.0) as f32` (55-57). No event voice, no re-detection, no braille, no watchdog.

### How LB calls speech
Through `GameContext` wrappers (`crates/lucky-break/src/app/mod.rs`):
```rust
    pub fn say(&mut self, text: impl Into<String>) {          // 167-171
        let text = text.into();
        self.speech.say(&text, true);
        self.record(text);
    }
    pub fn say_queued(&mut self, text: impl Into<String>) {   // 174-178
        let text = text.into();
        self.speech.say(&text, false);
        self.record(text);
    }
    fn record(&mut self, text: String) {                      // 180-190
        log::info!(target: "lucky_break.transcript", "{text}");
        self.transcript.push(text);   // capped at TRANSCRIPT_CAP = 2000 (37)
        ...
    }
    pub fn stop_speech(&mut self) { self.speech.stop(); }     // 192-194
```
128 `ctx.say(`/`ctx.say_queued(` call sites in `src/` (28 of them `say_queued`). Five representative patterns:
1. `states/bar.rs:126` -- `ctx.say(format!("{bankroll} dollars."));` (interrupting readout)
2. `states/bar.rs:135` -- `ctx.say_queued(format!("{word} served. {bankroll} dollars."));` (queued follow-up)
3. `states/blackjack.rs:461` -- `ctx.say_queued("Insurance? Y for yes, N for no.");`
4. `states/settings.rs:125-126` -- `ctx.settings.speech_rate = next; ctx.speech.set_rate(next);` (direct sink call; also `app/mod.rs:276` `self.speech.set_rate(self.settings.speech_rate);` in `apply_volumes`)
5. `states/title.rs:74-77` -- `format!("{WINDOW_TITLE}, version {VERSION}. {}, {}.", ctx.speech.name(), ctx.audio.backend_name())` (direct query)

Direct sink calls elsewhere: `app/mod.rs:424` `self.ctx.speech.poll();` (per frame), `app/mod.rs:583` `let speech: Box<dyn SpeechSink> = choose_speech();`, `app/mod.rs:320-333` test context `CaptureSpeech::new()` -> `(speech, transcript)`; tests read `TestTranscript = Arc<Mutex<Vec<(String, bool)>>>` (`app/mod.rs:41`), e.g. `states/bar.rs:162-172`, `states/blackjack.rs:562-573`, `states/online.rs:527-533`. Example `crates/lucky-break/examples/say.rs` uses `choose_speech()`, `.name()`, `.available()`, `.say(..)`. CI sets `LUCKY_BREAK_NO_SPEECH: "1"` (`.github/workflows/rust.yml:37`). LB `Settings.speech_rate: f64` default `1.0` (`crates/lucky-break/src/settings.rs:21, 70`), range 0.5..2.0 (settings UI).

### What LB must change to adopt the FF-shaped trait
- `poll()` -> `poll(dt: f64)` (1 call, `app/mod.rs:424`).
- `name()` -> `backend_name()` (title.rs:76, examples/say.rs, tests pinning `"capture"`/`"console"`).
- `set_rate(multiplier)` -> `configure(Some(fraction), None, None, None)`; rate unit changes from multiplier 0.5..2.0 (1.0 normal, relative to backend base) to absolute 0..1 fraction (FF default 0.5). Needs a settings migration or a mapping.
- Drop the `Send` supertrait assumption (LB never moves the sink across threads; only `PrismSpeech` internals do).
- `choose_speech()` -> construct `ThreadedSpeech::spawn()` (with LB's env names) or `NullSpeech`; replace `ConsoleSpeech` with `NullSpeech` (`backend_name` becomes `"none"`).
- `CaptureSpeech` shape differs: LB's `new()` returns `(Self, Arc<Mutex<Vec<(String,bool)>>>)` and reports `available() == true`, `name() == "capture"`; FF's returns `Self`, owns its entries, defaults to `available() == false`, `backend_name() == "none"`, and is shared via an `Rc<RefCell<..>>` adapter (`SharedCapture`). LB's `headless_for_tests` and every `TestTranscript` reader need an adapter, or hearken should ship a shared-handle capture.
- Shutdown: LB relies on `Drop` + unbounded join; FF-shaped sink needs an explicit `ctx.speech.shutdown()` at quit (3 s bound).
- LB drops `crates/prism`, `crates/prism-sys`, the vendored prismatoid DLLs, and `libloading`; it gains prismer's source build (CMake + C++ toolchain in CI) and must add the delay-load / whole-archive build-script plumbing (section 10).
- Transcript target: LB logs `"lucky_break.transcript"` in its own `GameContext::record`; nothing in the sink logs, same as FF, so this is unaffected unless the delivery layer moves into hearken.
- `say_event` / event voice / braille / previews are new capability; LB can ignore them (call only `say`, `stop`).

---

## Cut list

Strip or parameterize when lifting into `hearken-speech`:

1. **Env var names** -- `NO_SPEECH_ENV = "FREIGHT_FATE_NO_SPEECH"` (`speech.rs:108`), `SPEECH_BACKEND_ENV = "FREIGHT_FATE_SPEECH_BACKEND"` (`speech.rs:111`), literal `"FREIGHT_FATE_STREAM_TRANSCRIPT"` (`live.rs:17`), log text `live.rs:90`. Replace with a config value (e.g. an env prefix `&'static str` passed to `Speech::new` / `ThreadedSpeech::spawn`), so FF keeps `FREIGHT_FATE_*` and LB gets `LUCKY_BREAK_*`. Note LB checks `== "1"`, FF checks non-empty.
2. **`apply_speech_settings(sink, &mut ff_core::settings::Settings)`** (`speech.rs:284-306`) -- replace `Settings` with a hearken-owned struct holding `separate_event_voice: bool` (FF `sapi_events`), `event_backend: String`, `rate/pitch/volume: f64`, `voice: String`, `braille_only: bool`; keep the write-back of the substitute event backend. FF maps its `Settings` onto it.
3. **`CaptureSpeech::say_message` / `say_event_message`** (`capture.rs:213-226`) and the `terse` flag -- either move `SpokenMessage` + `terse_silent` (`speech_text.rs:68-177`) into hearken-speech (ff-core then re-exports it), or drop those two methods and the terse flag from the capture. Everything else in `speech_text.rs` stays in ff-core.
4. **Pacer re-export** `pub use ff_core::speech_pacing::{EventPriority, EventSpeechPacer};` (`speech.rs:74`) -- remove from the M1 crate. If the pacer moves later: lift `EventSpeechPacer`, `EventPriority`, `Clock`, `Valid`, `Cut`, `monotonic_seconds`, `DeliveryStatus`/receipts; replace the `category: Option<SpeechCategory>` parameters (`track`, `note_interrupt`, `note_queued`) with a `never_rescue: bool` (the only use is the `Confirmation` check at `speech_pacing.rs:690`); keep `SpeechCategory`, `Disposition`, `DRIVING_SPEECH_*`, `LADDER_EARCONS`, `disposition_for` in ff-core; consider renaming `Route` and making `CHARS_PER_S`/`BASE_UTTERANCE_S`/budgets configurable.
5. **`speech_delivery.rs` stays in FF for M1** -- it is `impl GameContext`, reads FF `Settings`, `Profile`, the state stack, `MessageLog`, the sound catalog and FF's `Audio` trait. A later generic `SpeechDelivery` would need hooks for: review-log add, earcon play, duck set/restore (`set_speech_duck`), "paces main speech" flag, "ladder applies" flag, and the transcript target name.
6. **Transcript target** `"freight_fate.transcript"` (`speech_delivery.rs:27`) -- not in the sinks, so nothing to cut for M1; if delivery moves, make the target a parameter.
7. **Hard-coded Windows names** -- `EVENT_BACKEND = "SAPI"` (`speech.rs:96`) is the default event-voice preference; make it a parameter or keep as a default. `"UIA"`/`"narrator.exe"`/OneCore pitch hack are Prism-generic knowledge; keep.
8. **`ThreadedSpeech::spawn`** hard-wires `live::Speech::new()`/`new_after_wedge()` (`threaded.rs:337-345`); it will need the env/config from item 1 threaded through (the factory is `Fn(bool)` and runs on the worker, so capture the config by value).
9. **Build plumbing** -- a library cannot apply `/DELAYLOAD` for the game binary, and games that stop depending on `prismer` directly lose `DEP_PRISMER_DELAY_LOAD_DLLS`. Either (a) give hearken-speech `links = "hearken_speech"` and a `build.rs` that re-emits `cargo:delay_load_dlls={DEP_PRISMER_DELAY_LOAD_DLLS}` (mirroring prismer's own passthrough), with each game's `build.rs` reading `DEP_HEARKEN_SPEECH_DELAY_LOAD_DLLS`; or (b) games keep a direct `prismer` dependency for the metadata. The Linux/macOS `static:+whole-archive,-bundle=prism` link-lib, frameworks, pkg-config and static libstdc++ (`build.rs:72-149`) can move into hearken-speech's own `build.rs` (`rustc-link-lib` propagates to the final link).
10. **Cosmetic** -- tester names, dates, "I-77", "road voice", "driving events" in doc comments across `speech.rs`, `threaded.rs`, `live.rs`, `backend.rs`, `capture.rs` (Python-port references too). `CaptureProfile::full_voice` NVDA/SAPI/David/Zira is fine as a generic fixture.
11. **Tests to move with the crate**: `speech/backend/recovery_tests.rs`, `speech/threaded/recovery_tests.rs`, `speech/threaded/tests.rs`, and the backend/capture/fakes parts of `tests/it/speech.rs` (all but the four `apply_speech_settings` tests, which move once item 2 lands, and `capture_resolves_spoken_messages_by_its_terse_flag`, which follows item 3). `speech_live.rs` moves too (needs item 1's env name).
