//! The saved settings: the engine's own, and the game's, in one
//! `settings.json`.

use std::io;

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
    pub game: GS,
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

impl<GS: Serialize> Settings<GS> {
    /// Write the settings as pretty JSON, through a temporary file, so a
    /// crash mid-write never leaves half a file behind.
    pub fn save(&self, paths: &Paths) -> io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        let file = paths.settings_file();
        let tmp = file.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &file)
    }
}
