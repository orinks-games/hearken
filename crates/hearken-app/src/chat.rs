//! Text chat between the players in a room, as the engine offers it to
//! every game with rooms: keys that work on whatever room the player is
//! looking at, a chat list, ready lines, a setting for how incoming lines
//! are heard, and the lines themselves said as they arrive, held while the
//! game says nothing may talk over play.
//!
//! The room the keys act on is the one `ctx.rooms` has in focus: the last
//! room a screen watched (the Room screen, a match screen, the chat list).
//! With none in focus the keys do nothing, and the game sees them as its
//! own.
//!
//! A game whose own keys clash with the defaults moves them, or takes
//! them away and calls [`Ctx::open_chat`] and [`Ctx::read_chat_back`]
//! from keys of its own ([`Game::chat`], or [`Game::chat_for`] to move
//! them for some screens only). A game that speaks lines its own way
//! (through a voice of its own, say) turns `spoken` off and reads the
//! `Chat` events in `take_room_notices` itself.
//!
//! Each machine here is explicit, changed in one place and logged:
//!
//! Quiet, shared with lobby notices (`[quiet] open -> held`):
//!
//! | State | Means | Leaves when |
//! |---|---|---|
//! | Open | Notices and chat pass; chat keys work | The game's `hold` says a timed run is on: Held |
//! | Held | Notices and chat wait; chat keys do nothing | `hold` is false again (pause, results, title): Open |
//!
//! An incoming line (`[chat] line 4: waiting -> said`):
//!
//! | State | Means | Leaves when |
//! |---|---|---|
//! | Waiting | Arrived from another player, not muted | Quiet is Held: Held. Fewer than three waiting: Said. Else: Counted |
//! | Held | Kept back while Quiet is Held | Quiet opens: Waiting |
//! | Said | Said as "Bo: text", or one sound at that setting | At once: over, kept in the room for read-back |
//! | Counted | Folded into "4 new chat lines." | At once: over, kept in the room |
//!
//! A line the player sends (`[chat] out: sending -> refused`):
//!
//! | State | Means | Leaves when |
//! |---|---|---|
//! | None | Nothing in hand | T, or Say something: the field opens |
//! | Sending | Posted, the field closed, the text kept | The server takes it: None. It refuses: Refused |
//! | Refused | The reason is said; the text is kept as a draft | T: the field opens with the draft. Another room, or five minutes: None |
//!
//! Reading back:
//!
//! | State | Means | Leaves when |
//! |---|---|---|
//! | Idle | The next press reads the newest | A press: At 0, or "Nothing said yet." |
//! | At n | Line n back was read | A press soon after: At n+1. A pause, a new line, another room: Idle. Past the oldest: End |
//! | End | "That is the oldest line." | A pause or a new line: Idle |

use std::time::{Duration, Instant};

use hearken_rooms::{Event, EventKind, Notice, RoomId, Ticket};
use serde::{Deserialize, Serialize};

use crate::bindings::Chord;
use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::{InputEvent, Key};
use crate::screens::confirm::{ConfirmOption, ConfirmScreen};
use crate::screens::text_entry::TextEntryScreen;
use crate::sound::UiSound;

/// How a line from another player is heard, the player's setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatLevel {
    /// Each line is said after what is being said.
    #[default]
    Spoken,
    /// A sound marks each line; the read-back key tells what it said.
    Sound,
    /// Nothing. The read-back key still works.
    Off,
}

impl ChatLevel {
    pub const ALL: [ChatLevel; 3] = [ChatLevel::Spoken, ChatLevel::Sound, ChatLevel::Off];

    /// The level as the Settings row reads it: "Chat: every line".
    pub fn spoken(self) -> &'static str {
        match self {
            ChatLevel::Spoken => "every line",
            ChatLevel::Sound => "a sound",
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

/// What a guest may send in a kind of room, as the server's `guest_chat`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GuestChat {
    /// Anything an account may.
    #[default]
    All,
    /// The kind's ready lines only.
    Ready,
    /// Nothing: guests listen.
    None,
}

/// A ready line a kind of room offers: sent by `id`, heard as `text` in
/// each player's own words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadyLine {
    pub id: &'static str,
    pub text: String,
}

impl ReadyLine {
    pub fn new(id: &'static str, text: impl Into<String>) -> Self {
        ReadyLine {
            id,
            text: text.into(),
        }
    }
}

/// What the engine does for chat in a game: which keys, and whether it
/// speaks incoming lines itself.
#[derive(Clone, Copy, Debug)]
pub struct ChatConfig {
    /// Opens a line to send to the room in focus. `None`: the game calls
    /// [`Ctx::open_chat`] from a key of its own.
    pub say: Option<Chord>,
    /// Reads the room's lines back, newest first, one per press. `None`:
    /// the game calls [`Ctx::read_chat_back`] itself.
    pub read: Option<Chord>,
    /// Opens the chat list ([`crate::screens::chat::ChatListScreen`]): the
    /// room in focus, else the lobby's channel. `None`: no key.
    pub list: Option<Chord>,
    /// Whether the engine speaks lines from other players as they arrive,
    /// under the player's chat setting. Off, the game reads the `Chat`
    /// events from `take_room_notices` and speaks them its own way.
    pub spoken: bool,
    /// The ready lines a kind of room offers, as the server's `quick`.
    pub ready: fn(&str) -> Vec<ReadyLine>,
    /// What a guest may send in a kind of room, as the server's `guest_chat`.
    pub guests: fn(&str) -> GuestChat,
    /// The line a player agrees to before their first typed line, once
    /// per account: "Chat rules: be kind." `None`: no rules to agree to.
    pub rules: Option<&'static str>,
    /// The longest typed line, as the server's `chat_limit.max_chars`.
    pub max_chars: usize,
}

fn no_ready(_kind: &str) -> Vec<ReadyLine> {
    Vec::new()
}

fn guests_all(_kind: &str) -> GuestChat {
    GuestChat::All
}

impl Default for ChatConfig {
    /// T says, Shift+T reads back, lines are spoken; no list key, no
    /// ready lines, guests as accounts, no rules.
    fn default() -> Self {
        ChatConfig {
            say: Some(Chord::plain(Key::T)),
            read: Some(Chord::shift(Key::T)),
            list: None,
            spoken: true,
            ready: no_ready,
            guests: guests_all,
            rules: None,
            max_chars: hearken_rooms::MAX_CHAT_CHARS,
        }
    }
}

impl ChatConfig {
    /// No engine keys and no engine speech: the game does it all.
    pub const fn none() -> Self {
        ChatConfig {
            say: None,
            read: None,
            list: None,
            spoken: false,
            ready: no_ready,
            guests: guests_all,
            rules: None,
            max_chars: hearken_rooms::MAX_CHAT_CHARS,
        }
    }

    /// The keys as the help reads them: "T says a line to the other
    /// players. Shift T reads the chat back, newest first." Empty with no
    /// keys.
    pub fn help(&self) -> String {
        let mut out = Vec::new();
        if let Some(say) = self.say {
            out.push(format!(
                "{} says a line to the other players.",
                say.spoken()
            ));
        }
        if let Some(read) = self.read {
            out.push(format!(
                "{} reads the chat back, newest first.",
                read.spoken()
            ));
        }
        if let Some(list) = self.list {
            out.push(format!("{} opens the chat list.", list.spoken()));
        }
        out.join(" ")
    }

    /// A chat line as the player hears it: a ready line in their own
    /// words, else as it was typed.
    pub fn line(&self, kind: &str, event: &Event) -> Option<String> {
        let mut event = event.clone();
        if let Some(quick) = event.quick.as_deref() {
            if let Some(ready) = (self.ready)(kind).into_iter().find(|r| r.id == quick) {
                event.text = Some(ready.text);
            }
        }
        event.chat_line()
    }
}

/// Where the screen-wide quiet stands (see the module's table).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Quiet {
    #[default]
    Open,
    Held,
}

impl Quiet {
    pub fn name(self) -> &'static str {
        match self {
            Quiet::Open => "open",
            Quiet::Held => "held",
        }
    }
}

/// An incoming line's state (see the module's table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineState {
    Waiting,
    Held,
    Said,
    Counted,
}

impl LineState {
    pub fn name(self) -> &'static str {
        match self {
            LineState::Waiting => "waiting",
            LineState::Held => "held",
            LineState::Said => "said",
            LineState::Counted => "counted",
        }
    }

    fn allows(self, to: LineState) -> bool {
        use LineState::*;
        matches!(
            (self, to),
            (Waiting, Held | Said | Counted) | (Held, Waiting)
        )
    }
}

/// A line from another player on its way to being heard.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Incoming {
    id: u64,
    room: RoomId,
    line: String,
    state: LineState,
}

/// A line the player sent and its state (see the module's table).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outgoing {
    Sending {
        ticket: Ticket,
        room: RoomId,
        text: String,
    },
    Refused {
        room: RoomId,
        text: String,
        at: Instant,
    },
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
/// A refused line is kept as a draft this long.
const DRAFT_FOR: Duration = Duration::from_secs(5 * 60);
/// From this many lines waiting at once, one count is said for them all.
const COUNT_FROM: usize = 3;

/// The engine's chat state, at `ctx.chat`.
#[derive(Debug, Default)]
pub(crate) struct ChatState {
    out: Vec<Outgoing>,
    review: Option<Review>,
    incoming: Vec<Incoming>,
    next: u64,
    pub(crate) quiet: Quiet,
}

impl<G: Game> Ctx<G> {
    /// The chat setup for the top screen.
    pub(crate) fn chat_config(&self) -> Option<ChatConfig> {
        G::chat_for(self, self.top_screen())
    }

    /// Whether notices and chat wait now: a timed run is on.
    pub fn quiet(&self) -> Quiet {
        self.chat.quiet
    }

    /// Sets the quiet from the game's `hold` for the top screen. Once a
    /// frame, before chat and notices are taken.
    pub(crate) fn update_quiet(&mut self, top: &'static str) {
        let held = G::lobby().is_some_and(|c| (c.hold)(self, top));
        let to = if held { Quiet::Held } else { Quiet::Open };
        if self.chat.quiet != to {
            let line = format!("[quiet] {} -> {}", self.chat.quiet.name(), to.name());
            log::info!("{line}");
            self.note_change(line);
            self.chat.quiet = to;
        }
    }

    /// Whether the chat keys have a room to act on: one in focus, not
    /// over, with a token to send under.
    fn chat_room(&self) -> Option<RoomId> {
        let room = self.rooms.focused()?;
        if room.info.phase.is_final() || !self.rooms.is_connected() {
            return None;
        }
        Some(room.info.id.clone())
    }

    fn set_out(&mut self, out: Outgoing) {
        let line = match &out {
            Outgoing::Sending { .. } => "[chat] out: none -> sending",
            Outgoing::Refused { .. } => "[chat] out: sending -> refused",
        };
        log::info!("{line}");
        self.note_change(line.to_string());
        self.chat.out.push(out);
    }

    /// The draft kept for `room` after a refusal, taken.
    fn take_draft(&mut self, room: &RoomId) -> Option<String> {
        let at = self.chat.out.iter().position(|o| {
            matches!(o, Outgoing::Refused { room: r, at, .. } if r == room && at.elapsed() < DRAFT_FOR)
        })?;
        match self.chat.out.remove(at) {
            Outgoing::Refused { text, .. } => {
                let line = "[chat] out: refused -> none";
                log::info!("{line}");
                self.note_change(line.to_string());
                Some(text)
            }
            Outgoing::Sending { .. } => None,
        }
    }

    /// Opens a line of chat to the room in focus: a text field that sends
    /// on Enter, with the kind's ready lines on Up and Down. Nothing
    /// happens with no room in focus. For a game that has its own key for
    /// it, or a Say item on a screen.
    pub fn open_chat(&mut self) {
        let Some(id) = self.chat_room() else {
            return;
        };
        let Some(config) = self.chat_config().or_else(G::chat) else {
            return;
        };
        let Some(info) = self.rooms.room(&id).map(|r| r.info.clone()) else {
            return;
        };
        let heading = if info.direct {
            format!(
                "Say to {}",
                info.others()
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "them".to_string())
            )
        } else if let Some(name) = &info.name {
            match G::lobby()
                .and_then(|l| l.channel)
                .filter(|c| c.kind == info.kind)
            {
                Some(channel) => format!("Say in the {}", channel.label),
                None => format!("Say in {name}"),
            }
        } else {
            match info.others().as_slice() {
                [one] => format!("Say to {one}"),
                _ => "Say to the other players".to_string(),
            }
        };
        let guest = self.online.is_guest();
        let allowed = if guest {
            (config.guests)(&info.kind)
        } else {
            GuestChat::All
        };
        let site = self.online.site().to_string();
        let ready = if allowed == GuestChat::None {
            Vec::new()
        } else {
            (config.ready)(&info.kind)
        };
        if allowed == GuestChat::None || (allowed == GuestChat::Ready && ready.is_empty()) {
            self.play_ui(UiSound::Error);
            self.say(format!("A typed line needs a {site} account."));
            return;
        }
        let typing = allowed == GuestChat::All;
        let instructions = match (typing, ready.is_empty()) {
            (true, true) => "Type a line. Enter sends it. Escape cancels.".to_string(),
            (true, false) => {
                "Type a line. Enter sends it. Up and Down choose a ready line. Escape cancels."
                    .to_string()
            }
            (false, _) => format!(
                "Ready lines. A typed line needs a {site} account. Up and Down choose, Enter sends."
            ),
        };
        let draft = self.take_draft(&id);
        let texts: Vec<String> = ready.iter().map(|r| r.text.clone()).collect();
        let ids: Vec<&'static str> = ready.iter().map(|r| r.id).collect();
        let typed_room = id.clone();
        let rules = config.rules;
        let mut entry = TextEntryScreen::<G>::unlabelled(&heading, move |ctx, text| {
            if text.trim().is_empty() {
                ctx.play_ui(UiSound::Back);
                ctx.pop();
                return;
            }
            let text = text.to_string();
            let room = typed_room.clone();
            if let Some(rules) = rules.filter(|_| !ctx.chat_rules_agreed()) {
                // The first typed line of the account: the rules first.
                ctx.push(
                    ConfirmScreen::<G>::with_options(
                        format!("{rules} Enter agrees."),
                        vec![
                            ConfirmOption::new("Agree", move |ctx: &mut Ctx<G>| {
                                ctx.agree_chat_rules();
                                ctx.pop_quiet();
                                ctx.send_line(&room, &text);
                                ctx.pop_quiet();
                            }),
                            ConfirmOption::close("Not now"),
                        ],
                    )
                    .cancel(1),
                );
                return;
            }
            ctx.send_line(&room, &text);
            ctx.pop_quiet();
        })
        .max_len(config.max_chars.min(hearken_rooms::MAX_CHAT_CHARS))
        .instructions(instructions);
        let ready_room = id.clone();
        entry = entry.ready_lines(
            texts,
            (!typing).then(|| format!("A typed line needs a {site} account.")),
            move |ctx, at| {
                let Some(quick) = ids.get(at) else {
                    return;
                };
                match ctx.rooms.say_quick(&ready_room, quick) {
                    Ok(ticket) => {
                        ctx.set_out(Outgoing::Sending {
                            ticket,
                            room: ready_room.clone(),
                            text: String::new(),
                        });
                        ctx.play_ui(UiSound::Select);
                        ctx.pop_quiet();
                    }
                    Err(e) => {
                        ctx.play_ui(UiSound::Error);
                        let site = ctx.online.site().to_string();
                        ctx.say(e.spoken(&site));
                    }
                }
            },
        );
        if let Some(draft) = draft {
            entry = entry.with_text(&draft);
        }
        self.push(entry);
    }

    /// Sends a typed line, keeping it as a draft if it is refused.
    fn send_line(&mut self, room: &RoomId, text: &str) {
        match self.rooms.say(room, text) {
            Ok(ticket) => {
                self.set_out(Outgoing::Sending {
                    ticket,
                    room: room.clone(),
                    text: text.to_string(),
                });
                // The sound says it went; the screen beneath has nothing
                // new to say.
                self.play_ui(UiSound::Select);
            }
            Err(e) => {
                self.play_ui(UiSound::Error);
                let site = self.online.site().to_string();
                self.say(e.spoken(&site));
                self.set_out(Outgoing::Refused {
                    room: room.clone(),
                    text: text.to_string(),
                    at: Instant::now(),
                });
            }
        }
    }

    /// Whether this account has agreed to the game's chat rules.
    pub fn chat_rules_agreed(&self) -> bool {
        let who = self.settings.online.name.as_deref().unwrap_or("");
        self.settings.online.chat_rules.as_deref() == Some(who)
    }

    fn agree_chat_rules(&mut self) {
        self.settings.online.chat_rules =
            Some(self.settings.online.name.clone().unwrap_or_default());
        self.save_settings();
        let line = "[chat] rules: asked -> agreed";
        log::info!("{line}");
        self.note_change(line.to_string());
    }

    /// The focused room's chat lines as the player hears them, oldest
    /// first, with each line's event.
    pub fn chat_lines(&self, id: &RoomId) -> Vec<(Event, String)> {
        let Some(room) = self.rooms.room(id) else {
            return Vec::new();
        };
        let config = self.chat_config().or_else(G::chat).unwrap_or_default();
        room.chat_events()
            .filter(|e| !e.handle.as_deref().is_some_and(|h| self.rooms.is_hidden(h)))
            .filter_map(|e| config.line(&room.info.kind, e).map(|l| (e.clone(), l)))
            .collect()
    }

    /// Reads the focused room's chat back, newest first: each press within
    /// a few seconds of the last reads one line further back; a later
    /// press starts again at the newest. Nothing happens with no room in
    /// focus.
    pub fn read_chat_back(&mut self) {
        let Some(id) = self.rooms.focus().cloned() else {
            return;
        };
        let lines: Vec<String> = self.chat_lines(&id).into_iter().map(|(_, l)| l).collect();
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

    /// Opens the chat list: the room in focus, else the lobby's channel.
    /// Nothing happens offline, or with neither.
    pub fn open_chat_list(&mut self) -> bool {
        if !self.rooms.is_connected() {
            return false;
        }
        if let Some(id) = self.chat_room() {
            self.push(crate::screens::chat::ChatListScreen::<G>::table(id));
            return true;
        }
        let Some(channel) = G::lobby().and_then(|l| l.channel) else {
            return false;
        };
        self.push(crate::screens::chat::ChatListScreen::<G>::channel(channel));
        true
    }

    /// A key press, against the game's chat keys: `true` when it was one
    /// and there was something for it to do. While the quiet is held the
    /// keys do nothing and say nothing.
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
        let held = self.chat.quiet == Quiet::Held;
        if pressed(chat.say) {
            if self.chat_room().is_none() {
                return false;
            }
            if !held {
                self.open_chat();
            }
            return true;
        }
        if pressed(chat.read) {
            if self.rooms.focused().is_none() {
                return false;
            }
            if !held {
                self.read_chat_back();
            }
            return true;
        }
        if pressed(chat.list) {
            if held {
                return self.chat_room().is_some()
                    || G::lobby().is_some_and(|l| l.channel.is_some());
            }
            return self.open_chat_list();
        }
        false
    }

    fn set_line_state(&mut self, id: u64, to: LineState) {
        let Some(line) = self.chat.incoming.iter_mut().find(|l| l.id == id) else {
            return;
        };
        if !line.state.allows(to) {
            log::warn!(
                "[chat] line {id}: no change from {} to {}",
                line.state.name(),
                to.name()
            );
            return;
        }
        let change = format!("[chat] line {id}: {} -> {}", line.state.name(), to.name());
        line.state = to;
        log::info!("{change}");
        self.note_change(change);
    }

    /// Takes what came in for chat this frame: answers to lines the player
    /// sent, and other players' lines in watched rooms, which wait for
    /// [`Ctx::flush_chat`] when the game leaves speaking them to the
    /// engine.
    pub(crate) fn update_chat(&mut self, notices: &[Notice], config: &ChatConfig) {
        let out = std::mem::take(&mut self.chat.out);
        for o in out {
            match o {
                Outgoing::Sending { ticket, room, text } => match self.rooms.result(ticket) {
                    None => self.chat.out.push(Outgoing::Sending { ticket, room, text }),
                    Some(Ok(_)) => {
                        let line = "[chat] out: sending -> none";
                        log::info!("{line}");
                        self.note_change(line.to_string());
                    }
                    Some(Err(e)) => {
                        self.play_ui(UiSound::Error);
                        let site = self.online.site().to_string();
                        self.say(e.spoken(&site));
                        if !text.is_empty() {
                            self.set_out(Outgoing::Refused {
                                room,
                                text,
                                at: Instant::now(),
                            });
                        }
                    }
                },
                refused @ Outgoing::Refused { .. } => {
                    if let Outgoing::Refused { at, .. } = &refused {
                        if at.elapsed() < DRAFT_FOR {
                            self.chat.out.push(refused);
                        }
                    }
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
        if !config.spoken || self.settings.online.chat == ChatLevel::Off {
            return;
        }
        let mut lines = Vec::new();
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
                // Lines said before the player came in are in the chat
                // list, not news.
                if event.kind != EventKind::Chat
                    || event.seat == room.info.you
                    || event.seq <= room.history_to()
                    || event
                        .handle
                        .as_deref()
                        .is_some_and(|h| self.rooms.is_hidden(h))
                {
                    continue;
                }
                if let Some(line) = config.line(&room.info.kind, event) {
                    lines.push((id.clone(), line));
                }
            }
        }
        for (room, line) in lines {
            self.chat.next += 1;
            let id = self.chat.next;
            let change = format!("[chat] line {id}: new -> waiting");
            log::info!("{change}");
            self.note_change(change);
            self.chat.incoming.push(Incoming {
                id,
                room,
                line,
                state: LineState::Waiting,
            });
        }
    }

    /// Says the lines waiting, unless the quiet is held: one or two as
    /// they are, three or more as one count. Once a frame, after notices,
    /// so a released run hears its notices first.
    pub(crate) fn flush_chat(&mut self) {
        if self.chat.incoming.is_empty() {
            return;
        }
        let ids: Vec<(u64, LineState)> =
            self.chat.incoming.iter().map(|l| (l.id, l.state)).collect();
        if self.chat.quiet == Quiet::Held {
            for (id, state) in ids {
                if state == LineState::Waiting {
                    self.set_line_state(id, LineState::Held);
                }
            }
            return;
        }
        for (id, state) in &ids {
            if *state == LineState::Held {
                self.set_line_state(*id, LineState::Waiting);
            }
        }
        // Lines from a room no longer known are dropped.
        let incoming = std::mem::take(&mut self.chat.incoming);
        let live: Vec<Incoming> = incoming
            .into_iter()
            .filter(|l| self.rooms.room(&l.room).is_some())
            .collect();
        let count = live.len();
        let level = self.settings.online.chat;
        self.chat.incoming = live.clone();
        if count >= COUNT_FROM {
            for l in &live {
                self.set_line_state(l.id, LineState::Counted);
            }
            match level {
                ChatLevel::Spoken => self.say_queued(format!("{count} new chat lines.")),
                ChatLevel::Sound => {
                    self.play_ui(UiSound::Notify);
                }
                ChatLevel::Off => {}
            }
        } else {
            for l in &live {
                self.set_line_state(l.id, LineState::Said);
            }
            match level {
                ChatLevel::Spoken => {
                    for l in live {
                        self.say_queued(l.line);
                    }
                }
                ChatLevel::Sound => {
                    self.play_ui(UiSound::Notify);
                }
                ChatLevel::Off => {}
            }
        }
        self.chat.incoming.clear();
    }

    /// Lines for the agent server's `screen`: chat lines waiting or held.
    pub fn describe_chat(&self) -> Vec<String> {
        let mut out = vec![format!("quiet {}", self.chat.quiet.name())];
        for l in &self.chat.incoming {
            out.push(format!("chat line {} {}: {}", l.id, l.state.name(), l.line));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_machine_refuses_a_jump_it_does_not_have() {
        use LineState::*;
        assert!(Waiting.allows(Held));
        assert!(Held.allows(Waiting));
        assert!(Waiting.allows(Counted));
        assert!(!Held.allows(Said));
        assert!(!Said.allows(Waiting));
    }

    #[test]
    fn a_ready_line_is_heard_in_the_players_words() {
        fn ready(kind: &str) -> Vec<ReadyLine> {
            if kind == "pebbles" {
                vec![ReadyLine::new("gg", "Good game.")]
            } else {
                Vec::new()
            }
        }
        let config = ChatConfig {
            ready,
            ..ChatConfig::default()
        };
        let event: Event = serde_json::from_value(serde_json::json!({
            "seq": 3, "type": "chat", "name": "Bo", "text": "gg-fallback", "quick": "gg"
        }))
        .unwrap();
        assert_eq!(config.line("pebbles", &event).unwrap(), "Bo: Good game.");
        assert_eq!(config.line("other", &event).unwrap(), "Bo: gg-fallback");
    }

    #[test]
    fn the_setting_names_what_reaches_the_player() {
        assert_eq!(ChatLevel::Spoken.spoken(), "every line");
        assert_eq!(ChatLevel::Sound.spoken(), "a sound");
    }
}
