//! Hello Hearken: the smallest complete Hearken game.

mod practice;
mod title;

use hearken_app::{Action, Chord, ControllerButton, Key, PadChord};

pub use practice::Practice;
pub use title::Title;

/// The help document, compiled in.
pub const HELP: &str = include_str!("../help.md");

pub struct HelloGame;

impl hearken_app::Game for HelloGame {
    const TITLE: &'static str = "Hello Hearken";
    const DATA_DIR: &'static str = "HelloHearken";
    const ENV_PREFIX: &'static str = "HELLO_HEARKEN";
    type Event = ();
    type Data = ();
    type Settings = ();
    type Action = HelloAction;
    type Category = hearken_app::NoCategory;
}

/// The controls the player can move to other keys and buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HelloAction {
    Count,
}

impl Action for HelloAction {
    fn all() -> &'static [Self] {
        &[HelloAction::Count]
    }
    fn id(self) -> &'static str {
        "count"
    }
    fn label(self) -> &'static str {
        "Count"
    }
    fn default_keys(self) -> &'static [Chord] {
        const { &[Chord::plain(Key::Space)] }
    }
    fn default_pad(self) -> &'static [PadChord] {
        const { &[PadChord::plain(ControllerButton::A)] }
    }
}
