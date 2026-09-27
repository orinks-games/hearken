//! The shell of an audio-first, screen-reader-first game.

pub mod input;
pub mod paths;
pub mod settings;
pub mod text;

pub use input::{ControllerAxis, ControllerButton, InputEvent, Key, Mods};
pub use paths::Paths;
pub use settings::Settings;
