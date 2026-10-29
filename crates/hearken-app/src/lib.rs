//! The shell of an audio-first, screen-reader-first game.
//!
//! Extracted from Lucky Break Casino (commit a101970) and Freight Fate (commit c5d3ef8d).

#[cfg(feature = "agent-server")]
pub mod agent_server;
pub mod app;
pub mod bindings;
pub mod clock;
pub mod commentary;
pub mod controller;
pub mod ctx;
pub mod discord;
pub mod game;
pub mod held_keys;
pub mod input;
pub mod logging;
pub mod menu;
#[cfg(feature = "online")]
pub mod online;
pub mod paths;
pub mod run;
pub mod screens;
pub mod sdl_shell;
pub mod settings;
pub mod shell;
pub mod single_instance;
pub mod sound;
pub mod soundtrack;
pub mod speech_delivery;
pub mod state;
#[cfg(feature = "testing")]
pub mod testing;
pub mod text;

pub use app::App;
pub use bindings::{Action, Chord, KeyBindings, NoAction, PadChord};
pub use clock::LocalTime;
pub use commentary::{Commentary, Line};
pub use controller::{ControllerManager, InputDevice};
pub use ctx::{Ctx, TRANSCRIPT_CAP, TRANSCRIPT_TARGET};
pub use discord::{Discord, DiscordConfig, Presence};
pub use game::Game;
pub use hearken_audio as audio;
#[cfg(feature = "kira")]
pub use hearken_audio::KiraBackend;
pub use hearken_audio::{
    Audio, Backend, BackendError, Bus, Call, Channel, ClipSource, Fades, LayeredClips, Listener,
    LiveSound, LoopRegion, LooseClips, MadeClips, MemoryClips, Mix, Recorder, RecordingBackend,
    SilentBackend, Sound, Source, SpatialConfig, Vec3,
};
pub use hearken_seal as seal;
pub use hearken_seal::{Opened, Tamper, Vault};
pub use hearken_speech::{
    Category, EventPriority, Level, NoCategory, PacerConfig, Rule, Verbosity,
};
pub use hearken_synth as synth;
pub use held_keys::HeldKeys;
pub use input::{controller_key_map, ControllerAxis, ControllerButton, InputEvent, Key, Mods};
#[cfg(feature = "online")]
pub use online::{Connect, Online, OnlineConfig, OnlineError, Plan, Ticket};
pub use paths::Paths;
pub use run::{already_running_line, run};
pub use screens::SettingsRow;
pub use sdl_shell::SdlShell;
pub use settings::{
    default_volume, AudioSettings, ControllerSettings, DiscordSettings, MenuSettings,
    OnlineSettings, Settings,
};
pub use shell::{HeadlessShell, Shell, FRAME_SECONDS};
pub use sound::{SoundConfig, UiSound, UiSounds, BUS_LABELS};
pub use soundtrack::{
    Performer, Piece, Soundtrack, SoundtrackScreen, Stems, Track, Variation, Variety,
};
pub use speech_delivery::SayEvent;
pub use state::{Reveal, StackOp, State};
#[cfg(feature = "testing")]
pub use testing::Headless;
