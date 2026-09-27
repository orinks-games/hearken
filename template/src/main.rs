#![cfg_attr(windows, windows_subsystem = "windows")]
//! `hello-hearken`: the game's entry point.

fn main() {
    std::process::exit(hearken_app::run::<hello_hearken::HelloGame>((), || {
        Box::new(hello_hearken::Title::new())
    }));
}
