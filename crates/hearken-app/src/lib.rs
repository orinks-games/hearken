//! The shell of an audio-first, screen-reader-first game.

pub mod app;
pub mod ctx;
pub mod game;
pub mod input;
pub mod menu;
pub mod paths;
pub mod screens;
pub mod settings;
pub mod shell;
pub mod state;
pub mod testing;
pub mod text;

pub use app::App;
pub use ctx::{Ctx, TRANSCRIPT_CAP, TRANSCRIPT_TARGET};
pub use game::Game;
pub use input::{ControllerAxis, ControllerButton, InputEvent, Key, Mods};
pub use paths::Paths;
pub use settings::Settings;
pub use shell::{HeadlessShell, Shell, FRAME_SECONDS};
pub use state::{StackOp, State};
pub use testing::Headless;
