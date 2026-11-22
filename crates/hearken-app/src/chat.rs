//! Text chat between the players in a room, as the engine offers it to
//! every game with rooms: two keys that work on whatever room the player
//! is looking at, a setting for how incoming lines are heard, and the
//! lines themselves spoken as they arrive.
//!
//! The room the keys act on is the one `ctx.rooms` has in focus: the last
//! room a screen watched (the Room screen, a match screen). With none in
//! focus the keys do nothing, and the game sees them as its own.
//!
//! A game whose own keys clash with the defaults moves them, or takes
//! them away and calls [`Ctx::open_chat`] and [`Ctx::read_chat_back`]
//! from keys of its own ([`Game::chat`]). A game that speaks lines its
//! own way (through a voice of its own, say) turns `spoken` off and reads
//! the `Chat` events in `take_room_notices` itself.

use std::time::{Duration, Instant};

use hearken_rooms::{EventKind, Notice, RoomId, Ticket};
use serde::{Deserialize, Serialize};

use crate::bindings::Chord;
use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::{InputEvent, Key};
use crate::screens::text_entry::TextEntryScreen;
use crate::sound::UiSound;

/// How a line from another player is heard, the player's setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatLevel {
    /// Each line is spoken after what is being said.
    #[default]
    Spoken,
    /// A sound marks each line; the read-back key tells what it said.
    Sound,
    /// Nothing. The read-back key still works.
    Off,
}

impl ChatLevel {
    pub const ALL: [ChatLevel; 3] = [ChatLevel::Spoken, ChatLevel::Sound, ChatLevel::Off];

    /// The level as the Settings row reads it.
    pub fn spoken(self) -> &'static str {
        match self {
            ChatLevel::Spoken => "spoken",
            ChatLevel::Sound => "sound only",
            ChatLevel::Off => "off",
        }
    }

    /// The next level along, wrapping, for Left and Right on the row.
    pub fn stepped(self, delta: i32) -> ChatLevel {
        let at = ChatLevel::ALL.iter().position(|l| *l == self).unwrap_or(0) as i32;
        let n = ChatLevel::ALL.len() as i32;
        ChatLevel::ALL[((at + delta).rem_euclid(n)) as usize]
    }

    pub fn is_default(&self) -> bool {
        *self == ChatLevel::default()
    }
}

/// What the engine does for chat in a game: which keys, and whether it
/// speaks incoming lines itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChatConfig {
    /// Opens a line to send to the room in focus. `None`: the game calls
    /// [`Ctx::open_chat`] from a key of its own.
    pub say: Option<Chord>,
    /// Reads the room's lines back, newest first, one per press. `None`:
    /// the game calls [`Ctx::read_chat_back`] itself.
    pub read: Option<Chord>,
    /// Whether the engine speaks lines from other players as they arrive,
    /// under the player's chat setting. Off, the game reads the `Chat`
    /// events from `take_room_notices` and speaks them its own way.
    pub spoken: bool,
}

impl Default for ChatConfig {
    /// T says, Shift+T reads back, lines are spoken.
    fn default() -> Self {
        ChatConfig {
            say: Some(Chord::plain(Key::T)),
            read: Some(Chord::shift(Key::T)),
            spoken: true,
        }
    }
}

impl ChatConfig {
    /// No engine keys and no engine speech: the game does it all.
    pub const fn none() -> Self {
        ChatConfig {
            say: None,
            read: None,
            spoken: false,
        }
    }

    /// The keys as the help reads them: "T says a line to the other
    /// players; Shift T reads the chat back." Empty with no keys.
    pub fn help(&self) -> String {
        let mut out = String::new();
        if let Some(say) = self.say {
            out.push_str(&format!(
                "{} says a line to the other players.",
                say.spoken()
            ));
        }
        if let Some(read) = self.read {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&format!(
                "{} reads the chat back, newest first.",
                read.spoken()
            ));
        }
        out
    }
}

/// Reading back: which line the last press reached, and when.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Review {
    room: RoomId,
    /// Lines back from the newest: 0 is the newest.
    back: usize,
    at: Instant,
}

/// Presses this far apart read on from the last line; later ones start
/// again at the newest.
const REVIEW_GAP: Duration = Duration::from_secs(8);

/// Lines the player sent that have not been answered, to speak a refusal.
#[derive(Debug, Default)]
pub(crate) struct ChatState {
    sent: Vec<Ticket>,
    review: Option<Review>,
}

impl<G: Game> Ctx<G> {
    /// Whether the chat keys have a room to act on: one in focus, not
    /// over, with a token to send under.
    fn chat_room(&self) -> Option<RoomId> {
        let room = self.rooms.focused()?;
        if room.info.phase.is_final() || !self.rooms.is_connected() {
            return None;
        }
        Some(room.info.id.clone())
    }

    /// Opens a line of chat to the room in focus: a text field that sends
    /// on Enter. Nothing happens with no room in focus. For a game that
    /// has its own key for it, or a Say item on a screen.
    pub fn open_chat(&mut self) {
        let Some(id) = self.chat_room() else {
            return;
        };
        let heading = match self.rooms.room(&id).map(|r| &r.info) {
            Some(info) if info.direct => format!(
                "Say to {}",
                info.others()
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "them".to_string())
            ),
            Some(info) if info.name.is_some() => {
                format!("Say in {}", info.name.clone().unwrap_or_default())
            }
            _ => "Say to the other players".to_string(),
        };
        self.push(
            TextEntryScreen::<G>::unlabelled(&heading, move |ctx, text| {
                if text.trim().is_empty() {
                    ctx.pop();
                    return;
                }
                match ctx.rooms.say(&id, text) {
                    Ok(ticket) => {
                        ctx.chat.sent.push(ticket);
                        // The sound says it went; the screen beneath has
                        // nothing new to say.
                        ctx.play_ui(UiSound::Select);
                        ctx.pop_quiet();
                    }
                    Err(e) => {
                        ctx.play_ui(UiSound::Error);
                        let site = ctx.online.site().to_string();
                        ctx.say(e.spoken(&site));
                    }
                }
            })
            .max_len(hearken_rooms::MAX_CHAT_CHARS)
            .instructions("Type a line, then press Enter to send it. Escape cancels."),
        );
    }

    /// Reads the focused room's chat back, newest first: each press within
    /// a few seconds of the last reads one line further back; a later
    /// press starts again at the newest. Nothing happens with no room in
    /// focus.
    pub fn read_chat_back(&mut self) {
        let Some(room) = self.rooms.focused() else {
            return;
        };
        let id = room.info.id.clone();
        let lines: Vec<String> = room.chat_events().filter_map(|e| e.chat_line()).collect();
        let now = Instant::now();
        let back = match &self.chat.review {
            Some(r) if r.room == id && now.duration_since(r.at) < REVIEW_GAP => r.back + 1,
            _ => 0,
        };
        if lines.is_empty() {
            self.chat.review = None;
            self.say("Nothing said yet.");
            return;
        }
        if back >= lines.len() {
            self.chat.review = Some(Review {
                room: id,
                back: lines.len() - 1,
                at: now,
            });
            self.say("That is the oldest line.");
            return;
        }
        self.chat.review = Some(Review {
            room: id,
            back,
            at: now,
        });
        let line = lines[lines.len() - 1 - back].clone();
        self.say(line);
    }

    /// A key press, against the game's chat keys: `true` when it was one
    /// and a room was there to take it.
    pub(crate) fn chat_key(&mut self, chat: &ChatConfig, event: &InputEvent) -> bool {
        let InputEvent::KeyDown {
            key,
            mods,
            repeat: false,
            ..
        } = event
        else {
            return false;
        };
        let pressed =
            |chord: Option<Chord>| chord.is_some_and(|c| c.key == *key && c.mods == *mods);
        if pressed(chat.say) {
            if self.chat_room().is_none() {
                return false;
            }
            self.open_chat();
            return true;
        }
        if pressed(chat.read) {
            if self.rooms.focused().is_none() {
                return false;
            }
            self.read_chat_back();
            return true;
        }
        false
    }

    /// Speaks what came in for chat this frame: refusals of lines the
    /// player sent, and other players' lines in watched rooms under the
    /// chat setting, when the game leaves that to the engine.
    pub(crate) fn update_chat(&mut self, notices: &[Notice], spoken: bool) {
        let sent = std::mem::take(&mut self.chat.sent);
        for ticket in sent {
            match self.rooms.result(ticket) {
                None => self.chat.sent.push(ticket),
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    self.play_ui(UiSound::Error);
                    let site = self.online.site().to_string();
                    self.say(e.spoken(&site));
                }
            }
        }
        // A new line anywhere in focus: the next read-back starts from it.
        if let Some(focus) = self.rooms.focus() {
            if notices
                .iter()
                .any(|n| matches!(n, Notice::Events(id, _) if id == focus))
            {
                self.chat.review = None;
            }
        }
        if !spoken {
            return;
        }
        let level = self.settings.online.chat;
        if level == ChatLevel::Off {
            return;
        }
        let mut lines = Vec::new();
        let mut heard = false;
        for notice in notices {
            let Notice::Events(id, first) = notice else {
                continue;
            };
            let Some(room) = self.rooms.room(id) else {
                continue;
            };
            if !room.is_watched() {
                continue;
            }
            for event in room.events_after(first.saturating_sub(1)) {
                if event.kind != EventKind::Chat || event.seat == room.info.you {
                    continue;
                }
                heard = true;
                if let Some(line) = event.chat_line() {
                    lines.push(line);
                }
            }
        }
        if !heard {
            return;
        }
        match level {
            ChatLevel::Spoken => {
                for line in lines {
                    self.say_queued(line);
                }
            }
            ChatLevel::Sound => {
                self.play_ui(UiSound::Notify);
            }
            ChatLevel::Off => {}
        }
    }
}
