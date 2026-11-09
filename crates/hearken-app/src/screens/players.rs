//! The other players in a room, and what can be done about each: mute,
//! block, report. Opened from the Room screen's Mute or report a player item.
//!
//! Mutes and blocks are kept on the server, per player, so they hold in
//! every room and on every copy of the game; the other player is never
//! told. The list reads them when it opens (`Step::Loading`), and each
//! change is one request (`Step::Setting`) whose answer is spoken; a
//! report is confirmed first and answered on the list.

use hearken_rooms::{Answer, Mute, MuteMode, RoomId, Ticket};

use crate::ctx::Ctx;
use crate::game::Game;
use crate::menu::{Menu, MenuCore, MenuItem};
use crate::screens::confirm::ConfirmScreen;
use crate::sound::UiSound;

/// What the screen is waiting on, if anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Idle,
    /// Reading who is muted or blocked.
    Loading(Ticket),
    /// Changing one player's standing.
    Setting(Ticket, MuteMode),
}

impl Step {
    fn ticket(self) -> Option<Ticket> {
        match self {
            Step::Idle => None,
            Step::Loading(t) | Step::Setting(t, _) => Some(t),
        }
    }
}

/// One of the other players, as the list shows them.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Other {
    name: String,
    handle: String,
}

/// The other players in a room.
pub struct PlayersScreen<G: Game> {
    room: RoomId,
    menu: MenuCore<Self, G>,
    step: Step,
    mutes: Vec<Mute>,
    loaded: bool,
    /// Announce once the list has been read again, so a changed standing
    /// is spoken as it now is, not as it was.
    announce_after_load: bool,
}

impl<G: Game> PlayersScreen<G> {
    pub fn new(room: RoomId) -> Self {
        PlayersScreen {
            room,
            menu: MenuCore::new("Players"),
            step: Step::Idle,
            mutes: Vec::new(),
            loaded: false,
            announce_after_load: false,
        }
    }

    fn others(&self, ctx: &Ctx<G>) -> Vec<Other> {
        let Some(room) = ctx.rooms.room(&self.room) else {
            return Vec::new();
        };
        room.info
            .members
            .iter()
            .filter(|m| Some(m.seat) != room.info.you)
            .filter_map(|m| {
                Some(Other {
                    name: m.name.clone(),
                    handle: m.handle.clone()?,
                })
            })
            .collect()
    }

    fn mode_of(mutes: &[Mute], handle: &str) -> MuteMode {
        mutes
            .iter()
            .find(|m| m.handle == handle)
            .map_or(MuteMode::None, |m| m.mode)
    }

    fn load(&mut self, ctx: &mut Ctx<G>) {
        if let Ok(ticket) = ctx.rooms.mutes() {
            self.step = Step::Loading(ticket);
        }
    }
}

impl<G: Game> Menu for PlayersScreen<G> {
    type Game = G;

    fn name(&self) -> &'static str {
        "players"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn enter(&mut self, ctx: &mut Ctx<G>) {
        self.load(ctx);
        crate::menu::menu_enter(self, ctx);
    }

    fn resume_with(&mut self, ctx: &mut Ctx<G>, result: &str) {
        // A player's standing changed on the screen above: read the list
        // again, and announce it once it is current.
        if !result.is_empty() {
            ctx.say(result.to_string());
        }
        self.load(ctx);
        self.refresh(ctx, true);
        if matches!(self.step, Step::Loading(_)) {
            self.announce_after_load = true;
        } else {
            self.announce(ctx);
        }
    }

    fn build_items(&mut self, ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        let others = self.others(ctx);
        if others.is_empty() {
            return vec![MenuItem::inert("No other players yet.")];
        }
        others
            .into_iter()
            .map(|other| {
                let mode = Self::mode_of(&self.mutes, &other.handle);
                let label = match mode {
                    MuteMode::None => other.name.clone(),
                    mode => format!("{}: {}", other.name, mode.spoken()),
                };
                let room = self.room.clone();
                MenuItem::new(label, move |_: &mut Self, ctx: &mut Ctx<G>| {
                    ctx.push(PlayerScreen::new(room.clone(), other.clone(), mode));
                })
                .help("Mute, block or report this player.")
            })
            .collect()
    }

    fn update(&mut self, ctx: &mut Ctx<G>, _dt: f64) {
        let Some(ticket) = self.step.ticket() else {
            return;
        };
        let Some(answer) = ctx.rooms.result(ticket) else {
            return;
        };
        self.step = Step::Idle;
        match answer {
            Ok(Answer::Mutes(mutes)) => {
                let changed = mutes != self.mutes || !self.loaded;
                self.mutes = mutes;
                self.loaded = true;
                if changed {
                    self.refresh(ctx, true);
                }
            }
            Ok(_) => {}
            Err(e) => {
                let site = ctx.online.site().to_string();
                ctx.say(e.spoken(&site));
            }
        }
        if std::mem::take(&mut self.announce_after_load) {
            ctx.queued_speech(|ctx| self.announce(ctx));
        }
    }

    fn help(&self, ctx: &Ctx<G>) -> String {
        self.current_help(ctx).unwrap_or_else(|| {
            "The other players in this game. Enter opens one to mute, block or report them. \
             Escape goes back."
                .to_string()
        })
    }
}

/// One other player: mute or block them, or hear them again, and report.
pub struct PlayerScreen<G: Game> {
    room: RoomId,
    other: Other,
    mode: MuteMode,
    menu: MenuCore<Self, G>,
    step: Step,
}

impl<G: Game> PlayerScreen<G> {
    fn new(room: RoomId, other: Other, mode: MuteMode) -> Self {
        PlayerScreen {
            menu: MenuCore::new(other.name.clone()),
            room,
            other,
            mode,
            step: Step::Idle,
        }
    }

    fn set(&mut self, ctx: &mut Ctx<G>, mode: MuteMode) {
        if self.step != Step::Idle {
            ctx.say("Still asking.");
            return;
        }
        match ctx.rooms.set_mute(&self.other.handle, mode) {
            Ok(ticket) => self.step = Step::Setting(ticket, mode),
            Err(e) => {
                ctx.play_ui(UiSound::Error);
                let site = ctx.online.site().to_string();
                ctx.say(e.spoken(&site));
            }
        }
    }
}

impl<G: Game> Menu for PlayerScreen<G> {
    type Game = G;

    fn name(&self) -> &'static str {
        "player"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn build_items(&mut self, _ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        let name = self.other.name.clone();
        let mut items = Vec::new();
        match self.mode {
            MuteMode::Mute => items.push(
                MenuItem::new("Unmute", |s: &mut Self, ctx: &mut Ctx<G>| {
                    s.set(ctx, MuteMode::None)
                })
                .help(format!("Hear what {name} says again.")),
            ),
            _ => items.push(
                MenuItem::new("Mute", |s: &mut Self, ctx: &mut Ctx<G>| {
                    s.set(ctx, MuteMode::Mute)
                })
                .help(format!(
                    "Stop hearing what {name} says, in every game. They are not told."
                )),
            ),
        }
        match self.mode {
            MuteMode::Block => items.push(
                MenuItem::new("Unblock", |s: &mut Self, ctx: &mut Ctx<G>| {
                    s.set(ctx, MuteMode::None)
                })
                .help(format!("Let {name} and you reach each other again.")),
            ),
            _ => items.push(
                MenuItem::new("Block", |s: &mut Self, ctx: &mut Ctx<G>| {
                    s.set(ctx, MuteMode::Block)
                })
                .help(format!(
                    "Neither of you hears the other, and Play with anyone never seats you \
                     with {name}. They are not told."
                )),
            ),
        }
        items.push(
            MenuItem::new("Report", |s: &mut Self, ctx: &mut Ctx<G>| {
                let site = ctx.online.site().to_string();
                let name = s.other.name.clone();
                let room = s.room.clone();
                let handle = s.other.handle.clone();
                ctx.push(ConfirmScreen::<G>::new(
                    format!(
                        "Report {name} to {site}? What they said in this game goes with the report."
                    ),
                    move |ctx| {
                        let sent = ctx.rooms.report(&room, &handle, "");
                        ctx.pop();
                        match sent {
                            Ok(_) => ctx.pop_with(format!("Reported {name}.")),
                            Err(e) => {
                                ctx.play_ui(UiSound::Error);
                                ctx.pop_with(e.spoken(&site));
                            }
                        }
                    },
                ));
            })
            .help("Send what this player said here to the people who run the server."),
        );
        items
    }

    fn update(&mut self, ctx: &mut Ctx<G>, _dt: f64) {
        let Some(ticket) = self.step.ticket() else {
            return;
        };
        let Some(answer) = ctx.rooms.result(ticket) else {
            return;
        };
        let step = self.step;
        self.step = Step::Idle;
        match (step, answer) {
            (Step::Setting(_, mode), Ok(_)) => {
                self.mode = mode;
                ctx.play_ui(UiSound::Select);
                let line = match mode {
                    MuteMode::Mute => format!("{} muted.", self.other.name),
                    MuteMode::Block => format!("{} blocked.", self.other.name),
                    MuteMode::None => format!("{} heard again.", self.other.name),
                };
                ctx.pop_with(line);
            }
            (_, Ok(_)) => {}
            (_, Err(e)) => {
                ctx.play_ui(UiSound::Error);
                let site = ctx.online.site().to_string();
                ctx.say(e.spoken(&site));
            }
        }
    }

    fn help(&self, ctx: &Ctx<G>) -> String {
        self.current_help(ctx).unwrap_or_else(|| {
            format!(
                "What to do about {}. Enter chooses, Escape goes back.",
                self.other.name
            )
        })
    }
}
