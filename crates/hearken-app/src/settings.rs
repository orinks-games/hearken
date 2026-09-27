//! The saved settings: the engine's own, and the game's, in one
//! `settings.json`.

use std::io;

use hearken_speech::SpeechSettings;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

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
    pub game: GS,
}

impl<GS: Serialize + DeserializeOwned + Default> Settings<GS> {
    /// The saved settings. A missing file gives the defaults quietly. A file
    /// that cannot be read or parsed gives the defaults too, is moved aside
    /// to `settings.json.bad` so the next save cannot destroy it, and returns
    /// a notice for the player.
    pub fn load(paths: &Paths) -> (Self, Option<String>) {
        let file = paths.settings_file();
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return (Self::default(), None),
            Err(e) => {
                log::warn!("could not read {}: {e}; using defaults", file.display());
                return (Self::default(), Some(UNREADABLE_NOTICE.to_string()));
            }
        };
        match serde_json::from_str(&text) {
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
