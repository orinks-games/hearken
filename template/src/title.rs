use hearken::menu::{Menu, MenuCore, MenuItem};
use hearken::screens::{HelpScreen, SettingsScreen};
use hearken::Ctx;

use crate::{HelloGame, Practice, HELP};

pub struct Title {
    menu: MenuCore<Self, HelloGame>,
}

impl Title {
    pub fn new() -> Self {
        Title {
            menu: MenuCore::new("Hello Hearken"),
        }
    }
}

impl Default for Title {
    fn default() -> Self {
        Self::new()
    }
}

impl Menu for Title {
    type Game = HelloGame;

    fn name(&self) -> &'static str {
        "title"
    }

    fn menu(&self) -> &MenuCore<Self, HelloGame> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, HelloGame> {
        &mut self.menu
    }

    fn build_items(&mut self, _ctx: &mut Ctx<HelloGame>) -> Vec<MenuItem<Self, HelloGame>> {
        vec![
            MenuItem::new("Play", |_s: &mut Self, ctx: &mut Ctx<HelloGame>| {
                ctx.push(Practice::new())
            })
            .help("Go to the practice room."),
            MenuItem::new("Settings", |_s: &mut Self, ctx: &mut Ctx<HelloGame>| {
                ctx.push(SettingsScreen::<HelloGame>::new())
            })
            .help("Speech, the controls, and the controller."),
            MenuItem::new("Help", |_s: &mut Self, ctx: &mut Ctx<HelloGame>| {
                ctx.push(HelpScreen::<HelloGame>::new(HELP))
            })
            .help("How to play."),
            MenuItem::new("Quit", |_s: &mut Self, ctx: &mut Ctx<HelloGame>| ctx.quit())
                .help("Close the game."),
        ]
    }

    /// The root of the stack never pops: that would end the run with no
    /// warning.
    fn on_escape(&mut self, ctx: &mut Ctx<HelloGame>) {
        ctx.say("Press Enter on Quit to leave.");
    }
}
