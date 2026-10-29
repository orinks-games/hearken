//! The saved settings: the engine's own, and the game's, in one
//! `settings.json`.

use std::collections::BTreeMap;
use std::io;

use hearken_audio::Bus;
use hearken_speech::SpeechSettings;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::bindings::SavedBindings;
use crate::paths::Paths;

/// What the player hears when their settings file could not be read.
pub const UNREADABLE_NOTICE: &str =
    "Your settings file could not be read, so the game is using its default settings.";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(bound(
    serialize = "GS: Serialize",
    deserialize = "GS: DeserializeOwned + Default"
))]
pub struct Settings<GS> {
    #[serde(default)]
    pub speech: SpeechSettings,
    #[serde(default)]
    pub menus: MenuSettings,
    #[serde(default)]
    pub controller: ControllerSettings,
    #[serde(default)]
    pub bindings: SavedBindings,
    #[serde(default)]
    pub audio: AudioSettings,
    /// The player's online account, for a game with online play.
    #[serde(default, skip_serializing_if = "OnlineSettings::is_empty")]
    pub online: OnlineSettings,
    /// Discord status, for a game that offers it.
    #[serde(default, skip_serializing_if = "DiscordSettings::is_default")]
    pub discord: DiscordSettings,
    #[serde(default)]
    pub game: GS,
}

/// Whether Discord shows what the player is doing, for a game that offers
/// it (`Game::discord`). On by default.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct DiscordSettings {
    pub enabled: bool,
}

impl Default for DiscordSettings {
    fn default() -> Self {
        DiscordSettings { enabled: true }
    }
}

impl DiscordSettings {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// A secret kept in `settings.json`: never shown by `Debug`, so it stays
/// out of logs.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(pub String);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(hidden)")
    }
}

/// The player's online account, saved in `settings.json`.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OnlineSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<Secret>,
    /// The name the server knows the player by.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl OnlineSettings {
    pub fn is_empty(&self) -> bool {
        self.token.is_none() && self.name.is_none()
    }
}

/// How menus read their rows.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct MenuSettings {
    /// Follow each row with where it is: "Beta. 2 of 3."
    pub announce_position: bool,
}

impl Default for MenuSettings {
    fn default() -> Self {
        MenuSettings {
            announce_position: true,
        }
    }
}

/// Whether the game listens to a game controller.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct ControllerSettings {
    pub enabled: bool,
}

impl Default for ControllerSettings {
    fn default() -> Self {
        ControllerSettings { enabled: true }
    }
}

/// The player's saved audio preferences: per-bus volumes, and whether other
/// buses duck while speech is talking.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct AudioSettings {
    /// Percent volume (0-100) by bus name (`Bus::name`); a bus with no entry
    /// uses `default_volume`. Kept as a map, not an array, so a bus a game
    /// dropped stays in the file instead of being silently lost.
    ///
    /// Deserialized leniently (`deserialize_volumes`): a hand-edited number
    /// out of range is clamped to `0..=100` and rounded rather than failing
    /// the whole settings file, and an entry that is not a number at all
    /// (`"loud"`) is dropped with a warning instead of it or anything else
    /// in the file being lost.
    #[serde(default, deserialize_with = "deserialize_volumes")]
    pub volumes: BTreeMap<String, u8>,
    /// Duck the ducked buses while gameplay speech is talking. Off until
    /// the player turns it on.
    pub duck_for_speech: bool,
}

impl AudioSettings {
    /// `bus`'s saved volume, capped at 100; `None` when nothing is saved
    /// for it. A hand-edited file cannot produce an out-of-range value here
    /// (`deserialize_volumes` already clamped it), but the cap stays as a
    /// second line of defense for a value set directly, such as in a test.
    pub fn saved_volume(&self, bus: Bus) -> Option<u8> {
        self.volumes.get(bus.name()).map(|&v| v.min(100))
    }

    /// `bus`'s saved volume, or the engine's own default when nothing is
    /// saved for it. A game with its own default (`SoundConfig::default_volume`)
    /// should read `Ctx::volume_percent` instead, which falls back to that
    /// instead of the engine's.
    pub fn volume(&self, bus: Bus) -> u8 {
        self.saved_volume(bus)
            .unwrap_or_else(|| default_volume(bus))
    }

    /// Saves `percent` (capped at 100) as `bus`'s volume.
    pub fn set_volume(&mut self, bus: Bus, percent: u8) {
        self.volumes
            .insert(bus.name().to_string(), percent.min(100));
    }
}

/// Deserializes `AudioSettings::volumes` leniently: any JSON number is
/// clamped to `0..=100` and rounded (half away from zero, so `75.5` becomes
/// `76`); an entry that is not a number is dropped with a warning naming the
/// bus. Never fails, so one bad entry in a hand-edited file cannot take the
/// rest of the settings down with it (Review Focus 5).
fn deserialize_volumes<'de, D>(deserializer: D) -> Result<BTreeMap<String, u8>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = BTreeMap::<String, serde_json::Value>::deserialize(deserializer)?;
    let mut volumes = BTreeMap::new();
    for (bus, value) in raw {
        match value.as_f64() {
            Some(n) => {
                volumes.insert(bus, n.clamp(0.0, 100.0).round() as u8);
            }
            None => {
                log::warn!("settings: {bus}'s saved volume ({value}) is not a number; dropping it");
            }
        }
    }
    Ok(volumes)
}

/// A bus's volume before the player changes it: 100 for `Bus::MASTER`, 80
/// for every other bus.
pub fn default_volume(bus: Bus) -> u8 {
    if bus == Bus::MASTER {
        100
    } else {
        80
    }
}

impl<GS: Serialize + DeserializeOwned + Default> Settings<GS> {
    /// The saved settings, with graceful fallback on any read or parse failure.
    ///
    /// A missing file gives the defaults quietly. A file that is not valid
    /// settings JSON (including invalid UTF-8 sequences) is moved aside to
    /// `settings.json.bad` so the next save cannot destroy it, and gives the
    /// defaults and a notice for the player. A file the game cannot open
    /// (permissions, lock, etc.) gives the defaults and a notice and is left
    /// where it is.
    pub fn load(paths: &Paths) -> (Self, Option<String>) {
        let file = paths.settings_file();
        let bytes = match std::fs::read(&file) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return (Self::default(), None),
            Err(e) => {
                log::warn!("could not read {}: {e}; using defaults", file.display());
                return (Self::default(), Some(UNREADABLE_NOTICE.to_string()));
            }
        };
        match serde_json::from_slice(&bytes) {
            Ok(settings) => (settings, None),
            Err(e) => {
                log::warn!("could not parse {}: {e}; using defaults", file.display());
                let bad = file.with_extension("json.bad");
                if let Err(e) = std::fs::rename(&file, &bad) {
                    log::warn!("could not keep the bad settings file aside: {e}");
                }
                (Self::default(), Some(UNREADABLE_NOTICE.to_string()))
            }
        }
    }
}

/// Writes `bytes` to `path`, readable by this user alone on Unix: the
/// settings can hold an online account's token.
fn write_private(path: &std::path::Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(0o600);
        // A leftover file keeps its old mode through `open`; set it again.
        if let Ok(meta) = std::fs::metadata(path) {
            if meta.permissions().mode() & 0o077 != 0 {
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            }
        }
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)
}

impl<GS: Serialize> Settings<GS> {
    /// Write the settings as pretty JSON, through a temporary file, so a
    /// crash mid-write never leaves half a file behind.
    pub fn save(&self, paths: &Paths) -> io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        let file = paths.settings_file();
        let tmp = file.with_extension("json.tmp");
        write_private(&tmp, json.as_bytes())?;
        std::fs::rename(&tmp, &file)
    }
}
