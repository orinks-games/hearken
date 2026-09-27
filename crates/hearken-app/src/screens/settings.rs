//! The engine's settings. Every change is heard at once; Escape saves
//! `settings.json` and goes back.

use crate::ctx::Ctx;
use crate::game::Game;
use crate::menu::{Label, Menu, MenuCore, MenuItem};

/// A speech-rate step: five percent of the voice's range.
const RATE_STEP: f64 = 0.05;

/// The rate as the player hears it: a whole percentage.
pub fn rate_label(rate: f64) -> String {
    format!("Speech rate: {} percent", (rate * 100.0).round() as i64)
}

pub struct SettingsScreen<G: Game> {
    menu: MenuCore<Self, G>,
}

impl<G: Game> SettingsScreen<G> {
    pub fn new() -> Self {
        SettingsScreen {
            menu: MenuCore::new("Settings").with_intro_help(
                "Up and Down move, Left and Right change the setting, Escape saves and goes back.",
            ),
        }
    }
}

impl<G: Game> Default for SettingsScreen<G> {
    fn default() -> Self {
        Self::new()
    }
}

impl<G: Game> Menu<G> for SettingsScreen<G> {
    fn name(&self) -> &'static str {
        "settings"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn build_items(&mut self, _ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        vec![MenuItem::inert(Label::dynamic(|_s: &Self, ctx: &Ctx<G>| {
            rate_label(ctx.settings.speech.rate)
        }))
        .help("Left and Right change how fast the game speaks.")
        .adjustable(|_s: &mut Self, ctx: &mut Ctx<G>, delta| {
            let next = ctx.settings.speech.rate + f64::from(delta) * RATE_STEP;
            // Rounded to whole percent so repeated steps never drift.
            ctx.settings.speech.rate = ((next * 100.0).round() / 100.0).clamp(0.0, 1.0);
            ctx.apply_speech_settings();
        })]
    }

    fn on_escape(&mut self, ctx: &mut Ctx<G>) {
        if let Err(e) = ctx.settings.save(&ctx.paths) {
            log::error!("could not save settings: {e}");
        }
        ctx.pop();
    }
}
