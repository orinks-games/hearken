//! What a game hands the engine about its own sound: the earcons for common
//! UI moments, its own buses, and the spatial and ducking defaults.

use hearken_audio::{Bus, SpatialConfig};
use hearken_speech::PacerConfig;

use crate::settings::default_volume;

/// A UI moment the engine plays a short earcon for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UiSound {
    Open,
    Move,
    Select,
    Back,
    Tick,
    Error,
    Notify,
}

/// Which sound key plays for each [`UiSound`]; `None` plays nothing.
#[derive(Clone, Debug, PartialEq)]
pub struct UiSounds([Option<&'static str>; 7]);

impl Default for UiSounds {
    /// `hearken_synth`'s built-in `UI_*` earcons.
    fn default() -> Self {
        UiSounds([
            Some(hearken_synth::UI_OPEN),
            Some(hearken_synth::UI_MOVE),
            Some(hearken_synth::UI_SELECT),
            Some(hearken_synth::UI_BACK),
            Some(hearken_synth::UI_TICK),
            Some(hearken_synth::UI_ERROR),
            Some(hearken_synth::UI_NOTIFY),
        ])
    }
}

impl UiSounds {
    /// The key that plays for `sound`, or `None` if the game turned it off.
    pub fn get(&self, sound: UiSound) -> Option<&'static str> {
        self.0[sound as usize]
    }

    /// Points `sound` at `key`, or turns it off with `None`.
    pub fn set(&mut self, sound: UiSound, key: Option<&'static str>) {
        self.0[sound as usize] = key;
    }
}

/// The engine's own buses, in the order every settings menu lists them.
pub const BUS_LABELS: [(Bus, &str); 6] = [
    (Bus::MASTER, "Master"),
    (Bus::UI, "Interface"),
    (Bus::EFFECTS, "Effects"),
    (Bus::AMBIENCE, "Ambience"),
    (Bus::MUSIC, "Music"),
    (Bus::VOICE, "Voices"),
];

/// A game's own sound configuration: its buses, ducking, spatial model and UI
/// earcons. Every value is a default a game can change.
#[derive(Clone, Debug)]
pub struct SoundConfig {
    /// The game's own buses, listed after the built-in six.
    pub buses: Vec<(Bus, &'static str)>,
    /// Buses lowered while something else ducks them.
    pub duck_buses: Vec<Bus>,
    /// How far a duck lowers those buses: `0.5` is -6 dB, `1.0` is no duck.
    pub duck_level: f32,
    /// Seconds an earcon ducks the ducked buses for.
    pub earcon_duck_s: f64,
    /// Pacing for the gameplay event voice; `None` switches the pacer off.
    pub pacer: Option<PacerConfig>,
    /// The spatial positioning model's tuning.
    pub spatial: SpatialConfig,
    /// Which sound plays for each UI moment.
    pub ui: UiSounds,
    /// A bus's volume before the player changes it, for a bus with nothing
    /// saved in `settings.audio.volumes`. Defaults to `settings::default_volume`
    /// (100 for `Bus::MASTER`, 80 for the rest); read through
    /// `Ctx::volume_percent`, not `AudioSettings::volume` directly, so a
    /// game's own default is honoured.
    pub default_volume: fn(Bus) -> u8,
}

impl Default for SoundConfig {
    fn default() -> Self {
        SoundConfig {
            buses: Vec::new(),
            duck_buses: vec![Bus::MUSIC, Bus::AMBIENCE],
            duck_level: 0.5,
            earcon_duck_s: 0.25,
            pacer: Some(PacerConfig::default()),
            spatial: SpatialConfig::default(),
            ui: UiSounds::default(),
            default_volume,
        }
    }
}
