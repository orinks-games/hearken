//! The shell of an audio-first, screen-reader-first game.
//!
//! Extracted from Lucky Break Casino (commit a101970) and Freight Fate (commit c5d3ef8d).

pub mod app;
pub mod ctx;
pub mod game;
pub mod input;
pub mod logging;
pub mod menu;
pub mod paths;
pub mod run;
pub mod screens;
pub mod sdl_shell;
pub mod settings;
pub mod shell;
pub mod single_instance;
pub mod state;
pub mod testing;
pub mod text;

pub use app::App;
pub use ctx::{Ctx, TRANSCRIPT_CAP, TRANSCRIPT_TARGET};
pub use game::Game;
pub use input::{ControllerAxis, ControllerButton, InputEvent, Key, Mods};
pub use paths::Paths;
pub use run::{already_running_line, run};
pub use sdl_shell::SdlShell;
pub use settings::{MenuSettings, Settings};
pub use shell::{HeadlessShell, Shell, FRAME_SECONDS};
pub use state::{StackOp, State};
pub use testing::Headless;
