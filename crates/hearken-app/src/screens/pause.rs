//! The pause menu: the way out of a game in progress.

use crate::ctx::Ctx;
use crate::game::Game;
use crate::menu::{menu_enter, Menu, MenuCore, MenuItem};
use crate::screens::{HelpScreen, SettingsScreen};

pub struct PauseScreen<G: Game> {
    menu: MenuCore<Self, G>,
    help_doc: &'static str,
    to_title: fn(&mut Ctx<G>),
}

impl<G: Game> PauseScreen<G> {
    /// `to_title` is what "Return to the title" does: usually saving and
    /// `ctx.reset_to(...)` the game's title screen.
    pub fn new(help_doc: &'static str, to_title: fn(&mut Ctx<G>)) -> Self {
        PauseScreen {
            menu: MenuCore::new("Paused")
                .with_intro_help("Up and Down move, Enter chooses, Escape goes back to the game."),
            help_doc,
            to_title,
        }
    }
}

impl<G: Game> Menu for PauseScreen<G> {
    type Game = G;

    fn name(&self) -> &'static str {
        "pause"
    }

    /// Holds the game's own sound and event speech while paused, so nothing
    /// keeps playing behind the menu.
    fn enter(&mut self, ctx: &mut Ctx<G>) {
        ctx.audio.pause_all();
        ctx.pause_event_speech();
        menu_enter(self, ctx);
    }

    /// Brings the game's sound and event speech back once the player leaves.
    fn exit(&mut self, ctx: &mut Ctx<G>) {
        ctx.audio.resume_all();
        ctx.resume_event_speech();
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn build_items(&mut self, _ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        vec![
            MenuItem::new("Resume", |_s: &mut Self, ctx: &mut Ctx<G>| ctx.pop())
                .help("Go back to where you were."),
            MenuItem::new("Settings", |_s: &mut Self, ctx: &mut Ctx<G>| {
                ctx.push(SettingsScreen::<G>::new())
            })
            .help("Speech rate and other settings."),
            MenuItem::new("Help", |s: &mut Self, ctx: &mut Ctx<G>| {
                ctx.push(HelpScreen::<G>::new(s.help_doc))
            })
            .help("How the game works."),
            MenuItem::new("Return to the title", |s: &mut Self, ctx: &mut Ctx<G>| {
                (s.to_title)(ctx)
            })
            .help("Leave the game and go back to the title."),
            MenuItem::new("Quit", |_s: &mut Self, ctx: &mut Ctx<G>| ctx.quit())
                .help("Close the game."),
        ]
    }
}
