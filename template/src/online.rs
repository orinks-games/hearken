//! Pebbles online: the template's two-player match, played through
//! `ctx.rooms`. The engine's screens make and fill the room; this screen
//! plays the match once it starts, with the rules from
//! `hearken::replay::pebbles`.
//!
//! A pile of pebbles sits between two seats. On your move, 1, 2 or 3 takes
//! that many; whoever takes the last pebble loses. The server orders the
//! turns, so the other player's move arrives as an event and is applied
//! here, the same way our own does.

use hearken::replay::pebbles::{Pebbles, Setup, Take};
use hearken::rooms::{Match, RoomId, Sync};
use hearken::screens::{OnlinePlayScreen, RoomKind};
use hearken::{Ctx, InputEvent, Key, State, UiSound};

use crate::HelloGame;

/// The Online play menu for Pebbles, for the title to push.
pub fn online_play() -> OnlinePlayScreen<HelloGame> {
    OnlinePlayScreen::new(RoomKind::new(
        "pebbles",
        "Pebbles",
        &Setup { pile: 10 },
        |ctx, id| ctx.push(PebblesMatch::new(id.clone())),
    ))
}

/// A match in play, or over.
pub struct PebblesMatch {
    id: RoomId,
    game: Option<Match<Pebbles>>,
    /// Whether the end of the match has been spoken.
    ended: bool,
}

impl PebblesMatch {
    pub fn new(id: RoomId) -> Self {
        PebblesMatch {
            id,
            game: None,
            ended: false,
        }
    }

    /// The other player's name.
    fn other(ctx: &Ctx<HelloGame>, id: &RoomId) -> String {
        ctx.rooms
            .room(id)
            .map(|r| r.info.others().join(" and "))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "the other player".to_string())
    }

    /// Where the match stands: the pile and whose move.
    fn standing(&self, ctx: &Ctx<HelloGame>) -> String {
        let Some(game) = &self.game else {
            return "The match has not started.".to_string();
        };
        let sim = game.sim();
        let you = ctx.rooms.room(&self.id).and_then(|r| r.info.you());
        if game.finished() {
            let standing = game.recording().outcome();
            return match (standing.winner, you.map(|s| s.number())) {
                (Some(w), Some(me)) if w == me => "You won.".to_string(),
                (Some(_), Some(_)) => format!("{} won.", Self::other(ctx, &self.id)),
                _ => "The match is over.".to_string(),
            };
        }
        let pile = sim.pile();
        let pebbles = if pile == 1 { "pebble" } else { "pebbles" };
        if Some(sim.turn()) == you {
            format!("{pile} {pebbles}. Your move: 1, 2 or 3.")
        } else {
            format!("{pile} {pebbles}. {}'s move.", Self::other(ctx, &self.id))
        }
    }

    fn take(&mut self, ctx: &mut Ctx<HelloGame>, n: u8) {
        let Some(game) = &mut self.game else {
            return;
        };
        if game.finished() {
            return;
        }
        let you = ctx.rooms.room(&self.id).and_then(|r| r.info.you());
        if Some(game.sim().turn()) != you {
            ctx.play_ui(UiSound::Error);
            ctx.say(format!("Not your move. {}", self.standing(ctx)));
            return;
        }
        if u32::from(n) > game.sim().pile() {
            ctx.play_ui(UiSound::Error);
            ctx.say(format!("Only {} left.", game.sim().pile()));
            return;
        }
        match game.post(&mut ctx.rooms, &self.id, &[Take(n)]) {
            Ok(_) => {
                ctx.play_ui(UiSound::Select);
                ctx.say(format!("You take {n}."));
            }
            Err(e) => {
                ctx.play_ui(UiSound::Error);
                let site = ctx.online.site().to_string();
                ctx.say(e.spoken(&site));
            }
        }
    }
}

impl State<HelloGame> for PebblesMatch {
    fn name(&self) -> &'static str {
        "pebbles_match"
    }

    fn enter(&mut self, ctx: &mut Ctx<HelloGame>) {
        ctx.rooms.watch(&self.id, true);
        if self.game.is_none() {
            if let Some(room) = ctx.rooms.room(&self.id) {
                // A room not started, or of some other kind, gives no
                // match: the standing line says so.
                if let Ok(mut game) = Match::<Pebbles>::new(&room.info) {
                    // A match reopened from My games catches up in silence.
                    game.apply_all(room);
                    self.game = Some(game);
                }
            }
        }
        let standing = self.standing(ctx);
        ctx.say(format!(
            "Pebbles with {}. Whoever takes the last pebble loses. {standing}",
            Self::other(ctx, &self.id)
        ));
    }

    fn handle_input(&mut self, ctx: &mut Ctx<HelloGame>, event: &InputEvent) {
        if event.is_repeat() {
            return;
        }
        match event.pressed() {
            Some(Key::Escape) => {
                ctx.play_ui(UiSound::Back);
                ctx.pop();
            }
            Some(Key::Num1) => self.take(ctx, 1),
            Some(Key::Num2) => self.take(ctx, 2),
            Some(Key::Num3) => self.take(ctx, 3),
            Some(Key::Space) => {
                let standing = self.standing(ctx);
                ctx.say(standing);
            }
            _ => {}
        }
    }

    fn update(&mut self, ctx: &mut Ctx<HelloGame>, _dt: f64) {
        // Notices are read by the room screen underneath once this one
        // closes; here the match itself says what changed.
        ctx.take_room_notices();
        let Some(game) = &mut self.game else {
            return;
        };
        game.update(&mut ctx.rooms);
        if let Some(e) = game.take_refusal() {
            ctx.play_ui(UiSound::Error);
            let site = ctx.online.site().to_string();
            ctx.say(e.spoken(&site));
        }
        let Some(room) = ctx.rooms.room(&self.id) else {
            return;
        };
        if game.sync(room) == Sync::Idle {
            return;
        }
        let you = room.info.you();
        let lines: Vec<String> = game
            .apply_all(room)
            .into_iter()
            .map(|turn| {
                let who = if Some(turn.seat) == you {
                    "You".to_string()
                } else {
                    room.info
                        .member(turn.seat)
                        .map_or_else(|| turn.seat.to_string(), |m| m.name.clone())
                };
                if turn.left {
                    format!("{who} left the match.")
                } else {
                    let taken: u32 = turn.inputs.iter().map(|t| u32::from(t.0)).sum();
                    format!("{who} took {taken}.")
                }
            })
            .collect();
        for line in lines {
            ctx.say(line);
        }
        let standing = self.standing(ctx);
        ctx.say_queued(standing);
        if let Some(game) = &self.game {
            if game.finished() && !self.ended {
                self.ended = true;
                ctx.play_ui(UiSound::Select);
                ctx.say_queued("Escape goes back to the room.");
            }
        }
    }

    fn lines(&self, ctx: &Ctx<HelloGame>) -> Vec<String> {
        let mut out = vec!["Pebbles".to_string(), self.standing(ctx)];
        out.extend(ctx.rooms.describe());
        out
    }

    fn help(&self, _ctx: &Ctx<HelloGame>) -> String {
        "Whoever takes the last pebble loses. On your move, 1, 2 or 3 takes that many. \
         Space repeats where the match stands. Escape goes back to the room; your seat is kept."
            .to_string()
    }
}
