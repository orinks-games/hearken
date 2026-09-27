//! What the engine needs to know about one game.

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::bindings::Action;
use crate::ctx::Ctx;

pub trait Game: Sized + 'static {
    /// The window title, and the name the game is known by.
    const TITLE: &'static str;
    /// The folder under the platform's local data folder that holds the
    /// settings and the session log. Also names the single-instance lock.
    const DATA_DIR: &'static str;
    /// Prefix of the game's environment overrides: `<PREFIX>_DATA_DIR`,
    /// `<PREFIX>_LOG_FILE`, `<PREFIX>_NO_SPEECH`, `<PREFIX>_SPEECH_BACKEND`,
    /// `<PREFIX>_STREAM_TRANSCRIPT`.
    const ENV_PREFIX: &'static str;

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
}
