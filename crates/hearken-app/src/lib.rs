//! The shell of an audio-first, screen-reader-first game.
//!
//! Extracted from Lucky Break Casino (commit a101970) and Freight Fate (commit c5d3ef8d).

pub mod app;
pub mod controller;
pub mod ctx;
pub mod game;
pub mod held_keys;
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
pub use controller::{ControllerManager, InputDevice};
pub use ctx::{Ctx, TRANSCRIPT_CAP, TRANSCRIPT_TARGET};
pub use game::Game;
pub use held_keys::HeldKeys;
pub use input::{controller_key_map, ControllerAxis, ControllerButton, InputEvent, Key, Mods};
pub use paths::Paths;
pub use run::{already_running_line, run};
pub use sdl_shell::SdlShell;
pub use settings::{ControllerSettings, MenuSettings, Settings};
pub use shell::{HeadlessShell, Shell, FRAME_SECONDS};
pub use state::{StackOp, State};
pub use testing::Headless;
