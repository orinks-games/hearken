//! Hello Hearken: the smallest complete Hearken game.

mod practice;
mod sound;
mod title;

use hearken::{Action, Chord, ControllerButton, Key, PadChord};

pub use practice::Practice;
pub use title::Title;

/// The help document, compiled in.
pub const HELP: &str = include_str!("../help.md");

pub struct HelloGame;

impl hearken::Game for HelloGame {
    const TITLE: &'static str = "Hello Hearken";
    const DATA_DIR: &'static str = "HelloHearken";
    const ENV_PREFIX: &'static str = "HELLO_HEARKEN";
    type Event = ();
    type Data = ();
    type Settings = ();
    type Action = HelloAction;
    type Category = hearken::NoCategory;
}

/// The controls the player can move to other keys and buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HelloAction {
    Count,
    Throttle,
}

impl Action for HelloAction {
    fn all() -> &'static [Self] {
        &[HelloAction::Count, HelloAction::Throttle]
    }
    fn id(self) -> &'static str {
        match self {
            HelloAction::Count => "count",
            HelloAction::Throttle => "throttle",
        }
    }
    fn label(self) -> &'static str {
        match self {
            HelloAction::Count => "Count",
            HelloAction::Throttle => "Engine",
        }
    }
    fn default_keys(self) -> &'static [Chord] {
        match self {
            HelloAction::Count => const { &[Chord::plain(Key::Space)] },
            HelloAction::Throttle => const { &[Chord::plain(Key::Up)] },
        }
    }
    fn default_pad(self) -> &'static [PadChord] {
        match self {
            HelloAction::Count => const { &[PadChord::plain(ControllerButton::A)] },
            HelloAction::Throttle => const { &[PadChord::plain(ControllerButton::X)] },
        }
    }
}
