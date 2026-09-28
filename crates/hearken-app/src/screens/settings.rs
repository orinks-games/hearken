//! The engine's settings, and the way to the rebinding screens. Every
//! change is heard at once; Escape saves `settings.json` and goes back.

use hearken_audio::Bus;

use crate::bindings::Action;
use crate::ctx::Ctx;
use crate::game::Game;
use crate::menu::{Label, Menu, MenuCore, MenuItem};
use crate::screens::{BindingDevice, BindingsScreen};
use crate::sound::UiSound;

/// A speech-rate step: five percent of the voice's range.
const RATE_STEP: f64 = 0.05;
/// A volume step: five percent, the whole range a Left or Right press moves.
const VOLUME_STEP: i32 = 5;

/// The rate as the player hears it: a whole percentage.
pub fn rate_label(rate: f64) -> String {
    format!("Speech rate: {} percent", (rate * 100.0).round() as i64)
}

fn toggle_positions<G: Game>(ctx: &mut Ctx<G>) {
    let menus = &mut ctx.settings.menus;
    menus.announce_position = !menus.announce_position;
}

fn toggle_controller<G: Game>(ctx: &mut Ctx<G>) {
    let on = !ctx.settings.controller.enabled;
    ctx.settings.controller.enabled = on;
    ctx.controller.set_enabled(on);
}

fn toggle_duck<G: Game>(ctx: &mut Ctx<G>) {
    ctx.settings.audio.duck_for_speech = !ctx.settings.audio.duck_for_speech;
}

/// Steps the player's verbosity level by `delta`, wrapping; does nothing for
/// a game with no levels.
fn step_verbosity<G: Game>(ctx: &mut Ctx<G>, delta: i32) {
    let levels = ctx.verbosity().levels;
    if levels.is_empty() {
        return;
    }
    let current = levels
        .iter()
        .position(|l| l.id == ctx.settings.speech.verbosity)
        .unwrap_or(0);
    let n = levels.len() as i32;
    let next = (current as i32 + delta).rem_euclid(n) as usize;
    ctx.settings.speech.verbosity = levels[next].id.to_string();
}

/// Steps `bus`'s volume by `delta * 5` percent (clamped to `0..=100`), saves
/// it and applies it to the mix at once.
fn step_volume<G: Game>(ctx: &mut Ctx<G>, bus: Bus, delta: i32) {
    let current = ctx.volume_percent(bus) as i32;
    let next = (current + delta * VOLUME_STEP).clamp(0, 100) as u8;
    ctx.settings.audio.set_volume(bus, next);
    ctx.apply_audio_settings();
}

/// Plays the Move earcon on `bus`, so a volume change is heard through the
/// bus it changed; on the UI bus for the master volume, since nothing plays
/// "on" the master bus itself.
fn play_move_on<G: Game>(ctx: &mut Ctx<G>, bus: Bus) {
    if let Some(key) = ctx.ui_sounds.get(UiSound::Move) {
        let target = if bus == Bus::MASTER { Bus::UI } else { bus };
        ctx.audio.play(key, target);
    }
}

/// One row of the Settings screen, tagged with a stable id so a game's
/// [`Game::settings_rows`] can find, reorder, drop or add to the built-in
/// rows.
pub struct SettingsRow<G: Game> {
    pub id: String,
    pub item: MenuItem<SettingsScreen<G>, G>,
}

impl<G: Game> SettingsRow<G> {
    pub fn new(id: impl Into<String>, item: MenuItem<SettingsScreen<G>, G>) -> Self {
        SettingsRow {
            id: id.into(),
            item,
        }
    }
}

pub struct SettingsScreen<G: Game> {
    menu: MenuCore<Self, G>,
    /// The id of each row in `menu.items`, in the same order, read back by
    /// `speak_adjusted` to find the speech-rate row without assuming it is
    /// first: a game's `settings_rows` may have moved it.
    row_ids: Vec<String>,
}

impl<G: Game> SettingsScreen<G> {
    pub fn new() -> Self {
        SettingsScreen {
            menu: MenuCore::new("Settings").with_intro_help(
                "Up and Down move, Left and Right change the setting, Escape saves and goes back.",
            ),
            row_ids: Vec::new(),
        }
    }
}

impl<G: Game> Default for SettingsScreen<G> {
    fn default() -> Self {
        Self::new()
    }
}

impl<G: Game> Menu for SettingsScreen<G> {
    type Game = G;

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
        let mut rows = Vec::new();

        if ctx.speech.supports_rate() {
            rows.push(SettingsRow::new(
                "speech_rate",
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
            ));
        }

        if !ctx.verbosity().is_empty() {
            rows.push(SettingsRow::new(
                "verbosity",
                MenuItem::new(
                    Label::dynamic(|_s: &Self, ctx: &Ctx<G>| {
                        let label = ctx.verbosity_level().map(|l| l.label).unwrap_or("");
                        format!("Speech detail: {label}")
                    }),
                    |s: &mut Self, ctx: &mut Ctx<G>| {
                        step_verbosity(ctx, 1);
                        // The rows are lent to this action while it runs; rebuild
                        // them so the read-back finds this row.
                        Menu::refresh(s, ctx, true);
                        s.speak_current(ctx);
                    },
                )
                .help("How much the game says while you play. Left and Right change it.")
                .adjustable(|_s: &mut Self, ctx: &mut Ctx<G>, delta| step_verbosity(ctx, delta)),
            ));
        }

        rows.push(SettingsRow::new(
            "duck",
            MenuItem::new(
                Label::dynamic(|_s: &Self, ctx: &Ctx<G>| {
                    let on = ctx.settings.audio.duck_for_speech;
                    format!(
                        "Lower game sound under speech: {}",
                        if on { "on" } else { "off" }
                    )
                }),
                |s: &mut Self, ctx: &mut Ctx<G>| {
                    toggle_duck(ctx);
                    // The rows are lent to this action while it runs; rebuild
                    // them so the read-back finds this row.
                    Menu::refresh(s, ctx, true);
                    s.speak_current(ctx);
                },
            )
            .help(
                "Whether music and background sound get quieter while the game speaks. \
                 Enter, Left or Right changes it.",
            )
            .adjustable(|_s: &mut Self, ctx: &mut Ctx<G>, _delta| toggle_duck(ctx)),
        ));

        for (bus, label) in ctx.buses() {
            let help = if bus == Bus::MASTER {
                "Left and Right change how loud the whole game is.".to_string()
            } else {
                format!(
                    "Left and Right change the {} volume.",
                    label.to_lowercase()
                )
            };
            rows.push(SettingsRow::new(
                format!("volume.{}", bus.name()),
                MenuItem::inert(Label::dynamic(move |_s: &Self, ctx: &Ctx<G>| {
                    format!("{label} volume: {} percent", ctx.volume_percent(bus))
                }))
                .help(help)
                .adjustable(move |_s: &mut Self, ctx: &mut Ctx<G>, delta| {
                    step_volume(ctx, bus, delta);
                    play_move_on(ctx, bus);
                })
                .silent_adjust(),
            ));
        }

        rows.push(SettingsRow::new(
            "menu_positions",
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
        ));
        let actions = <G::Action as Action>::all();
        if actions
            .iter()
            .any(|action| !action.default_keys().is_empty())
        {
            rows.push(SettingsRow::new(
                "keyboard",
                MenuItem::new("Keyboard shortcuts", |_s: &mut Self, ctx: &mut Ctx<G>| {
                    ctx.push(BindingsScreen::<G>::new(BindingDevice::Keyboard))
                })
                .help("Move any control to another key."),
            ));
        }
        if actions
            .iter()
            .any(|action| !action.default_pad().is_empty())
        {
            rows.push(SettingsRow::new(
                "controller_buttons",
                MenuItem::new("Controller buttons", |_s: &mut Self, ctx: &mut Ctx<G>| {
                    ctx.push(BindingsScreen::<G>::new(BindingDevice::Controller))
                })
                .help("Move any control to another controller button."),
            ));
        }
        rows.push(SettingsRow::new(
            "controller",
            MenuItem::new(
                Label::dynamic(|_s: &Self, ctx: &Ctx<G>| {
                    let on = ctx.settings.controller.enabled;
                    format!("Controller: {}", if on { "on" } else { "off" })
                }),
                |s: &mut Self, ctx: &mut Ctx<G>| {
                    toggle_controller(ctx);
                    // The rows are lent to this action while it runs; rebuild
                    // them so the read-back finds this row.
                    Menu::refresh(s, ctx, true);
                    s.speak_current(ctx);
                },
            )
            .help("Whether the game listens to a game controller. Enter, Left or Right changes it.")
            .adjustable(|_s: &mut Self, ctx: &mut Ctx<G>, _delta| toggle_controller(ctx)),
        ));

        let rows = G::settings_rows(&*ctx, rows);
        self.row_ids = rows.iter().map(|r| r.id.clone()).collect();
        rows.into_iter().map(|r| r.item).collect()
    }

    /// A new rate is read back by the voice whose rate changed.
    fn speak_adjusted(&mut self, ctx: &mut Ctx<G>) {
        if self.row_ids.get(self.menu.index).map(String::as_str) == Some("speech_rate") {
            let text = self.current_text(ctx);
            ctx.say_preview("speech_rate", text);
        } else {
            self.speak_current(ctx);
        }
    }

    fn on_escape(&mut self, ctx: &mut Ctx<G>) {
        ctx.save_settings();
        ctx.play_ui(UiSound::Back);
        ctx.pop();
    }
}
