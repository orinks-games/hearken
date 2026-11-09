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
/// The built-in voice's speech-rate step: one percent.
const BUILTIN_RATE_STEP: f64 = 0.01;
/// A volume step: five percent, the whole range a Left or Right press moves.
const VOLUME_STEP: i32 = 5;

/// The rate as the player hears it: a whole percentage.
pub fn rate_label(rate: f64) -> String {
    format!("Speech rate: {} percent", (rate * 100.0).round() as i64)
}

fn step_language<G: Game>(ctx: &mut Ctx<G>, delta: i32) {
    let options = crate::language::offered_settings::<G>(&ctx.paths.data_dir);
    if options.is_empty() {
        return;
    }
    let current = &ctx.settings.language;
    let at = options.iter().position(|c| c == current).unwrap_or(0);
    let next = (at as i32 + delta).rem_euclid(options.len() as i32) as usize;
    let code = options[next].clone();
    ctx.set_language(&code);
}

fn language_value<G: Game>(ctx: &Ctx<G>) -> String {
    crate::language::language_label(
        &ctx.settings.language,
        hearken_l10n::system_locale().as_deref(),
    )
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

fn toggle_discord<G: Game>(ctx: &mut Ctx<G>) {
    let on = !ctx.settings.discord.enabled;
    ctx.set_discord_status(on);
}

/// Steps the update channel through the game's list, wrapping. The update
/// the check at start found was for the old channel, so it is forgotten.
#[cfg(feature = "update")]
fn step_channel<G: Game>(ctx: &mut Ctx<G>, delta: i32) {
    let channels = ctx.updates.channels();
    let current = ctx.updates.channel(&ctx.settings.updates);
    let at = channels.iter().position(|c| *c == current).unwrap_or(0);
    let next = (at as i32 + delta).rem_euclid(channels.len() as i32) as usize;
    ctx.settings.updates.channel = Some(channels[next].clone());
    ctx.updates.forget();
}

#[cfg(feature = "update")]
fn toggle_update_check<G: Game>(ctx: &mut Ctx<G>) {
    let updates = &mut ctx.settings.updates;
    updates.check_at_start = !updates.check_at_start;
}

/// Whether the built-in voice is speaking or chosen.
fn builtin_in_use<G: Game>(ctx: &Ctx<G>) -> bool {
    ctx.settings.speech.self_voicing || ctx.speech.backend_name() == hearken_speech::BUILTIN_BACKEND
}

/// "automatic", or the built-in voice the player chose; "automatic, US
/// female" when automatic fell back to the built-in voice.
fn speech_voice_value<G: Game>(ctx: &Ctx<G>) -> String {
    let speech = &ctx.settings.speech;
    if speech.self_voicing {
        speech.builtin_voice.clone()
    } else if ctx.speech.backend_name() == hearken_speech::BUILTIN_BACKEND {
        format!("automatic, {}", speech.builtin_voice)
    } else {
        "automatic".to_string()
    }
}

/// Steps Speech voice through automatic and each built-in voice, wrapping.
fn step_speech_voice<G: Game>(ctx: &mut Ctx<G>, delta: i32) {
    let voices = hearken_speech::builtin_voices();
    if voices.is_empty() {
        return;
    }
    let speech = &mut ctx.settings.speech;
    let current = if speech.self_voicing {
        voices
            .iter()
            .position(|v| *v == speech.builtin_voice)
            .map_or(1, |i| i + 1)
    } else {
        0
    };
    let n = voices.len() as i32 + 1;
    let next = (current as i32 + delta).rem_euclid(n) as usize;
    if next == 0 {
        speech.self_voicing = false;
    } else {
        speech.self_voicing = true;
        speech.builtin_voice = voices[next - 1].to_string();
    }
    ctx.apply_speech_settings();
}

fn step_speech_volume<G: Game>(ctx: &mut Ctx<G>, delta: i32) {
    let next = ctx.settings.speech.volume + f64::from(delta) * f64::from(VOLUME_STEP) / 100.0;
    ctx.settings.speech.volume = ((next * 100.0).round() / 100.0).clamp(0.05, 1.0);
    ctx.apply_speech_settings();
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

/// The body every toggle row on this screen shares: run the toggle (`f`),
/// then rebuild the rows and read the current one back. The rows are lent to
/// the row's action while it runs, so they must be rebuilt for the read-back
/// to find this row.
fn toggle_row<G: Game>(s: &mut SettingsScreen<G>, ctx: &mut Ctx<G>, f: impl FnOnce(&mut Ctx<G>)) {
    f(ctx);
    Menu::refresh(s, ctx, true);
    s.speak_current(ctx);
}

/// The built-in rows' categories, used when [`Game::settings_categories`]
/// is on.
pub mod categories {
    pub const SPEECH: &str = "Speech";
    pub const SOUND: &str = "Sound";
    pub const DISPLAY: &str = "Display";
    pub const CONTROLS: &str = "Controls";
    pub const ONLINE: &str = "Online";
    pub const UPDATES: &str = "Updates";
}

/// The category of a built-in row, by its id.
fn builtin_category(id: &str) -> Option<&'static str> {
    use categories::*;
    Some(match id {
        "language" | "speech_rate" | "speech_voice" | "speech_volume" | "verbosity"
        | "menu_positions" => SPEECH,
        "duck" => SOUND,
        _ if id.starts_with("volume.") => SOUND,
        "display" | "display_text" | "display_motion" | "display_contrast" => DISPLAY,
        "keyboard" | "controller_buttons" | "controller" => CONTROLS,
        "discord" | "online" | "chat" => ONLINE,
        "updates" | "update_channel" | "update_at_start" => UPDATES,
        _ => return None,
    })
}

/// One row of the Settings screen, tagged with a stable id so a game's
/// [`Game::settings_rows`] can find, reorder, drop or add to the built-in
/// rows.
pub struct SettingsRow<G: Game> {
    pub id: String,
    pub item: MenuItem<SettingsScreen<G>, G>,
    /// The category this row is listed under when the game turns on
    /// [`Game::settings_categories`]. A row with none stays on the first
    /// Settings screen, between the categories.
    pub category: Option<String>,
}

impl<G: Game> SettingsRow<G> {
    pub fn new(id: impl Into<String>, item: MenuItem<SettingsScreen<G>, G>) -> Self {
        SettingsRow {
            id: id.into(),
            item,
            category: None,
        }
    }

    /// Lists the row under `category`, such as `"Sound"` (see
    /// [`Game::settings_categories`]).
    pub fn in_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// Left and Right read back `value` alone (computed after the change,
    /// such as `"55 percent"` or `"Hard, difficulty"`) instead of the whole
    /// row and its position. Moving onto the row still reads it all. The
    /// speech-rate row's read-back still goes through the voice it changed.
    pub fn value_first(mut self, value: impl Fn(&Ctx<G>) -> String + 'static) -> Self {
        self.item = self.item.adjusted_text(Label::dynamic(
            move |_s: &SettingsScreen<G>, ctx: &Ctx<G>| value(ctx),
        ));
        self
    }
}

pub struct SettingsScreen<G: Game> {
    menu: MenuCore<Self, G>,
    /// The one category this screen lists, or `None` for the first
    /// Settings screen (see [`Game::settings_categories`]).
    category: Option<String>,
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
            category: None,
            row_ids: Vec::new(),
        }
    }

    /// One category's rows, titled with its name, such as `"Sound"`.
    /// Settings opens these itself when [`Game::settings_categories`] is on.
    pub fn category(name: impl Into<String>) -> Self {
        let name = name.into();
        SettingsScreen {
            menu: MenuCore::new(name.clone()).with_intro_help(
                "Up and Down move, Left and Right change the setting, Escape goes back.",
            ),
            category: Some(name),
            row_ids: Vec::new(),
        }
    }

    /// The category this screen lists, if it lists one.
    pub fn category_name(&self) -> Option<&str> {
        self.category.as_deref()
    }

    /// Sorts the rows for this screen when the game uses categories. A
    /// category screen keeps its own rows. The first screen keeps rows with
    /// no category where they are and puts one row in place of each
    /// category, where its first row was.
    fn sorted(&self, rows: Vec<SettingsRow<G>>) -> Vec<SettingsRow<G>> {
        if let Some(name) = &self.category {
            return rows
                .into_iter()
                .filter(|r| r.category.as_ref() == Some(name))
                .collect();
        }
        let mut out: Vec<SettingsRow<G>> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for row in rows {
            let Some(name) = row.category else {
                out.push(row);
                continue;
            };
            if seen.contains(&name) {
                continue;
            }
            seen.push(name.clone());
            let open = name.clone();
            out.push(SettingsRow::new(
                format!("category.{name}"),
                MenuItem::new(name.clone(), move |_s: &mut Self, ctx: &mut Ctx<G>| {
                    ctx.push(SettingsScreen::<G>::category(open.clone()))
                })
                .help(format!("{name} settings.")),
            ));
        }
        out
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

        if crate::language::has_catalogs::<G>(&ctx.paths.data_dir) {
            rows.push(
                SettingsRow::new(
                    "language",
                    MenuItem::inert(Label::dynamic(|_s: &Self, ctx: &Ctx<G>| {
                        format!("Language: {}", language_value(ctx))
                    }))
                    .help(
                        "Automatic follows your system language. Left and Right choose a language. \
                         Lines with no translation stay in English.",
                    )
                    .adjustable(|_s: &mut Self, ctx: &mut Ctx<G>, delta| step_language(ctx, delta)),
                )
                .value_first(language_value),
            );
        }

        if ctx.speech.supports_rate() {
            rows.push(SettingsRow::new(
                "speech_rate",
                MenuItem::inert(Label::dynamic(|_s: &Self, ctx: &Ctx<G>| {
                    rate_label(ctx.settings.speech.rate)
                }))
                .help("Left and Right change how fast the game speaks.")
                .adjustable(|_s: &mut Self, ctx: &mut Ctx<G>, delta| {
                    // The built-in voice takes fine steps: its rate is
                    // the player's to tune, where a screen reader's is not.
                    let step = if builtin_in_use(ctx) {
                        BUILTIN_RATE_STEP
                    } else {
                        RATE_STEP
                    };
                    let next = ctx.settings.speech.rate + f64::from(delta) * step;
                    // Rounded to whole percent so repeated steps never drift.
                    ctx.settings.speech.rate = ((next * 100.0).round() / 100.0).clamp(0.0, 1.0);
                    ctx.apply_speech_settings();
                }),
            ));
        }

        if !hearken_speech::builtin_voices().is_empty() {
            rows.push(
                SettingsRow::new(
                    "speech_voice",
                    MenuItem::new(
                        Label::dynamic(|_s: &Self, ctx: &Ctx<G>| {
                            format!("Speech voice: {}", speech_voice_value(ctx))
                        }),
                        |s: &mut Self, ctx: &mut Ctx<G>| {
                            toggle_row(s, ctx, |ctx| step_speech_voice(ctx, 1))
                        },
                    )
                    .help(
                        "Automatic uses a running screen reader or an installed system voice when it can. Otherwise, it uses the built-in voice. \
                         Left and Right choose automatic or a built-in voice. \
                         Press Control, Shift, and V together on any screen to switch between your chosen built-in voice and automatic. The game saves the choice.",
                    )
                    .adjustable(|s: &mut Self, ctx: &mut Ctx<G>, delta| {
                        step_speech_voice(ctx, delta);
                        // The volume row comes and goes with the built-in voice.
                        Menu::refresh(s, ctx, true);
                    }),
                )
                .value_first(speech_voice_value),
            );
        }

        if builtin_in_use(ctx) {
            rows.push(
                SettingsRow::new(
                    "speech_volume",
                    MenuItem::inert(Label::dynamic(|_s: &Self, ctx: &Ctx<G>| {
                        format!(
                            "Speech volume: {} percent",
                            (ctx.settings.speech.volume * 100.0).round() as i64
                        )
                    }))
                    .help("Left and Right change how loud the built-in voice is.")
                    .adjustable(|_s: &mut Self, ctx: &mut Ctx<G>, delta| {
                        step_speech_volume(ctx, delta)
                    }),
                )
                .value_first(|ctx: &Ctx<G>| {
                    format!(
                        "{} percent",
                        (ctx.settings.speech.volume * 100.0).round() as i64
                    )
                }),
            );
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
                        toggle_row(s, ctx, |ctx| step_verbosity(ctx, 1))
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
                |s: &mut Self, ctx: &mut Ctx<G>| toggle_row(s, ctx, toggle_duck),
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
                format!("Left and Right change the {} volume.", label.to_lowercase())
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
                |s: &mut Self, ctx: &mut Ctx<G>| toggle_row(s, ctx, toggle_positions),
            )
            .help(
                "Whether menus say where you are, such as 2 of 4. Enter, Left or Right changes it.",
            )
            .adjustable(|_s: &mut Self, ctx: &mut Ctx<G>, _delta| toggle_positions(ctx)),
        ));
        #[cfg(feature = "visuals")]
        {
            rows.push(SettingsRow::new(
                "display",
                MenuItem::new(
                    Label::dynamic(|_s: &Self, ctx: &Ctx<G>| {
                        format!(
                            "Visual display: {}",
                            if ctx.settings.display.enabled {
                                "on"
                            } else {
                                "off"
                            }
                        )
                    }),
                    |s: &mut Self, ctx: &mut Ctx<G>| {
                        toggle_row(s, ctx, |ctx| {
                            ctx.settings.display.enabled = !ctx.settings.display.enabled
                        })
                    },
                )
                .help("Show the game, menus and captions. Speech and controls stay the same.")
                .adjustable(|_s: &mut Self, ctx: &mut Ctx<G>, _| {
                    ctx.settings.display.enabled = !ctx.settings.display.enabled
                }),
            ));
            rows.push(SettingsRow::new(
                "display_text",
                MenuItem::inert(Label::dynamic(|_s: &Self, ctx: &Ctx<G>| {
                    format!(
                        "Display text: {} percent",
                        ctx.settings.display.text_percent.clamp(100, 200)
                    )
                }))
                .help("Left and Right change the size of menu text and captions.")
                .adjustable(|_s: &mut Self, ctx: &mut Ctx<G>, delta| {
                    ctx.settings.display.text_percent =
                        (i32::from(ctx.settings.display.text_percent.clamp(100, 200)) + delta * 25)
                            .clamp(100, 200) as u16;
                }),
            ));
        }
        #[cfg(feature = "visuals")]
        {
            rows.push(SettingsRow::new(
                "display_motion",
                MenuItem::new(
                    Label::dynamic(|_s: &Self, ctx: &Ctx<G>| {
                        format!(
                            "Decorative motion: {}",
                            if ctx.settings.display.reduced_motion {
                                "off"
                            } else {
                                "on"
                            }
                        )
                    }),
                    |s: &mut Self, ctx: &mut Ctx<G>| {
                        toggle_row(s, ctx, |ctx| {
                            ctx.settings.display.reduced_motion =
                                !ctx.settings.display.reduced_motion
                        })
                    },
                )
                .help("Stop decorative motion. Game positions and timing cues still update.")
                .adjustable(|_s: &mut Self, ctx: &mut Ctx<G>, _| {
                    ctx.settings.display.reduced_motion = !ctx.settings.display.reduced_motion
                }),
            ));
            rows.push(SettingsRow::new(
                "display_contrast",
                MenuItem::new(
                    Label::dynamic(|_s: &Self, ctx: &Ctx<G>| {
                        format!(
                            "High contrast text: {}",
                            if ctx.settings.display.high_contrast {
                                "on"
                            } else {
                                "off"
                            }
                        )
                    }),
                    |s: &mut Self, ctx: &mut Ctx<G>| {
                        toggle_row(s, ctx, |ctx| {
                            ctx.settings.display.high_contrast = !ctx.settings.display.high_contrast
                        })
                    },
                )
                .help("Use white text on black with yellow keyboard focus.")
                .adjustable(|_s: &mut Self, ctx: &mut Ctx<G>, _| {
                    ctx.settings.display.high_contrast = !ctx.settings.display.high_contrast
                }),
            ));
        }
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
                |s: &mut Self, ctx: &mut Ctx<G>| toggle_row(s, ctx, toggle_controller),
            )
            .help("Whether the game listens to a game controller. Enter, Left or Right changes it.")
            .adjustable(|_s: &mut Self, ctx: &mut Ctx<G>, _delta| toggle_controller(ctx)),
        ));

        if ctx.discord.is_offered() {
            rows.push(SettingsRow::new(
                "discord",
                MenuItem::new(
                    Label::dynamic(|_s: &Self, ctx: &Ctx<G>| {
                        let on = ctx.settings.discord.enabled;
                        format!("Discord status: {}", if on { "on" } else { "off" })
                    }),
                    |s: &mut Self, ctx: &mut Ctx<G>| toggle_row(s, ctx, toggle_discord),
                )
                .help(
                    "Whether Discord shows your friends what you are doing in the game. \
                     Enter, Left or Right changes it.",
                )
                .adjustable(|_s: &mut Self, ctx: &mut Ctx<G>, _delta| toggle_discord(ctx)),
            ));
        }

        #[cfg(feature = "online")]
        if ctx.online.is_offered() {
            rows.push(SettingsRow::new(
                "online",
                MenuItem::new(
                    Label::dynamic(|_s: &Self, ctx: &Ctx<G>| {
                        match ctx
                            .online
                            .player_name()
                            .filter(|_| ctx.online.is_connected())
                        {
                            Some(name) if ctx.online.is_guest() => {
                                format!("Online account: guest {name}")
                            }
                            Some(name) => format!("Online account: {name}"),
                            None => "Online account: not connected".to_string(),
                        }
                    }),
                    |_s: &mut Self, ctx: &mut Ctx<G>| {
                        ctx.push(crate::screens::OnlineAccountScreen::new())
                    },
                )
                .help(Label::dynamic(|_s: &Self, ctx: &Ctx<G>| {
                    format!(
                        "{} an account on {} to send your runs there, or disconnect it.",
                        if ctx.online.offers_guests() {
                            "Play as a guest or connect"
                        } else {
                            "Connect"
                        },
                        ctx.online.site()
                    )
                })),
            ));
        }

        #[cfg(feature = "online")]
        if ctx.rooms.is_offered() && G::chat().is_some() {
            fn step_chat<G: Game>(ctx: &mut Ctx<G>, delta: i32) {
                ctx.settings.online.chat = ctx.settings.online.chat.stepped(delta);
                ctx.save_settings();
            }
            rows.push(SettingsRow::new(
                "chat",
                MenuItem::new(
                    Label::dynamic(|_s: &Self, ctx: &Ctx<G>| {
                        format!("Chat: {}", ctx.settings.online.chat.spoken())
                    }),
                    |s: &mut Self, ctx: &mut Ctx<G>| toggle_row(s, ctx, |ctx| step_chat(ctx, 1)),
                )
                .help(
                    "How other players' lines reach you in a game: spoken as they come, a sound for \
                     each, or nothing. Reading the chat back works either way. Enter, Left or Right \
                     changes it.",
                )
                .adjustable(|_s: &mut Self, ctx: &mut Ctx<G>, delta| step_chat(ctx, delta)),
            ));
        }

        #[cfg(feature = "update")]
        if ctx.updates.is_offered() {
            rows.push(SettingsRow::new(
                "updates",
                MenuItem::new(
                    Label::dynamic(|_s: &Self, ctx: &Ctx<G>| match ctx.updates.available() {
                        Some(update) => format!(
                            "Install version {}",
                            crate::updates::spoken_version(update.version())
                        ),
                        None => "Check for updates".to_string(),
                    }),
                    |_s: &mut Self, ctx: &mut Ctx<G>| match ctx.updates.available().cloned() {
                        // The check at start does not look for the files
                        // this copy already has. With a file index, find
                        // them, without asking for the update again, so the
                        // offer and the download take only the files that
                        // changed.
                        Some(update)
                            if update.manifest.index.is_none() || update.plan.is_some() =>
                        {
                            ctx.push(crate::screens::UpdatePromptScreen::<G>::new(update))
                        }
                        Some(update) => ctx.push(crate::screens::UpdateCheckScreen::found(update)),
                        None => ctx.push(crate::screens::UpdateCheckScreen::new()),
                    },
                )
                .help("Looks for a newer version of the game, and installs it if you choose."),
            ));
            if ctx.updates.channels().len() > 1 {
                rows.push(SettingsRow::new(
                    "update_channel",
                    MenuItem::new(
                        Label::dynamic(|_s: &Self, ctx: &Ctx<G>| {
                            format!(
                                "Update channel: {}",
                                ctx.updates.channel(&ctx.settings.updates)
                            )
                        }),
                        |s: &mut Self, ctx: &mut Ctx<G>| {
                            toggle_row(s, ctx, |ctx| step_channel(ctx, 1))
                        },
                    )
                    .help("Which releases this copy follows. Left and Right change it.")
                    .adjustable(|_s: &mut Self, ctx: &mut Ctx<G>, delta| step_channel(ctx, delta)),
                ));
            }
            if ctx.updates.config().is_some_and(|c| c.check_at_start) {
                rows.push(SettingsRow::new(
                    "update_at_start",
                    MenuItem::new(
                        Label::dynamic(|_s: &Self, ctx: &Ctx<G>| {
                            let on = ctx.settings.updates.check_at_start;
                            format!(
                                "Check for updates at start: {}",
                                if on { "on" } else { "off" }
                            )
                        }),
                        |s: &mut Self, ctx: &mut Ctx<G>| toggle_row(s, ctx, toggle_update_check),
                    )
                    .help(
                        "Whether the game looks for a newer version each time it starts. \
                         Enter, Left or Right changes it.",
                    )
                    .adjustable(|_s: &mut Self, ctx: &mut Ctx<G>, _delta| toggle_update_check(ctx)),
                ));
            }
        }

        let rows: Vec<SettingsRow<G>> = rows
            .into_iter()
            .map(|row| match builtin_category(&row.id) {
                Some(c) => row.in_category(c),
                None => row,
            })
            .collect();
        let mut rows = G::settings_rows(&*ctx, rows);
        if G::settings_categories() {
            rows = self.sorted(rows);
        }
        self.row_ids = rows.iter().map(|r| r.id.clone()).collect();
        rows.into_iter().map(|r| r.item).collect()
    }

    /// A new rate is read back by the voice whose rate changed.
    fn speak_adjusted(&mut self, ctx: &mut Ctx<G>) {
        let text = self.adjusted_text(ctx);
        if self.row_ids.get(self.menu.index).map(String::as_str) == Some("speech_rate") {
            ctx.say_preview("speech_rate", text);
        } else {
            ctx.say(text);
        }
    }

    fn on_escape(&mut self, ctx: &mut Ctx<G>) {
        ctx.save_settings();
        ctx.play_ui(UiSound::Back);
        ctx.pop();
    }
}
