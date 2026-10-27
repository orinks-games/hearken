//! What the engine needs to know about one game.

use hearken_audio::{Backend, ClipSource, Vec3};
use hearken_speech::Verbosity;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::bindings::Action;
use crate::ctx::Ctx;
use crate::screens::settings::SettingsRow;
use crate::sound::SoundConfig;

pub trait Game: Sized + 'static {
    /// The window title, and the name the game is known by.
    const TITLE: &'static str;
    /// The folder under the platform's local data folder that holds the
    /// settings and the session log. Also names the single-instance lock.
    const DATA_DIR: &'static str;
    /// Prefix of the game's environment overrides: `<PREFIX>_DATA_DIR`,
    /// `<PREFIX>_LOG`, `<PREFIX>_LOG_FILE`, `<PREFIX>_NO_SPEECH`, `<PREFIX>_SPEECH_BACKEND`,
    /// `<PREFIX>_STREAM_TRANSCRIPT`, `<PREFIX>_AUDIO_BACKEND` (`silent` turns sound off),
    /// `<PREFIX>_SOUNDS` (the loose sounds folder). A release made with a
    /// sound pack ignores `<PREFIX>_SOUNDS`, and one made with `seal = true`
    /// ignores `<PREFIX>_DATA_DIR`, so neither can swap in other files.
    const ENV_PREFIX: &'static str;
    /// Whether a release made with `seal = true` still imports plain save
    /// files an older, unsealed version left (`Opened::Legacy`). While this
    /// is true, the vault leaves one small marker outside the data folder so
    /// a wiped folder cannot pass a hand-written plain file off as old saves.
    /// Leave it false (the default) and no marker is ever written; turn it
    /// off once players have had a sealing release.
    const IMPORTS_PLAIN_SAVES: bool = false;

    /// What the game's rules emit. The top screen receives each one in
    /// `State::on_event`.
    type Event;
    /// The game's own state and services, at `ctx.game`.
    type Data;
    /// The game's settings, saved under `"game"` in `settings.json`.
    type Settings: Serialize + DeserializeOwned + Default + Clone;
    /// The controls the player can move to other keys and buttons, from
    /// Settings. `hearken_app::NoAction` when there are none.
    type Action: Action;
    /// The game's own event categories for the verbosity table, or
    /// `hearken_app::NoCategory` for a game with none.
    type Category: hearken_speech::Category;

    /// The events the game's rules produced this frame. Called once per
    /// frame, after input.
    fn pump(_ctx: &mut Ctx<Self>, _dt: f64) -> Vec<Self::Event> {
        Vec::new()
    }

    /// Sees every pumped event before the top screen does: bookkeeping that
    /// must happen whichever screen is open.
    fn observe(_ctx: &mut Ctx<Self>, _event: &Self::Event) {}

    /// Called once when the run ends, before the settings are saved and
    /// speech shuts down.
    fn on_exit(_ctx: &mut Ctx<Self>) {}

    /// The game's own sound configuration: its buses, ducking, spatial model
    /// and UI earcons.
    fn sound() -> SoundConfig {
        SoundConfig::default()
    }

    /// What each event category does at each verbosity level the player can
    /// choose. Empty by default: the game has no categories to speak of.
    fn verbosity() -> Verbosity<Self::Category> {
        Verbosity::NONE
    }

    /// A sound backend the game supplies itself. Runs off the game loop,
    /// while the device opens. `None` uses the engine's own default;
    /// `Some(Err(reason))` reaches the player the same way a failed default
    /// backend would, through the "no sound device" notice.
    fn audio_backend() -> Option<Result<Box<dyn Backend>, String>> {
        None
    }

    /// Sounds the game carries inside itself, such as synthesized
    /// placeholders. A file of the same key in the sounds folder wins, so a
    /// real recording replaces a built-in sound with no code change.
    fn builtin_clips() -> Option<Box<dyn ClipSource>> {
        None
    }

    /// Whether something between the listener and a sound at `pos` should
    /// muffle it. Asked every frame for every positioned sound.
    fn occluded(_data: &Self::Data, _listener: Vec3, _pos: Vec3) -> bool {
        false
    }

    /// The Settings screen's rows, built-in ones first: a chance to hide,
    /// reorder or add to them. The default keeps every built-in row as is.
    /// The game's server for verified runs (`ctx.online`), or `None` for
    /// no online play. A build without a rules ID of its own (any build but
    /// a release made with `rules`) takes `<PREFIX>_ONLINE_URL` in its
    /// place, to try a local server.
    #[cfg(feature = "online")]
    fn online() -> Option<crate::online::OnlineConfig> {
        None
    }

    fn settings_rows(_ctx: &Ctx<Self>, rows: Vec<SettingsRow<Self>>) -> Vec<SettingsRow<Self>> {
        rows
    }
}
