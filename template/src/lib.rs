//! Hello Hearken: the smallest complete Hearken game.

mod practice;
mod title;

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
}
