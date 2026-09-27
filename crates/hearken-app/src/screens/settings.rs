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

fn toggle_positions<G: Game>(ctx: &mut Ctx<G>) {
    let menus = &mut ctx.settings.menus;
    menus.announce_position = !menus.announce_position;
}

pub struct SettingsScreen<G: Game> {
    menu: MenuCore<Self, G>,
    /// Whether the first row is the speech rate. It is built only when a
    /// voice can change rate: without one every step would sound the same.
    rate_row: bool,
}

impl<G: Game> SettingsScreen<G> {
    pub fn new() -> Self {
        SettingsScreen {
            menu: MenuCore::new("Settings").with_intro_help(
                "Up and Down move, Left and Right change the setting, Escape saves and goes back.",
            ),
            rate_row: false,
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

    fn build_items(&mut self, ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        let mut items = Vec::new();
        self.rate_row = ctx.speech.supports_rate();
        if self.rate_row {
            items.push(
                MenuItem::inert(Label::dynamic(|_s: &Self, ctx: &Ctx<G>| {
                    rate_label(ctx.settings.speech.rate)
                }))
                .help("Left and Right change how fast the game speaks.")
                .adjustable(|_s: &mut Self, ctx: &mut Ctx<G>, delta| {
                    let next = ctx.settings.speech.rate + f64::from(delta) * RATE_STEP;
                    // Rounded to whole percent so repeated steps never drift.
                    ctx.settings.speech.rate = ((next * 100.0).round() / 100.0).clamp(0.0, 1.0);
                    ctx.apply_speech_settings();
                }),
            );
        }
        items.push(
            MenuItem::new(
                Label::dynamic(|_s: &Self, ctx: &Ctx<G>| {
                    let on = ctx.settings.menus.announce_position;
                    format!("Menu positions: {}", if on { "on" } else { "off" })
                }),
                |s: &mut Self, ctx: &mut Ctx<G>| {
                    toggle_positions(ctx);
                    // The rows are lent to this action while it runs; rebuild
                    // them so the read-back finds this row.
                    Menu::refresh(s, ctx, true);
                    s.speak_current(ctx);
                },
            )
            .help(
                "Whether menus say where you are, such as 2 of 4. Enter, Left or Right changes it.",
            )
            .adjustable(|_s: &mut Self, ctx: &mut Ctx<G>, _delta| toggle_positions(ctx)),
        );
        items
    }

    /// A new rate is read back by the voice whose rate changed.
    fn speak_adjusted(&mut self, ctx: &mut Ctx<G>) {
        if self.rate_row && self.menu.index == 0 {
            let text = self.current_text(ctx);
            ctx.say_preview("speech_rate", text);
        } else {
            self.speak_current(ctx);
        }
    }

    fn on_escape(&mut self, ctx: &mut Ctx<G>) {
        ctx.save_settings();
        ctx.pop();
    }
}
