//! Online play between players, ready made: the Online play menu a game
//! pushes for one of its room kinds, the Room screen that speaks the invite
//! code and the players as they arrive, and the My games list.
//!
//! The game describes its kind once ([`RoomKind`]) and says how to open a
//! match that has started; everything up to that point is these screens.
//! Each screen's state is explicit and spoken as it changes: a request on
//! its way (`Asking`), a room waiting, a match in play, a connection lost
//! and found. The screens read `ctx.rooms` and take `ctx.take_room_notices`
//! on their `update`.

use std::rc::Rc;
use std::time::{Duration, Instant};

use hearken_rooms::{Answer, Connection, EventKind, Notice, Phase, RoomId, RoomInfo, Ticket};
use serde_json::Value;

use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::{InputEvent, Key};
use crate::menu::{Menu, MenuCore, MenuItem};
use crate::screens::confirm::ConfirmScreen;
use crate::screens::online::OnlineAccountScreen;
use crate::screens::text_entry::TextEntryScreen;
use crate::sound::UiSound;

/// Opens the game's own screen for a match in play: pushed over the Room
/// screen, so a finished match comes back to the room and its result.
pub type OpenMatch<G> = Rc<dyn Fn(&mut Ctx<G>, &RoomId)>;

/// What the Online play screens know about one of a game's kinds of room.
pub struct RoomKind<G: Game> {
    /// The kind as the server names it (`"pebbles"`).
    pub kind: String,
    /// The kind as the player hears it (`"Pebbles"`).
    pub label: String,
    /// What Create and Play with anyone send as the room's context.
    pub context: Value,
    /// Opens the game's screen for a match in play.
    pub open: OpenMatch<G>,
}

impl<G: Game> RoomKind<G> {
    pub fn new(
        kind: &str,
        label: &str,
        context: &impl serde::Serialize,
        open: impl Fn(&mut Ctx<G>, &RoomId) + 'static,
    ) -> Self {
        RoomKind {
            kind: kind.to_string(),
            label: label.to_string(),
            context: serde_json::to_value(context).unwrap_or(Value::Null),
            open: Rc::new(open),
        }
    }

    /// `kind` as the player hears it: this kind's label, else the name.
    fn label_of<'a>(&'a self, kind: &'a str) -> &'a str {
        if kind == self.kind {
            &self.label
        } else {
            kind
        }
    }
}

impl<G: Game> Clone for RoomKind<G> {
    fn clone(&self) -> Self {
        RoomKind {
            kind: self.kind.clone(),
            label: self.label.clone(),
            context: self.context.clone(),
            open: self.open.clone(),
        }
    }
}

/// What a screen is waiting on from the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Asking {
    Nothing,
    Create(Ticket),
    Join(Ticket),
    Find(Ticket),
    List(Ticket),
    Start(Ticket),
    Leave(Ticket),
}

impl Asking {
    fn ticket(self) -> Option<Ticket> {
        match self {
            Asking::Nothing => None,
            Asking::Create(t)
            | Asking::Join(t)
            | Asking::Find(t)
            | Asking::List(t)
            | Asking::Start(t)
            | Asking::Leave(t) => Some(t),
        }
    }
}

/// The line for a connection notice, spoken once by whichever screen is
/// open.
fn connection_line(site: &str, connection: Connection) -> String {
    match connection {
        Connection::Offline => format!("Could not reach {site}. Still trying."),
        Connection::Polling => format!("Reached {site} again."),
    }
}

/// The players as a list: "Ada, you, and Bo".
fn players_line(info: &RoomInfo) -> String {
    let names: Vec<String> = info
        .seated()
        .map(|m| {
            let mut name = m.name.clone();
            if Some(m.seat) == info.you {
                name.push_str(" (you)");
            }
            if m.seat == info.host {
                name.push_str(", host");
            }
            name
        })
        .collect();
    match names.len() {
        0 => "No players".to_string(),
        1 => names[0].clone(),
        _ => names.join("; "),
    }
}

/// One line for a room in a list: whose game it is and where it stands.
fn room_line<G: Game>(kind: &RoomKind<G>, info: &RoomInfo) -> String {
    let label = kind.label_of(&info.kind);
    let others = info.others();
    let with = if others.is_empty() {
        String::new()
    } else {
        format!(" with {}", others.join(" and "))
    };
    let standing = match info.phase {
        Phase::Waiting => format!("waiting for {} more", info.missing()),
        Phase::Playing => info
            .you()
            .and_then(|seat| info.summary_for(seat))
            .unwrap_or_else(|| {
                if info.my_turn() {
                    "your move".to_string()
                } else {
                    "in play".to_string()
                }
            }),
        Phase::Finished => info
            .you()
            .and_then(|seat| info.summary_for(seat))
            .map_or_else(|| "over".to_string(), |s| format!("over, {s}")),
        other => other.spoken().to_string(),
    };
    let unread = if info.unread > 0 && info.phase != Phase::Waiting {
        format!(", {} new", info.unread)
    } else {
        String::new()
    };
    format!("{label}{with}: {standing}{unread}")
}

/// Where a room the player just entered goes: a waiting room to its screen,
/// a match in play to the game.
fn enter_room<G: Game>(ctx: &mut Ctx<G>, kind: &RoomKind<G>, id: &RoomId) {
    let phase = ctx.rooms.room(id).map(|r| r.info.phase);
    match phase {
        Some(Phase::Playing) => {
            ctx.push(RoomScreen::new(kind.clone(), id.clone()));
            (kind.open)(ctx, id);
        }
        _ => ctx.push(RoomScreen::new(kind.clone(), id.clone())),
    }
}

// -- Online play ---------------------------------------------------------------

/// The Online play menu for one kind of room: Create a game, Join with a
/// code, Play with anyone, My games. With no account connected it offers
/// to connect one first.
pub struct OnlinePlayScreen<G: Game> {
    kind: RoomKind<G>,
    menu: MenuCore<Self, G>,
    asking: Asking,
    /// A code typed on the entry screen, waiting to be sent.
    typed_code: Rc<std::cell::RefCell<Option<String>>>,
}

impl<G: Game> OnlinePlayScreen<G> {
    pub fn new(kind: RoomKind<G>) -> Self {
        OnlinePlayScreen {
            menu: MenuCore::new(format!("{} online", kind.label)),
            kind,
            asking: Asking::Nothing,
            typed_code: Rc::new(std::cell::RefCell::new(None)),
        }
    }

    fn site(ctx: &Ctx<G>) -> String {
        ctx.online.site().to_string()
    }

    fn ask(
        &mut self,
        ctx: &mut Ctx<G>,
        sent: Result<Asking, hearken_rooms::RoomsError>,
        doing: &str,
    ) {
        match sent {
            Ok(asking) => {
                self.asking = asking;
                ctx.say(format!("{doing}."));
            }
            Err(e) => {
                ctx.play_ui(UiSound::Error);
                ctx.say(e.spoken(&Self::site(ctx)));
            }
        }
    }

    fn create(&mut self, ctx: &mut Ctx<G>) {
        let sent = ctx
            .rooms
            .create(&self.kind.kind, &self.kind.context, false)
            .map(Asking::Create);
        self.ask(ctx, sent, "Creating a game");
    }

    fn find(&mut self, ctx: &mut Ctx<G>) {
        let sent = ctx.rooms.find(&self.kind.kind).map(Asking::Find);
        self.ask(ctx, sent, "Looking for a game");
    }

    fn join(&mut self, ctx: &mut Ctx<G>, code: &str) {
        let sent = ctx.rooms.join(code).map(Asking::Join);
        self.ask(ctx, sent, "Joining");
    }
}

impl<G: Game> Menu for OnlinePlayScreen<G> {
    type Game = G;

    fn name(&self) -> &'static str {
        "online_play"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn build_items(&mut self, ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        let site = Self::site(ctx);
        if !ctx.online.is_connected() {
            let label = if ctx.online.offers_guests() {
                "Play online: connect an account or play as a guest".to_string()
            } else {
                format!("Connect a {site} account to play online")
            };
            return vec![MenuItem::new(label, |_: &mut Self, ctx: &mut Ctx<G>| {
                ctx.push(OnlineAccountScreen::new());
            })
            .help("Online play needs an account, so other players know who they are playing.")];
        }
        vec![
            MenuItem::new("Create a game", |s: &mut Self, ctx: &mut Ctx<G>| {
                if s.asking == Asking::Nothing {
                    s.create(ctx);
                } else {
                    ctx.say(format!("Still asking {}.", Self::site(ctx)));
                }
            })
            .help("Make a game and hear its invite code, to give to the people you want to play."),
            MenuItem::new("Join with a code", |s: &mut Self, ctx: &mut Ctx<G>| {
                let typed = s.typed_code.clone();
                ctx.push(
                    TextEntryScreen::<G>::new("Join a game", "Invite code", move |ctx, text| {
                        match hearken_rooms::normalize_code(text) {
                            Some(code) => {
                                *typed.borrow_mut() = Some(code);
                                ctx.pop_quiet();
                            }
                            None => {
                                ctx.play_ui(UiSound::Error);
                                ctx.say("A code is six letters and numbers.");
                            }
                        }
                    })
                    .instructions("Type the six letters and numbers you were given. Enter joins, Escape goes back.")
                    .max_len(12),
                );
            })
            .help("Type the code another player gave you."),
            MenuItem::new("Play with anyone", |s: &mut Self, ctx: &mut Ctx<G>| {
                if s.asking == Asking::Nothing {
                    s.find(ctx);
                } else {
                    ctx.say(format!("Still asking {}.", Self::site(ctx)));
                }
            })
            .help("Join a game that is waiting for players, or start one for the next player to find."),
            MenuItem::new("My games", |s: &mut Self, ctx: &mut Ctx<G>| {
                ctx.push(MyGamesScreen::new(s.kind.clone()));
            })
            .help("The games you are in: whose move it is, and what has happened since you looked."),
        ]
    }

    fn resume_quiet(&mut self, ctx: &mut Ctx<G>) {
        self.refresh(ctx, true);
        let code = self.typed_code.borrow_mut().take();
        if let Some(code) = code {
            self.join(ctx, &code);
        }
    }

    fn update(&mut self, ctx: &mut Ctx<G>, _dt: f64) {
        for notice in ctx.take_room_notices() {
            if let Notice::Connection(c) = notice {
                ctx.say(connection_line(&Self::site(ctx), c));
            }
        }
        let Some(ticket) = self.asking.ticket() else {
            return;
        };
        let Some(answer) = ctx.rooms.result(ticket) else {
            return;
        };
        self.asking = Asking::Nothing;
        match answer {
            Ok(Answer::Room(id)) => {
                ctx.play_ui(UiSound::Select);
                enter_room(ctx, &self.kind, &id);
            }
            Ok(_) => {}
            Err(e) => {
                ctx.play_ui(UiSound::Error);
                ctx.say(e.spoken(&Self::site(ctx)));
            }
        }
    }

    fn lines(&self, ctx: &Ctx<G>) -> Vec<String> {
        let mut out = vec![self.menu.title.clone()];
        if self.asking != Asking::Nothing {
            out.push(format!("Asking {}.", Self::site(ctx)));
        }
        for (i, item) in self.menu.items.iter().enumerate() {
            let marker = if i == self.menu.index { "> " } else { "  " };
            out.push(format!("{marker}{}", item.text(self, ctx)));
        }
        out.extend(ctx.rooms.describe());
        out
    }
}

// -- Room ----------------------------------------------------------------------

/// One room: its invite code, its players as they come and go, Start for
/// the host, and the result once the match is over. A match in play is the
/// game's own screen, opened over this one.
pub struct RoomScreen<G: Game> {
    kind: RoomKind<G>,
    id: RoomId,
    menu: MenuCore<Self, G>,
    asking: Asking,
    /// The phase last spoken, so each change is spoken once.
    heard: Option<Phase>,
}

impl<G: Game> RoomScreen<G> {
    pub fn new(kind: RoomKind<G>, id: RoomId) -> Self {
        RoomScreen {
            menu: MenuCore::new(format!("{} game", kind.label)),
            kind,
            id,
            asking: Asking::Nothing,
            heard: None,
        }
    }

    fn info<'a>(&self, ctx: &'a Ctx<G>) -> Option<&'a RoomInfo> {
        ctx.rooms.room(&self.id).map(|r| &r.info)
    }

    fn site(ctx: &Ctx<G>) -> String {
        ctx.online.site().to_string()
    }

    fn code_line(info: &RoomInfo) -> String {
        format!("Invite code {}", hearken_rooms::spell_code(&info.code))
    }

    /// What a new event says, if anything.
    fn event_line(&self, ctx: &Ctx<G>, event: &hearken_rooms::Event) -> Option<String> {
        let info = self.info(ctx)?;
        let mine = event.seat.is_some() && event.seat == info.you;
        let name = event.name.clone().unwrap_or_else(|| {
            event
                .seat()
                .and_then(|s| info.member(s))
                .map_or_else(|| "A player".to_string(), |m| m.name.clone())
        });
        match event.kind {
            EventKind::Join if !mine => Some(format!("{name} joined.")),
            EventKind::Leave if !mine => Some(format!("{name} left.")),
            EventKind::Left if !mine => Some(format!("{name} left the match.")),
            _ => None,
        }
    }

    fn leave(&mut self, ctx: &mut Ctx<G>) {
        match ctx.rooms.leave(&self.id) {
            Ok(ticket) => {
                self.asking = Asking::Leave(ticket);
                ctx.say("Leaving.");
            }
            Err(e) => {
                ctx.play_ui(UiSound::Error);
                ctx.say(e.spoken(&Self::site(ctx)));
            }
        }
    }
}

impl<G: Game> Menu for RoomScreen<G> {
    type Game = G;

    fn name(&self) -> &'static str {
        "room"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn enter(&mut self, ctx: &mut Ctx<G>) {
        ctx.rooms.watch(&self.id, true);
        self.heard = self.info(ctx).map(|i| i.phase);
        crate::menu::menu_enter(self, ctx);
    }

    fn exit(&mut self, ctx: &mut Ctx<G>) {
        ctx.rooms.watch(&self.id, false);
    }

    fn resume(&mut self, ctx: &mut Ctx<G>) {
        ctx.rooms.watch(&self.id, true);
        self.heard = self.info(ctx).map(|i| i.phase);
        self.refresh(ctx, true);
        self.announce(ctx);
    }

    fn build_items(&mut self, ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        let Some(info) = self.info(ctx).cloned() else {
            return vec![MenuItem::inert("This game is gone.")];
        };
        let mut items = Vec::new();
        match info.phase {
            Phase::Waiting => {
                let code = Self::code_line(&info);
                items.push(
                    MenuItem::new(code.clone(), move |_: &mut Self, ctx: &mut Ctx<G>| {
                        ctx.say(code.clone());
                    })
                    .help("Give this code to the people you want to play. Enter repeats it.")
                    .select_sound(None),
                );
                let missing = info.missing();
                items.push(MenuItem::inert(format!(
                    "Players: {}. {}",
                    players_line(&info),
                    if missing > 0 {
                        format!("Waiting for {missing} more.")
                    } else {
                        "Ready to start.".to_string()
                    }
                )));
                if info.is_host() {
                    items.push(
                        MenuItem::new("Start the game", |s: &mut Self, ctx: &mut Ctx<G>| {
                            if s.asking != Asking::Nothing {
                                ctx.say(format!("Still asking {}.", Self::site(ctx)));
                                return;
                            }
                            match ctx.rooms.start(&s.id) {
                                Ok(ticket) => {
                                    s.asking = Asking::Start(ticket);
                                    ctx.say("Starting.");
                                }
                                Err(e) => {
                                    ctx.play_ui(UiSound::Error);
                                    ctx.say(e.spoken(&Self::site(ctx)));
                                }
                            }
                        })
                        .help(
                            "Begin once enough players have joined. Seats still free stay empty.",
                        ),
                    );
                } else {
                    items.push(MenuItem::inert(format!(
                        "Waiting for {} to start the game",
                        info.host_name().unwrap_or("the host")
                    )));
                }
            }
            Phase::Playing => {
                items.push(MenuItem::inert(format!("Players: {}", players_line(&info))));
                let standing = info
                    .you()
                    .and_then(|s| info.summary_for(s))
                    .unwrap_or_else(|| "In play".to_string());
                items.push(
                    MenuItem::new(
                        format!("Play: {standing}"),
                        |s: &mut Self, ctx: &mut Ctx<G>| {
                            let open = s.kind.open.clone();
                            open(ctx, &s.id);
                        },
                    )
                    .help("Go to the match."),
                );
            }
            phase => {
                let standing = info.you().and_then(|s| info.summary_for(s)).map_or_else(
                    || format!("This game is {}.", phase.spoken()),
                    |s| format!("Over: {s}."),
                );
                items.push(MenuItem::inert(standing));
                items.push(MenuItem::inert(format!("Players: {}", players_line(&info))));
            }
        }
        if info.phase.is_final() {
            items.push(
                MenuItem::new("Remove from my games", |s: &mut Self, ctx: &mut Ctx<G>| {
                    ctx.rooms.forget(&s.id);
                    ctx.pop_with("Removed.");
                })
                .help("Forget this finished game on this computer."),
            );
        } else {
            let label = if info.phase == Phase::Playing {
                "Leave the match"
            } else {
                "Leave the game"
            };
            items.push(
                MenuItem::new(label, |s: &mut Self, ctx: &mut Ctx<G>| {
                    let playing = s.info(ctx).is_some_and(|i| i.phase == Phase::Playing);
                    if playing {
                        let id = s.id.clone();
                        ctx.push(ConfirmScreen::<G>::new(
                            "Leave the match? The other players win it.",
                            move |ctx| {
                                // The answer goes unread: the seat is given
                                // up either way, and the room is forgotten.
                                let _ = ctx.rooms.leave(&id);
                                ctx.rooms.forget(&id);
                                ctx.pop();
                                ctx.pop_with("Left the match.");
                            },
                        ));
                    } else {
                        s.leave(ctx);
                    }
                })
                .help("Give up your seat. Escape instead keeps it and goes back; the game stays in My games."),
            );
        }
        items
    }

    fn update(&mut self, ctx: &mut Ctx<G>, _dt: f64) {
        let mut changed = false;
        let mut lines = Vec::new();
        for notice in ctx.take_room_notices() {
            match notice {
                Notice::Connection(c) => lines.push(connection_line(&Self::site(ctx), c)),
                Notice::Events(id, first) if id == self.id => {
                    if let Some(room) = ctx.rooms.room(&self.id) {
                        let events: Vec<_> = room.events_after(first - 1).cloned().collect();
                        for event in &events {
                            if let Some(line) = self.event_line(ctx, event) {
                                lines.push(line);
                            }
                        }
                    }
                    changed = true;
                }
                Notice::Updated(id) if id == self.id => changed = true,
                Notice::Phase(id, _) if id == self.id => changed = true,
                Notice::Dropped(id, e) if id == self.id => {
                    lines.push(e.spoken(&Self::site(ctx)));
                    changed = true;
                }
                _ => {}
            }
        }
        if let Some(ticket) = self.asking.ticket() {
            if let Some(answer) = ctx.rooms.result(ticket) {
                let asking = self.asking;
                self.asking = Asking::Nothing;
                match (asking, answer) {
                    (Asking::Leave(_), Ok(_)) => {
                        let id = self.id.clone();
                        ctx.rooms.forget(&id);
                        ctx.play_ui(UiSound::Select);
                        ctx.pop_with("Left the game.");
                        return;
                    }
                    (_, Ok(_)) => changed = true,
                    (_, Err(e)) => {
                        ctx.play_ui(UiSound::Error);
                        lines.push(e.spoken(&Self::site(ctx)));
                    }
                }
            }
        }
        let phase = self.info(ctx).map(|i| i.phase);
        if phase != self.heard {
            self.heard = phase;
            changed = true;
            match phase {
                Some(Phase::Playing) => {
                    ctx.play_ui(UiSound::Select);
                    for line in lines.drain(..) {
                        ctx.say(line);
                    }
                    ctx.say("The game starts.");
                    self.refresh(ctx, true);
                    let open = self.kind.open.clone();
                    open(ctx, &self.id);
                    return;
                }
                Some(Phase::Finished) => lines.push("The game is over.".to_string()),
                Some(phase) if phase.is_final() => {
                    lines.push(format!("This game is {}.", phase.spoken()));
                }
                _ => {}
            }
        }
        if changed {
            self.refresh(ctx, true);
        }
        for line in lines {
            ctx.say(line);
        }
    }

    fn lines(&self, ctx: &Ctx<G>) -> Vec<String> {
        let mut out = vec![self.menu.title.clone()];
        if let Some(info) = self.info(ctx) {
            out.push(format!("Code: {}", info.code));
            out.push(format!("Phase: {:?}", info.phase));
        }
        for (i, item) in self.menu.items.iter().enumerate() {
            let marker = if i == self.menu.index { "> " } else { "  " };
            out.push(format!("{marker}{}", item.text(self, ctx)));
        }
        out.extend(ctx.rooms.describe());
        out
    }

    fn help(&self, ctx: &Ctx<G>) -> String {
        self.current_help(ctx).unwrap_or_else(|| {
            "The players in this game and what they can do. Up and Down move, Enter chooses, \
             Escape goes back and keeps your seat."
                .to_string()
        })
    }
}

// -- My games ------------------------------------------------------------------

/// The player's rooms, newest first: each as one line with whose move it is
/// and what has happened since. Enter opens a room; the list refreshes
/// itself while open.
pub struct MyGamesScreen<G: Game> {
    kind: RoomKind<G>,
    menu: MenuCore<Self, G>,
    asking: Asking,
    rooms: Vec<RoomInfo>,
    listed: bool,
    next_list: Instant,
}

/// How often the list is asked for again while the screen is open.
const RELIST: Duration = Duration::from_secs(10);

impl<G: Game> MyGamesScreen<G> {
    pub fn new(kind: RoomKind<G>) -> Self {
        MyGamesScreen {
            kind,
            menu: MenuCore::new("My games"),
            asking: Asking::Nothing,
            rooms: Vec::new(),
            listed: false,
            next_list: Instant::now(),
        }
    }

    fn site(ctx: &Ctx<G>) -> String {
        ctx.online.site().to_string()
    }

    fn list(&mut self, ctx: &mut Ctx<G>) {
        if self.asking != Asking::Nothing {
            return;
        }
        if let Ok(ticket) = ctx.rooms.list() {
            self.asking = Asking::List(ticket);
        }
        self.next_list = Instant::now() + RELIST;
    }

    fn open(&mut self, ctx: &mut Ctx<G>, index: usize) {
        let Some(info) = self.rooms.get(index).cloned() else {
            return;
        };
        let id = info.id.clone();
        ctx.rooms.adopt(info);
        enter_room(ctx, &self.kind, &id);
    }
}

impl<G: Game> Menu for MyGamesScreen<G> {
    type Game = G;

    fn name(&self) -> &'static str {
        "my_games"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn enter(&mut self, ctx: &mut Ctx<G>) {
        self.list(ctx);
        crate::menu::menu_enter(self, ctx);
    }

    fn resume(&mut self, ctx: &mut Ctx<G>) {
        self.list(ctx);
        self.refresh(ctx, true);
        self.announce(ctx);
    }

    fn build_items(&mut self, ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        if !self.listed {
            return vec![MenuItem::inert(format!("Asking {}", Self::site(ctx)))];
        }
        if self.rooms.is_empty() {
            return vec![MenuItem::inert("No games yet")];
        }
        self.rooms
            .iter()
            .enumerate()
            .map(|(i, info)| {
                MenuItem::new(
                    room_line(&self.kind, info),
                    move |s: &mut Self, ctx: &mut Ctx<G>| {
                        s.open(ctx, i);
                    },
                )
                .help("Enter opens this game.")
            })
            .collect()
    }

    fn handle_input(&mut self, ctx: &mut Ctx<G>, event: &InputEvent) {
        if event.pressed() == Some(Key::F5) && !event.is_repeat() {
            self.list(ctx);
            ctx.say("Refreshing.");
            return;
        }
        crate::menu::menu_input(self, ctx, event);
    }

    fn update(&mut self, ctx: &mut Ctx<G>, _dt: f64) {
        for notice in ctx.take_room_notices() {
            if let Notice::Connection(c) = notice {
                ctx.say(connection_line(&Self::site(ctx), c));
            }
        }
        if let Some(ticket) = self.asking.ticket() {
            if let Some(answer) = ctx.rooms.result(ticket) {
                self.asking = Asking::Nothing;
                match answer {
                    Ok(Answer::List(rooms)) => {
                        let first = !self.listed;
                        let changed = rooms != self.rooms;
                        self.rooms = rooms;
                        self.listed = true;
                        if changed {
                            self.refresh(ctx, true);
                        }
                        if first {
                            self.speak_current(ctx);
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        ctx.play_ui(UiSound::Error);
                        ctx.say(e.spoken(&Self::site(ctx)));
                        if !self.listed {
                            self.listed = true;
                            self.refresh(ctx, true);
                        }
                    }
                }
            }
        }
        if Instant::now() >= self.next_list {
            self.list(ctx);
        }
    }

    fn lines(&self, ctx: &Ctx<G>) -> Vec<String> {
        let mut out = vec![self.menu.title.clone()];
        for (i, item) in self.menu.items.iter().enumerate() {
            let marker = if i == self.menu.index { "> " } else { "  " };
            out.push(format!("{marker}{}", item.text(self, ctx)));
        }
        out.extend(ctx.rooms.describe());
        out
    }

    fn help(&self, ctx: &Ctx<G>) -> String {
        self.current_help(ctx).unwrap_or_else(|| {
            "Your online games, newest first. Enter opens one, F5 asks for the list again, \
             Escape goes back."
                .to_string()
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use hearken_rooms::transport::{reply, Scripted};
    use serde_json::json;

    use super::*;
    use crate::online::OnlineConfig;
    use crate::state::State;
    use crate::testing::Headless;
    use crate::{Game, NoAction, NoCategory};

    struct Demo;

    impl Game for Demo {
        const TITLE: &'static str = "Demo";
        const DATA_DIR: &'static str = "hearken-rooms-screens";
        const ENV_PREFIX: &'static str = "HEARKEN_ROOMS_SCREENS";
        type Event = ();
        type Data = ();
        type Settings = ();
        type Action = NoAction;
        type Category = NoCategory;

        fn online() -> Option<OnlineConfig> {
            Some(
                OnlineConfig::new("https://example.test/api/games/demo", "example.test")
                    .with_rooms(),
            )
        }
    }

    /// The screen under the ones tested.
    struct Blank;

    impl State<Demo> for Blank {
        fn name(&self) -> &'static str {
            "blank"
        }
        fn help(&self, _ctx: &Ctx<Demo>) -> String {
            String::new()
        }
    }

    /// The game's match screen: says it opened, and which room.
    struct Playing(RoomId);

    impl State<Demo> for Playing {
        fn name(&self) -> &'static str {
            "playing"
        }
        fn enter(&mut self, ctx: &mut Ctx<Demo>) {
            ctx.say(format!("Playing {}.", self.0));
        }
        fn help(&self, _ctx: &Ctx<Demo>) -> String {
            String::new()
        }
    }

    const ID: &str = "0123456789abcdef01234567";

    fn kind() -> RoomKind<Demo> {
        RoomKind::new("pebbles", "Pebbles", &json!({ "pile": 10 }), |ctx, id| {
            ctx.push(Playing(id.clone()))
        })
    }

    fn room(phase: &str, seq: u64, members: &[(u8, &str)], you: u8) -> serde_json::Value {
        json!({
            "id": ID, "kind": "pebbles", "code": "M4K7T8", "phase": phase, "open": false,
            "seats": { "min": 2, "max": 2 }, "host": 0,
            "members": members.iter().map(|(seat, name)| json!({
                "seat": seat, "name": name, "guest": false, "here": true, "left": false
            })).collect::<Vec<_>>(),
            "you": you, "seed": if phase == "waiting" { json!(null) } else { json!("7") },
            "context": { "pile": 10 }, "seq": seq,
            "snapshot": if phase == "playing" { json!({ "ticks": 0, "finished": false, "outcome": {
                "pile": 10, "next_seat": 0, "winner": null,
                "summary": ["Your move, 10 left", "Their move, 10 left"] } }) } else { json!(null) },
            "result": null, "updated_at": 1000, "turn_started_at": null,
        })
    }

    /// A headless app with Ada's account held and a scripted server, with
    /// `screen` just opened.
    fn start(screen: impl State<Demo> + 'static) -> (Headless<Demo>, Arc<Scripted>) {
        let mut h = Headless::<Demo>::new((), Blank);
        let script = Scripted::new();
        h.app.ctx.rooms.set_transport(script.clone());
        h.app.ctx.online.hold_account("tok", "Ada");
        h.frame(0.0);
        h.clear();
        h.app.ctx.push(screen);
        (h, script)
    }

    /// Frames until `spoken` has a line containing `text`.
    fn wait_for(h: &mut Headless<Demo>, text: &str) {
        for _ in 0..400 {
            if h.spoken().iter().any(|l| l.contains(text)) {
                return;
            }
            h.frame(0.016);
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("never heard {text:?}; heard {:?}", h.spoken());
    }

    fn press(h: &mut Headless<Demo>, key: Key) {
        h.send(InputEvent::key(key));
    }

    #[test]
    fn a_game_is_created_and_its_code_spoken_then_a_player_joins_and_the_host_starts() {
        let (mut h, script) = start(OnlinePlayScreen::new(kind()));
        h.frame(0.0);
        assert!(
            h.spoken().iter().any(|l| l.contains("Pebbles online")),
            "{:?}",
            h.spoken()
        );
        assert!(
            h.spoken().iter().any(|l| l.contains("Create a game")),
            "{:?}",
            h.spoken()
        );
        h.clear();

        script.push_for(
            "rooms?kind=pebbles",
            reply(
                200,
                json!({ "ok": true, "room": room("waiting", 1, &[(0, "Ada")], 0) }),
            ),
        );
        script.push_for("?after=0", reply(200, json!({ "ok": true, "room": room("waiting", 1, &[(0, "Ada")], 0),
            "events": [{ "seq": 1, "type": "join", "at": 1, "seat": 0, "name": "Ada" }], "more": false })));
        press(&mut h, Key::Return);
        wait_for(&mut h, "Invite code M 4 K, 7 T 8");
        assert_eq!(h.app.top_name(), "room");
        let spoken = h.spoken();
        assert!(spoken.iter().any(|l| l == "Creating a game."), "{spoken:?}");
        assert!(
            spoken.iter().any(|l| l.contains("Pebbles game.")),
            "{spoken:?}"
        );
        assert!(
            !spoken.iter().any(|l| l.contains("Ada joined")),
            "own join is silent: {spoken:?}"
        );
        h.clear();

        // Bo takes the second seat: spoken, and the rows change.
        script.push_for("?after=1", reply(200, json!({ "ok": true, "room": room("waiting", 2, &[(0, "Ada"), (1, "Bo")], 0),
            "events": [{ "seq": 2, "type": "join", "at": 2, "seat": 1, "name": "Bo" }], "more": false })));
        h.app.ctx.rooms.refresh(&RoomId(ID.into()));
        wait_for(&mut h, "Bo joined.");
        let lines = h.app.top_lines();
        assert!(
            lines
                .iter()
                .any(|l| l.contains("Players: Ada (you), host; Bo. Ready to start.")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("Start the game")),
            "{lines:?}"
        );
        h.clear();

        // Down to Start, Enter: the match begins and the game's screen opens.
        press(&mut h, Key::Down);
        press(&mut h, Key::Down);
        assert!(
            h.spoken().iter().any(|l| l.starts_with("Start the game")),
            "{:?}",
            h.spoken()
        );
        script.push_for(
            "/start",
            reply(
                200,
                json!({ "ok": true, "room": room("playing", 3, &[(0, "Ada"), (1, "Bo")], 0) }),
            ),
        );
        press(&mut h, Key::Return);
        wait_for(&mut h, "The game starts.");
        wait_for(&mut h, &format!("Playing {ID}."));
        assert_eq!(h.app.top_name(), "playing");

        // Back from the match: the room shows the standing and a way in.
        h.clear();
        press(&mut h, Key::Escape);
        h.app.ctx.pop();
        h.frame(0.0);
        let lines = h.app.top_lines();
        assert!(
            lines.iter().any(|l| l.contains("Play: Your move, 10 left")),
            "{lines:?}"
        );
    }

    #[test]
    fn a_typed_code_joins_and_a_refusal_is_spoken() {
        let (mut h, script) = start(OnlinePlayScreen::new(kind()));
        h.frame(0.0);
        h.clear();
        press(&mut h, Key::Down);
        press(&mut h, Key::Return);
        assert_eq!(h.app.top_name(), "text_entry");
        for c in "m4k7t8".chars() {
            h.send(InputEvent::typed(c));
        }
        script.push_for(
            "rooms/join",
            reply(404, json!({ "ok": false, "error": "unknown-code" })),
        );
        press(&mut h, Key::Return);
        wait_for(&mut h, "No game has that code.");
        assert_eq!(h.app.top_name(), "online_play");
        let sent = script.requests.lock().unwrap().last().unwrap().2.clone();
        assert_eq!(sent, json!({ "code": "M4K7T8" }));
    }

    #[test]
    fn my_games_lists_rooms_and_opens_one() {
        let (mut h, script) = start(MyGamesScreen::new(kind()));
        script.push_for("/rooms", reply(200, json!({ "ok": true, "rooms": [
            { "id": ID, "kind": "pebbles", "code": "M4K7T8", "phase": "playing",
              "seats": { "min": 2, "max": 2 }, "host": 0, "you": 1, "seed": "7", "seq": 5, "unread": 2,
              "members": [{ "seat": 0, "name": "Bo" }, { "seat": 1, "name": "Ada" }],
              "context": {}, "snapshot": { "ticks": 2, "finished": false, "outcome": {
                  "next_seat": 1, "summary": ["Their move, 6 left", "Your move, 6 left"] } } },
            { "id": "fedcba9876543210fedcba98", "kind": "pebbles", "code": "TTTTTT", "phase": "waiting",
              "seats": { "min": 2, "max": 2 }, "host": 1, "you": 1, "seq": 1, "unread": 0,
              "members": [{ "seat": 1, "name": "Ada" }], "context": {} },
        ] })));
        h.frame(0.0);
        wait_for(&mut h, "Pebbles with Bo: Your move, 6 left, 2 new");
        let lines = h.app.top_lines();
        assert!(
            lines
                .iter()
                .any(|l| l.contains("Pebbles: waiting for 1 more")),
            "{lines:?}"
        );
        h.clear();
        press(&mut h, Key::Return);
        wait_for(&mut h, &format!("Playing {ID}."));
    }

    #[test]
    fn without_an_account_the_menu_offers_to_connect() {
        let mut h = Headless::<Demo>::new((), Blank);
        h.app.ctx.push(OnlinePlayScreen::new(kind()));
        h.frame(0.0);
        let lines = h.app.top_lines();
        assert!(
            lines
                .iter()
                .any(|l| l.contains("Connect a example.test account to play online")),
            "{lines:?}"
        );
    }
}
