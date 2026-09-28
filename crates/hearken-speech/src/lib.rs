//! Screen reader output via Prism.
//!
//! Extracted from Freight Fate (commit c5d3ef8d).
//!
//! Prism is a screen reader abstraction layer that unifies NVDA, JAWS, SAPI,
//! VoiceOver, Speech Dispatcher, and many other backends behind one API. This
//! module wraps it in a small game-friendly interface that:
//!
//! * never crashes the game if speech is unavailable (silent fallback),
//! * picks the best backend that is actually usable on this machine: Prism's
//!   `acquire_best` cannot be trusted for this -- it returns the highest
//!   priority backend that already has a live cached instance (whatever the
//!   game happens to be holding), and otherwise ranks by static registry
//!   priority whether or not that screen reader is running -- so the registry
//!   is enumerated in priority order and every candidate is validated against
//!   its live `is_supported_at_runtime` check,
//! * treats Prism's `UIA` backend (the route to Narrator, via UI Automation
//!   notifications) as a gated last resort: it reports runtime support on every
//!   modern Windows whether or not anyone is listening, so it is skipped unless
//!   Narrator is actually running -- and even then it only wins when no other
//!   voice works, because the backend raises every notification with
//!   `NotificationProcessing_ImportantAll`, which Narrator queues without
//!   ever cancelling: interrupt and stop are no-ops, so menu browsing through
//!   it piles up unread items,
//! * keeps watching while the game runs: if the player switches screen readers
//!   mid-session (NVDA off, Narrator on, back to NVDA), a periodic health check
//!   re-detects the running one and reconnects speech instead of going silent,
//! * prefers `output` (speech + braille) and falls back to `speak`; with
//!   the braille-only setting on it sends every line, including events,
//!   to the main voice's `braille` instead and speaks nothing,
//! * can be disabled, or forced to a specific backend, through [`SpeechEnv`],
//!   whose environment variable names are chosen per game so two games on one
//!   machine never share them (useful for headless test suites and CI).
//!
//! # Threading: each Prism context stays on its speech worker
//!
//! Every call on a Prism [`prismer::Prism`] -- construction, health checks,
//! speaking, configuring and shutdown -- happens on the worker that created
//! it. [`live::Speech`] is deliberately not `Send` or `Sync`. Recovery may
//! leave an old generation inside an in-flight native call while a
//! replacement uses private backend instances. After that call returns, the
//! old worker stops dispatching operations and retains its native objects
//! until process exit.
//!
//! Running speech straight on the game loop's own thread means any spoken
//! line is a synchronous screen-reader/SAPI call the loop waits on -- and the
//! one time such a call wedges, the whole game freezes with it, permanently.
//! [`ThreadedSpeech`] avoids that: the context lives on a dedicated worker
//! thread (created there, never moved), the game loop only queues commands,
//! and a wedged backend costs sentences, never the frame. A direct sink such
//! as [`live::Speech`] or a capture remains useful for tests, where the
//! synchronous call is exactly what is wanted.
//!
//! # Shape of the module
//!
//! The [`SpeechSink`] trait is what a game needs from a speech object; the
//! Prism-backed [`live::Speech`] and the test doubles [`CaptureSpeech`] and
//! [`NullSpeech`] (`capture`) each implement it. The Prism calls themselves
//! sit behind the [`backend::VoiceRegistry`] / [`backend::VoiceBackend`]
//! traits so the selection logic -- `pick_backend`, the event voice, the
//! refresh poll -- can be tested against fake registries instead of real
//! screen readers.

pub mod backend;
pub mod capture;
pub mod fakes;
pub mod live;
pub mod pacer;
pub mod spoken;
pub mod threaded;
pub mod verbosity;

pub use backend::{
    narrator_running, pick_backend, pick_backend_gated, pick_event_backend,
    preserve_backend_default_pitch, usable, BackendId, PrismRegistry, PrismVoice, VoiceBackend,
    VoiceFeatures, VoiceRegistry,
};
pub use capture::{
    CaptureProfile, CaptureSpeech, ConfigureCall, NullSpeech, SharedCapture, SpeechChannel,
    SpokenEntry,
};
pub use live::{Speech, SpeechConfig};
pub use pacer::{
    monotonic_seconds, Clock, Cut, EventPriority, EventSpeechPacer, PacerConfig, Valid,
};
pub use spoken::{terse_silent, SpokenMessage};
pub use threaded::ThreadedSpeech;
pub use verbosity::{Category, Level, NoCategory, Rule, Verbosity};

/// Seconds between runtime health checks of the speech backend. Short enough
/// that a player who switches screen readers hears the game again within a few
/// seconds, long enough that the registry scan costs nothing per frame.
pub const REFRESH_INTERVAL_S: f64 = 3.0;

/// Preferred separate event voice per platform; the first one that exists wins.
/// These are the controllable software TTS engines, not screen readers: SAPI is
/// Windows, AVSpeech is macOS, Speech Dispatcher is Linux. `select_event_backend`
/// falls back to whatever the machine actually has, so this is only a hint.
pub const EVENT_BACKEND: &str = "SAPI";

/// Ranks the UIA backend below every other voice even while Narrator is
/// running. Prism's UIA backend raises all notifications with
/// `NotificationProcessing_ImportantAll`, which Narrator queues without ever
/// cancelling -- interrupt and stop are no-ops -- so menu browsing through it
/// stacks up unread items. Until that is fixed upstream (the fix is
/// `ImportantMostRecent` for the interrupt case), UIA is only for machines
/// where nothing else can speak at all: queued speech beats silence.
pub const UIA_LAST_RESORT_PRIORITY: i32 = -1;

/// Settings keys whose preview speaks through the voice that setting adjusts,
/// paired with the backend feature that voice must support.
pub const PREVIEW_FEATURES: [(&str, PreviewFeature); 4] = [
    ("speech_rate", PreviewFeature::Rate),
    ("speech_pitch", PreviewFeature::Pitch),
    ("speech_volume", PreviewFeature::Volume),
    ("speech_voice", PreviewFeature::Voice),
];

/// The backend capability a settings preview needs, one per adjustable
/// speech setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewFeature {
    Rate,
    Pitch,
    Volume,
    Voice,
}

impl PreviewFeature {
    /// The feature a settings key previews through, or `None` for a setting
    /// that is not voice-adjustable.
    pub fn for_setting(setting: &str) -> Option<Self> {
        PREVIEW_FEATURES
            .iter()
            .find(|(key, _)| *key == setting)
            .map(|(_, feature)| *feature)
    }

    /// Whether `features` lets a voice be adjusted along this axis.
    pub fn supported_by(self, features: VoiceFeatures) -> bool {
        match self {
            PreviewFeature::Rate => features.supports_set_rate,
            PreviewFeature::Pitch => features.supports_set_pitch,
            PreviewFeature::Volume => features.supports_set_volume,
            PreviewFeature::Voice => features.supports_set_voice,
        }
    }
}

/// What a game needs from a speech object, the public surface a game-facing
/// context talks to.
///
/// Two channels: `say` is the main voice (menus, info keys, everything the
/// player asked for), `say_event` the dedicated event voice (gameplay events,
/// warnings) that falls back to the main channel when no separate voice is
/// bound. `interrupt` cuts off the previous utterance on that channel.
///
/// The "Speech is now using X." line a backend switch produces is spoken by
/// the object itself, straight through the new main voice and bypassing any
/// message log -- exactly what `refresh(announce=true)` promises. No hook and
/// no returned list: a test double that wants to see it records what `say`
/// received.
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

    /// Choose which backend speaks events (`None` = the main voice).
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
    /// ("Output: braille only"): menus, readouts, and the events that would
    /// otherwise go to the separate event voice. Honoured only while the
    /// bound main voice can braille (`supports_braille`); with any other
    /// voice speech carries on unchanged, so the switch can never leave a
    /// player with nothing.
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

/// The environment overrides a game's speech reads, named after the game's
/// prefix so two games on one machine never share them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeechEnv {
    /// Any non-empty value: start with speech off, loading no voice.
    pub no_speech: String,
    /// A backend name tried first, for testing one voice.
    pub backend: String,
    /// Any non-empty value: print every line a voice spoke to stdout.
    pub stream_transcript: String,
}

impl SpeechEnv {
    pub fn with_prefix(prefix: &str) -> SpeechEnv {
        SpeechEnv {
            no_speech: format!("{prefix}_NO_SPEECH"),
            backend: format!("{prefix}_SPEECH_BACKEND"),
            stream_transcript: format!("{prefix}_STREAM_TRANSCRIPT"),
        }
    }

    /// Whether `no_speech` is set to something non-empty.
    pub fn disabled(&self) -> bool {
        std::env::var_os(&self.no_speech).is_some_and(|v| !v.is_empty())
    }
}

/// The speech settings a game saves: which voices speak and how.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SpeechSettings {
    /// Speak gameplay events through a separate software voice, so the
    /// screen reader cannot talk over them.
    pub separate_event_voice: bool,
    /// The voice that speaks events when `separate_event_voice` is on.
    pub event_backend: String,
    /// 0.0 to 1.0.
    pub rate: f64,
    /// 0.0 to 1.0.
    pub pitch: f64,
    /// 0.0 to 1.0.
    pub volume: f64,
    /// An installed voice's name; empty for the voice's default.
    pub voice: String,
    /// Send every line to the braille display and speak nothing.
    pub braille_only: bool,
    /// The verbosity level's id; empty for the game's first level.
    pub verbosity: String,
}

impl Default for SpeechSettings {
    fn default() -> Self {
        SpeechSettings {
            separate_event_voice: true,
            event_backend: EVENT_BACKEND.to_string(),
            rate: 0.5,
            pitch: 0.5,
            volume: 1.0,
            voice: String::new(),
            braille_only: false,
            verbosity: String::new(),
        }
    }
}

/// Reflect a game's saved speech settings on the sink.
///
/// Selects the event voice the player chose (or the main voice when
/// `separate_event_voice` is off), records the voice the sink binds in its
/// place when the saved one is not on this machine (a save made with one
/// voice opened on a machine without it) so the settings menu and later
/// sessions reflect reality, and pushes rate, pitch, volume and voice. One
/// definition of the settings side of speech, called at startup and after
/// every settings change.
///
/// The substitute is decided from the option list, never read back from the
/// sink: a threaded sink queues the switch and answers `event_backend_name`
/// from a snapshot that still names the voice bound before it, and writing
/// that stale name over the setting would turn a freshly chosen voice back
/// into the old one, both in memory and on disk at the next save. The
/// fallback mirrors `select_event_backend`: a preference missing from a
/// non-empty option list becomes the first option.
pub fn apply_speech_settings(sink: &mut dyn SpeechSink, settings: &mut SpeechSettings) {
    let preference = settings
        .separate_event_voice
        .then(|| settings.event_backend.clone());
    sink.select_event_backend(preference.as_deref());
    if settings.separate_event_voice {
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
    let voice = (!settings.voice.is_empty()).then(|| settings.voice.clone());
    sink.configure(
        Some(settings.rate),
        Some(settings.pitch),
        Some(settings.volume),
        voice.as_deref(),
    );
    sink.set_braille_only(settings.braille_only);
}
