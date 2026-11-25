//! Play online: the screens over the lobby (see `crate::lobby`). One
//! place, above every kind of room, for who is here, invites and friends,
//! and the notice the notice key opens.
//!
//! - `PlayOnlineScreen` ("play_online"): Who's here, Invites, Friends,
//!   Notices (while any), then each kind's own Online play menu (Create a game, Join
//!   with a code, Play with anyone, My games).
//! - `WhosHereScreen` ("whos_here") and `FriendsScreen` ("friends"): the
//!   players, each opening `LobbyPlayerScreen` ("lobby_player"): Invite,
//!   Add or Remove friend, Mute, Block.
//! - `InvitesScreen` ("invites"): invites to answer and the player's own
//!   waiting; each opens `InviteScreen` ("lobby_invite").
//! - `NoticeScreen` ("notice"): a notice's line, then Go there, Later, No.
//!
//! Every list reads `ctx.rooms.lobby()`, the last pulse, and reads it again
//! when a new pulse comes. A request on its way is one explicit state
//! (`Asking`), spoken when it is answered.

use hearken_rooms::{Answer, InviteAnswer, LobbyNotice, MuteMode, Player, RoomsError, Ticket};

use crate::ctx::Ctx;
use crate::game::Game;
use crate::lobby::{kind_label, NoticeState};
use crate::menu::{Label, Menu, MenuCore, MenuItem};
use crate::screens::online::OnlineAccountScreen;
use crate::screens::rooms::{OnlinePlayScreen, RoomKind};
use crate::sound::UiSound;

/// What a lobby screen is waiting on from the server.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Asking {
    Nothing,
    /// An invite on its way, to the player named.
    Invite(Ticket, String),
    /// An answer to an invite: accept, decline, withdraw.
    Answer(Ticket, InviteAnswer, String),
    /// Adding or removing a friend.
    Friend(Ticket, bool, String),
    /// A mute or block.
    Mute(Ticket, MuteMode, String),
    /// Joining an open table by its code.
    Join(Ticket, String),
    /// A report of a player by name.
    Report(Ticket, String),
}

impl Asking {
    fn ticket(&self) -> Option<Ticket> {
        match self {
            Asking::Nothing => None,
            Asking::Invite(t, _)
            | Asking::Answer(t, _, _)
            | Asking::Friend(t, _, _)
            | Asking::Mute(t, _, _)
            | Asking::Join(t, _)
            | Asking::Report(t, _) => Some(*t),
        }
    }
}

/// "Parlor, 3 here", with "new lines" first when someone spoke since the
/// player last looked.
fn channel_row<G: Game>(ctx: &Ctx<G>, channel: crate::lobby::Channel) -> String {
    let Some(summary) = ctx
        .rooms
        .lobby()
        .and_then(|l| l.channel(channel.kind, channel.name))
    else {
        return channel.label.to_string();
    };
    let new = summary
        .last
        .as_ref()
        .is_some_and(|l| l.seq > ctx.lobby.channel_seen);
    match (new, summary.here) {
        (true, 0) => format!("{}, new lines", channel.label),
        (true, n) => format!("{}, new lines, {n} here", channel.label),
        (false, 0) => channel.label.to_string(),
        (false, n) => format!("{}, {n} here", channel.label),
    }
}

fn kinds<G: Game>(ctx: &Ctx<G>) -> Vec<RoomKind<G>> {
    G::lobby().map(|c| (c.kinds)(ctx)).unwrap_or_default()
}

fn site<G: Game>(ctx: &Ctx<G>) -> String {
    ctx.online.site().to_string()
}

fn refused<G: Game>(ctx: &mut Ctx<G>, e: &RoomsError) {
    ctx.play_ui(UiSound::Error);
    let site = site(ctx);
    ctx.say(e.spoken(&site));
}

/// Sends `sent`, or speaks why it could not go. `Some` is the new state.
fn send<G: Game>(
    ctx: &mut Ctx<G>,
    sent: Result<Ticket, RoomsError>,
    make: impl FnOnce(Ticket) -> Asking,
) -> Option<Asking> {
    match sent {
        Ok(t) => Some(make(t)),
        Err(e) => {
            refused(ctx, &e);
            None
        }
    }
}

/// Collects `asking`'s answer when it has come, and speaks it. Returns
/// whether it came, so a list reads itself again.
fn collect<G: Game>(ctx: &mut Ctx<G>, asking: &mut Asking) -> bool {
    let Some(ticket) = asking.ticket() else {
        return false;
    };
    let Some(answer) = ctx.rooms.result(ticket) else {
        return false;
    };
    let was = std::mem::replace(asking, Asking::Nothing);
    match (was, answer) {
        (_, Err(e)) => refused(ctx, &e),
        (Asking::Invite(_, name), Ok(_)) => {
            ctx.play_ui(UiSound::Select);
            ctx.say(format!("Invite sent to {name}."));
        }
        (Asking::Answer(_, InviteAnswer::Accept, kind), Ok(Answer::Room(id))) => {
            ctx.play_ui(UiSound::Select);
            // The invite or notice screen gives way to the table.
            ctx.pop_quiet();
            ctx.go_to_room(&kind, &id);
        }
        (Asking::Answer(_, InviteAnswer::Decline, _), Ok(_)) => {
            ctx.play_ui(UiSound::Select);
            ctx.say("Invite declined.");
        }
        (Asking::Answer(_, InviteAnswer::Withdraw, _), Ok(_)) => {
            ctx.play_ui(UiSound::Select);
            ctx.say("Invite taken back.");
        }
        (Asking::Friend(_, on, name), Ok(_)) => {
            ctx.play_ui(UiSound::Select);
            ctx.say(if on {
                format!("{name} added to friends.")
            } else {
                format!("{name} removed from friends.")
            });
        }
        (Asking::Mute(_, mode, name), Ok(_)) => {
            ctx.play_ui(UiSound::Select);
            ctx.say(match mode {
                MuteMode::None => format!("{name} unmuted."),
                mode => format!("{name} {}.", mode.spoken()),
            });
        }
        (Asking::Report(_, name), Ok(_)) => {
            ctx.play_ui(UiSound::Select);
            ctx.say(format!("{name} reported and muted."));
        }
        (Asking::Join(_, kind), Ok(Answer::Room(id))) => {
            ctx.play_ui(UiSound::Select);
            ctx.pop_quiet();
            ctx.go_to_room(&kind, &id);
        }
        _ => {}
    }
    true
}

fn connect_row<S: 'static, G: Game>(ctx: &Ctx<G>) -> MenuItem<S, G> {
    let site = site(ctx);
    let label = if ctx.online.offers_guests() {
        "Play online: connect an account or play as a guest".to_string()
    } else {
        format!(
            "Connect {} {site} account to play online",
            crate::online::article(&site)
        )
    };
    MenuItem::new(label, |_: &mut S, ctx: &mut Ctx<G>| {
        ctx.push(OnlineAccountScreen::new());
    })
    .help("Online play needs an account, so other players know who they are playing.")
}

/// "Who's here, 4", "Who's here, none".
fn count_label(what: &str, n: Option<usize>) -> String {
    match n {
        Some(0) => format!("{what}, none"),
        Some(n) => format!("{what}, {n}"),
        None => what.to_string(),
    }
}

// -- Play online ---------------------------------------------------------------

/// The lobby's Play online menu: everything online, for every game.
pub struct PlayOnlineScreen<G: Game> {
    menu: MenuCore<Self, G>,
    seen: u64,
    connected: bool,
}

impl<G: Game> PlayOnlineScreen<G> {
    pub fn new() -> Self {
        PlayOnlineScreen {
            menu: MenuCore::new("Play online"),
            seen: 0,
            connected: false,
        }
    }
}

impl<G: Game> Default for PlayOnlineScreen<G> {
    fn default() -> Self {
        Self::new()
    }
}

impl<G: Game> Menu for PlayOnlineScreen<G> {
    type Game = G;

    fn name(&self) -> &'static str {
        "play_online"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn build_items(&mut self, ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        self.seen = ctx.rooms.lobby_generation();
        self.connected = ctx.online.is_connected();
        if !self.connected {
            return vec![connect_row(ctx)];
        }
        let mut items = Vec::new();
        if let Some(channel) = G::lobby().and_then(|l| l.channel) {
            items.push(
                MenuItem::new(
                    Label::dynamic(move |_: &Self, ctx: &Ctx<G>| channel_row(ctx, channel)),
                    move |_: &mut Self, ctx: &mut Ctx<G>| {
                        ctx.push(crate::screens::chat::ChatListScreen::<G>::channel(channel));
                    },
                )
                .help("Chat with everyone online. Enter opens it."),
            );
        }
        items.push(
            MenuItem::new(
                Label::dynamic(|_: &Self, ctx: &Ctx<G>| {
                    count_label("Who's here", ctx.rooms.lobby().map(|l| l.here.len()))
                }),
                |_: &mut Self, ctx: &mut Ctx<G>| ctx.push(WhosHereScreen::new()),
            )
            .help("Everyone online now, friends first, and where they are. Enter on a player invites them or adds a friend."),
        );
        items.push(
            MenuItem::new(
                Label::dynamic(|_: &Self, ctx: &Ctx<G>| {
                    count_label(
                        "Invites",
                        ctx.rooms
                            .lobby()
                            .map(|l| l.incoming().count() + l.outgoing().count()),
                    )
                }),
                |_: &mut Self, ctx: &mut Ctx<G>| ctx.push(InvitesScreen::new()),
            )
            .help("Invites to answer, and yours waiting on others."),
        );
        items.push(
            MenuItem::new(
                Label::dynamic(|_: &Self, ctx: &Ctx<G>| {
                    count_label("Friends", ctx.rooms.lobby().map(|l| l.friends.len()))
                }),
                |_: &mut Self, ctx: &mut Ctx<G>| ctx.push(FriendsScreen::new()),
            )
            .help("The players you added, here or away. Add friends from Who's here."),
        );
        // After the lists, so a notice arriving never moves the cursor.
        let kept = ctx.notices().count();
        if kept > 0 {
            items.push(
                MenuItem::new(
                    Label::dynamic(|_: &Self, ctx: &Ctx<G>| {
                        count_label("Notices", Some(ctx.notices().count()))
                    }),
                    |_: &mut Self, ctx: &mut Ctx<G>| ctx.push(NoticesScreen::new()),
                )
                .help("What happened online since you looked: Enter opens one."),
            );
        }
        for kind in kinds(ctx) {
            let label = format!("{} online", kind.label);
            items.push(
                MenuItem::new(label, move |_: &mut Self, ctx: &mut Ctx<G>| {
                    ctx.push(OnlinePlayScreen::new(kind.clone()));
                })
                .help("Create a game with a code, join with a code, play with anyone, and your games."),
            );
        }
        items
    }

    fn update(&mut self, ctx: &mut Ctx<G>, _dt: f64) {
        let fresh = ctx.rooms.lobby_generation() != self.seen;
        if fresh || ctx.online.is_connected() != self.connected {
            self.refresh(ctx, true);
        }
    }

    fn lines(&self, ctx: &Ctx<G>) -> Vec<String> {
        let core = self.menu();
        let mut out = vec![core.title.clone()];
        for (i, item) in core.items.iter().enumerate() {
            let marker = if i == core.index { "> " } else { "  " };
            out.push(format!("{marker}{}", item.text(self, ctx)));
        }
        out.extend(ctx.rooms.describe());
        out.extend(ctx.describe_notices());
        out
    }
}

// -- Players ---------------------------------------------------------------------

/// Which players a list shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Players {
    Here,
    Friends,
}

fn player_list<G: Game>(ctx: &Ctx<G>, which: Players) -> Vec<Player> {
    let Some(lobby) = ctx.rooms.lobby() else {
        return Vec::new();
    };
    match which {
        Players::Here => lobby.here.clone(),
        Players::Friends => lobby.friends.clone(),
    }
}

fn player_items<S: 'static, G: Game>(ctx: &Ctx<G>, which: Players) -> Vec<MenuItem<S, G>> {
    let players = player_list(ctx, which);
    if ctx.rooms.lobby().is_none() {
        return vec![MenuItem::inert(format!("Asking {}.", site(ctx)))];
    }
    if players.is_empty() {
        return vec![MenuItem::inert(match which {
            Players::Here => "Nobody else is here.",
            Players::Friends => "No friends yet. Add them from Who's here.",
        })];
    }
    players
        .into_iter()
        .map(|p| {
            let mut line = p.line();
            if which == Players::Here && p.friend {
                line.push_str(", friend");
            }
            MenuItem::new(line, move |_: &mut S, ctx: &mut Ctx<G>| {
                ctx.push(LobbyPlayerScreen::new(p.clone()));
            })
        })
        .collect()
}

macro_rules! player_list_screen {
    ($ty:ident, $name:literal, $title:literal, $which:expr, $help:literal) => {
        pub struct $ty<G: Game> {
            menu: MenuCore<Self, G>,
            seen: u64,
        }

        impl<G: Game> $ty<G> {
            pub fn new() -> Self {
                $ty {
                    menu: MenuCore::new($title).with_intro_help($help),
                    seen: 0,
                }
            }
        }

        impl<G: Game> Default for $ty<G> {
            fn default() -> Self {
                Self::new()
            }
        }

        impl<G: Game> Menu for $ty<G> {
            type Game = G;

            fn name(&self) -> &'static str {
                $name
            }

            fn menu(&self) -> &MenuCore<Self, G> {
                &self.menu
            }

            fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
                &mut self.menu
            }

            fn build_items(&mut self, ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
                self.seen = ctx.rooms.lobby_generation();
                player_items(ctx, $which)
            }

            fn update(&mut self, ctx: &mut Ctx<G>, _dt: f64) {
                if ctx.rooms.lobby_generation() != self.seen {
                    self.refresh(ctx, true);
                }
            }

            fn help(&self, _ctx: &Ctx<G>) -> String {
                $help.to_string()
            }
        }
    };
}

player_list_screen!(
    WhosHereScreen,
    "whos_here",
    "Who's here",
    Players::Here,
    "Everyone online now, friends first. Enter on a player invites them, adds a friend, or mutes them."
);
player_list_screen!(
    FriendsScreen,
    "friends",
    "Friends",
    Players::Friends,
    "The players you added, here or away. Enter on a friend invites them or removes them. An invite to a friend who is away waits for them."
);

/// One player: invite them, add or remove them as a friend, mute or block.
pub struct LobbyPlayerScreen<G: Game> {
    player: Player,
    menu: MenuCore<Self, G>,
    asking: Asking,
    seen: u64,
    /// Whether the player has them muted or blocked, once read.
    mode: MuteMode,
    mutes: MutesRead,
}

/// Reading the player's mutes, so Mute and Block can offer the way back.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MutesRead {
    NotYet,
    Asking(Ticket),
    Read,
}

impl<G: Game> LobbyPlayerScreen<G> {
    pub fn new(player: Player) -> Self {
        LobbyPlayerScreen {
            menu: MenuCore::new(hearken_rooms::clean_text(&player.name, 64)),
            player,
            asking: Asking::Nothing,
            seen: 0,
            mode: MuteMode::None,
            mutes: MutesRead::NotYet,
        }
    }

    fn busy(&self, ctx: &mut Ctx<G>) -> bool {
        if self.asking != Asking::Nothing {
            ctx.say(format!("Still asking {}.", site(ctx)));
            return true;
        }
        false
    }

    fn set_mute(&mut self, ctx: &mut Ctx<G>, mode: MuteMode) {
        let name = self.player.name.clone();
        let sent = ctx.rooms.set_mute(&self.player.handle, mode);
        if let Some(a) = send(ctx, sent, |t| Asking::Mute(t, mode, name)) {
            self.asking = a;
        }
    }
}

impl<G: Game> Menu for LobbyPlayerScreen<G> {
    type Game = G;

    fn name(&self) -> &'static str {
        "lobby_player"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn build_items(&mut self, ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        self.seen = ctx.rooms.lobby_generation();
        let mut items = Vec::new();
        let name = hearken_rooms::clean_text(&self.player.name, 64);
        for kind in kinds(ctx) {
            items.push(
                MenuItem::new(
                    format!("Invite to {}", kind.label),
                    move |s: &mut Self, ctx: &mut Ctx<G>| {
                        if s.busy(ctx) {
                            return;
                        }
                        let name = s.player.name.clone();
                        let sent = ctx
                            .rooms
                            .invite(&s.player.handle, &kind.kind, &kind.context);
                        if let Some(a) = send(ctx, sent, |t| Asking::Invite(t, name)) {
                            s.asking = a;
                        }
                    },
                )
                .help(format!(
                    "{name} hears the invite wherever they are. If they say yes, the game starts."
                )),
            );
        }
        let friend = ctx
            .rooms
            .lobby()
            .is_some_and(|l| l.is_friend(&self.player.handle));
        items.push(
            MenuItem::new(
                if friend {
                    "Remove friend"
                } else {
                    "Add friend"
                },
                move |s: &mut Self, ctx: &mut Ctx<G>| {
                    if s.busy(ctx) {
                        return;
                    }
                    let name = s.player.name.clone();
                    let sent = ctx.rooms.set_friend(&s.player.handle, !friend);
                    if let Some(a) = send(ctx, sent, |t| Asking::Friend(t, !friend, name)) {
                        s.asking = a;
                    }
                },
            )
            .help("Friends are listed first, and you hear when they come in. They are not asked."),
        );
        let rows = [
            match self.mode {
                MuteMode::Mute => ("Unmute", MuteMode::None, "Their chat lines and invites reach you again."),
                _ => ("Mute", MuteMode::Mute, "Stops their chat lines and invites reaching you. They are not told."),
            },
            match self.mode {
                MuteMode::Block => ("Unblock", MuteMode::None, "You and they can reach each other again."),
                _ => ("Block", MuteMode::Block, "You stop seeing each other, and you are never seated together. They are not told. Settings, Online has the list of blocked players."),
            },
        ];
        for (label, mode, help) in rows {
            items.push(
                MenuItem::new(label, move |s: &mut Self, ctx: &mut Ctx<G>| {
                    if s.busy(ctx) {
                        return;
                    }
                    if mode == MuteMode::Block {
                        // A block hides them everywhere: asked first.
                        let name = hearken_rooms::clean_text(&s.player.name, 64);
                        ctx.push(crate::screens::confirm::ConfirmScreen::<G>::new(
                            format!("Block {name}?"),
                            |ctx| ctx.pop_with("block"),
                        ));
                        return;
                    }
                    s.set_mute(ctx, mode);
                })
                .help(help),
            );
        }
        let site_name = site(ctx);
        items.push(
            MenuItem::new("Report", |s: &mut Self, ctx: &mut Ctx<G>| {
                if s.busy(ctx) {
                    return;
                }
                let name = hearken_rooms::clean_text(&s.player.name, 64);
                let site = site(ctx);
                ctx.push(crate::screens::confirm::ConfirmScreen::<G>::new(
                    format!("Report {name}'s name to {site}?"),
                    |ctx| ctx.pop_with("report"),
                ));
            })
            .help(format!("Sends this player's name to {site_name}, for a name that should not be here, and mutes them for you.")),
        );
        items
    }

    fn resume_with(&mut self, ctx: &mut Ctx<G>, result: &str) {
        match result {
            "block" => self.set_mute(ctx, MuteMode::Block),
            "report" => {
                let name = self.player.name.clone();
                let sent = ctx.rooms.report_player(&self.player.handle, "");
                if let Some(a) = send(ctx, sent, |t| Asking::Report(t, name)) {
                    self.asking = a;
                }
            }
            "" => self.announce(ctx),
            other => {
                ctx.say(other.to_string());
                self.refresh(ctx, true);
            }
        }
    }

    fn update(&mut self, ctx: &mut Ctx<G>, _dt: f64) {
        match self.mutes {
            MutesRead::NotYet => {
                self.mutes = ctx.rooms.mutes().map_or(MutesRead::Read, MutesRead::Asking);
            }
            MutesRead::Asking(ticket) => {
                if let Some(answer) = ctx.rooms.result(ticket) {
                    self.mutes = MutesRead::Read;
                    if let Ok(Answer::Mutes(mutes)) = answer {
                        let handle = &self.player.handle;
                        let mode = mutes
                            .iter()
                            .find(|m| &m.handle == handle)
                            .map_or(MuteMode::None, |m| m.mode);
                        if mode != self.mode {
                            self.mode = mode;
                            self.refresh(ctx, true);
                        }
                    }
                }
            }
            MutesRead::Read => {}
        }
        let muting = matches!(self.asking, Asking::Mute(..) | Asking::Report(..));
        let came = collect(ctx, &mut self.asking);
        if came && muting {
            // Read them again: the rows offer the way back.
            self.mutes = MutesRead::NotYet;
        }
        // A friend change shows once the pulse it asks for comes back.
        if came || ctx.rooms.lobby_generation() != self.seen {
            self.refresh(ctx, true);
        }
    }

    fn help(&self, ctx: &Ctx<G>) -> String {
        self.current_help(ctx)
            .unwrap_or_else(|| "Invite this player, add them as a friend, or mute them.".into())
    }
}

// -- Invites -------------------------------------------------------------------

/// Invites to answer, then the player's own waiting on others.
pub struct InvitesScreen<G: Game> {
    menu: MenuCore<Self, G>,
    seen: u64,
}

impl<G: Game> InvitesScreen<G> {
    pub fn new() -> Self {
        InvitesScreen {
            menu: MenuCore::new("Invites"),
            seen: 0,
        }
    }
}

impl<G: Game> Default for InvitesScreen<G> {
    fn default() -> Self {
        Self::new()
    }
}

impl<G: Game> Menu for InvitesScreen<G> {
    type Game = G;

    fn name(&self) -> &'static str {
        "invites"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn build_items(&mut self, ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        self.seen = ctx.rooms.lobby_generation();
        let Some(lobby) = ctx.rooms.lobby().cloned() else {
            return vec![MenuItem::inert(format!("Asking {}.", site(ctx)))];
        };
        let kinds = kinds(ctx);
        let mut items = Vec::new();
        for invite in lobby.incoming() {
            ctx.rooms.mark_heard(&invite.id);
            let line = format!(
                "{} invites you. {}.",
                hearken_rooms::clean_text(&invite.from.name, 64),
                kind_label(&kinds, &invite.kind)
            );
            let invite = invite.clone();
            items.push(MenuItem::new(
                line,
                move |_: &mut Self, ctx: &mut Ctx<G>| {
                    ctx.push(InviteScreen::new(invite.clone()));
                },
            ));
        }
        for invite in lobby.outgoing() {
            let line = format!(
                "You invited {}. {}, waiting.",
                hearken_rooms::clean_text(&invite.to.name, 64),
                kind_label(&kinds, &invite.kind)
            );
            let invite = invite.clone();
            items.push(MenuItem::new(
                line,
                move |_: &mut Self, ctx: &mut Ctx<G>| {
                    ctx.push(InviteScreen::new(invite.clone()));
                },
            ));
        }
        if items.is_empty() {
            items.push(MenuItem::inert("No invites."));
        }
        items
    }

    fn update(&mut self, ctx: &mut Ctx<G>, _dt: f64) {
        if ctx.rooms.lobby_generation() != self.seen {
            self.refresh(ctx, true);
        }
    }

    fn help(&self, _ctx: &Ctx<G>) -> String {
        "Enter on an invite to you accepts or declines it. Enter on one of yours takes it back. An invite nobody answers ends after 3 days.".into()
    }
}

/// One invite: Accept and Decline, or Take back for the player's own.
pub struct InviteScreen<G: Game> {
    invite: hearken_rooms::Invite,
    menu: MenuCore<Self, G>,
    asking: Asking,
}

impl<G: Game> InviteScreen<G> {
    pub fn new(invite: hearken_rooms::Invite) -> Self {
        InviteScreen {
            menu: MenuCore::new(if invite.mine {
                format!(
                    "Invite to {}",
                    hearken_rooms::clean_text(&invite.to.name, 64)
                )
            } else {
                format!(
                    "Invite from {}",
                    hearken_rooms::clean_text(&invite.from.name, 64)
                )
            }),
            invite,
            asking: Asking::Nothing,
        }
    }

    fn answer(&mut self, ctx: &mut Ctx<G>, answer: InviteAnswer) {
        if self.asking != Asking::Nothing {
            ctx.say(format!("Still asking {}.", site(ctx)));
            return;
        }
        let kind = self.invite.kind.clone();
        let sent = ctx.rooms.answer_invite(&self.invite.id, answer);
        if let Some(a) = send(ctx, sent, |t| Asking::Answer(t, answer, kind)) {
            self.asking = a;
        }
    }
}

impl<G: Game> Menu for InviteScreen<G> {
    type Game = G;

    fn name(&self) -> &'static str {
        "lobby_invite"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn build_items(&mut self, _ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        if self.invite.mine {
            return vec![MenuItem::new(
                "Take back",
                |s: &mut Self, ctx: &mut Ctx<G>| s.answer(ctx, InviteAnswer::Withdraw),
            )];
        }
        vec![
            MenuItem::new("Accept", |s: &mut Self, ctx: &mut Ctx<G>| {
                s.answer(ctx, InviteAnswer::Accept)
            })
            .help("The game starts, and you go to the table."),
            MenuItem::new("Decline", |s: &mut Self, ctx: &mut Ctx<G>| {
                s.answer(ctx, InviteAnswer::Decline)
            }),
        ]
    }

    fn update(&mut self, ctx: &mut Ctx<G>, _dt: f64) {
        let accepting = matches!(self.asking, Asking::Answer(_, InviteAnswer::Accept, _));
        if collect(ctx, &mut self.asking) && !accepting {
            ctx.pop_quiet();
        }
    }

    fn help(&self, ctx: &Ctx<G>) -> String {
        self.current_help(ctx)
            .unwrap_or_else(|| "Enter chooses. Escape goes back.".into())
    }
}

// -- Notices ---------------------------------------------------------------------

/// The notices of this session not yet over, newest first.
pub struct NoticesScreen<G: Game> {
    menu: MenuCore<Self, G>,
}

impl<G: Game> NoticesScreen<G> {
    pub fn new() -> Self {
        NoticesScreen {
            menu: MenuCore::new("Notices"),
        }
    }
}

impl<G: Game> Default for NoticesScreen<G> {
    fn default() -> Self {
        Self::new()
    }
}

impl<G: Game> Menu for NoticesScreen<G> {
    type Game = G;

    fn name(&self) -> &'static str {
        "notices"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn build_items(&mut self, ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        let mut items: Vec<MenuItem<Self, G>> = ctx
            .notices()
            .filter(|n| matches!(n.state, NoticeState::Said | NoticeState::Kept))
            .map(|n| {
                let id = n.id;
                MenuItem::new(n.line.clone(), move |_: &mut Self, ctx: &mut Ctx<G>| {
                    ctx.push(NoticeScreen::new(id));
                })
            })
            .collect();
        items.reverse();
        if items.is_empty() {
            items.push(MenuItem::inert("No notices."));
        }
        items
    }

    fn help(&self, _ctx: &Ctx<G>) -> String {
        "Enter on a notice goes there, or puts it off.".into()
    }
}

/// Where Go there leads for a notice.
#[derive(Clone, Debug, PartialEq)]
enum Destination {
    /// Accept the invite: the match starts.
    Accept(String, String),
    /// A room of the player's.
    Room(String, hearken_rooms::RoomId),
    /// Join an open table by its code.
    Join(String, String),
    /// A friend: their own menu.
    Player(Player),
    /// Play online.
    Lobby,
    /// The game's chat channel.
    Channel,
}

/// A notice: its line, then Go there, Later or No. Escape is Later.
pub struct NoticeScreen<G: Game> {
    id: u64,
    menu: MenuCore<Self, G>,
    asking: Asking,
    to: Option<Destination>,
}

impl<G: Game> NoticeScreen<G> {
    pub fn new(id: u64) -> Self {
        NoticeScreen {
            id,
            menu: MenuCore::new("Notice"),
            asking: Asking::Nothing,
            to: None,
        }
    }

    fn go(&mut self, ctx: &mut Ctx<G>) {
        if self.asking != Asking::Nothing {
            ctx.say(format!("Still asking {}.", site(ctx)));
            return;
        }
        let Some(to) = self.to.clone() else {
            return;
        };
        match to {
            Destination::Accept(id, kind) => {
                let sent = ctx.rooms.answer_invite(&id, InviteAnswer::Accept);
                if let Some(a) = send(ctx, sent, |t| Asking::Answer(t, InviteAnswer::Accept, kind))
                {
                    self.asking = a;
                }
            }
            Destination::Join(code, kind) => {
                let sent = ctx.rooms.join(&code);
                if let Some(a) = send(ctx, sent, |t| Asking::Join(t, kind)) {
                    self.asking = a;
                }
            }
            Destination::Room(kind, room) => {
                ctx.settle_notice(self.id, false);
                ctx.pop_quiet();
                ctx.go_to_room(&kind, &room);
            }
            Destination::Player(player) => {
                ctx.settle_notice(self.id, false);
                ctx.pop_quiet();
                ctx.push(LobbyPlayerScreen::new(player));
            }
            Destination::Lobby => {
                ctx.settle_notice(self.id, false);
                ctx.pop_quiet();
                ctx.push(PlayOnlineScreen::new());
            }
            Destination::Channel => {
                ctx.settle_notice(self.id, false);
                ctx.pop_quiet();
                if let Some(channel) = G::lobby().and_then(|l| l.channel) {
                    ctx.push(crate::screens::chat::ChatListScreen::<G>::channel(channel));
                }
            }
        }
    }

    fn later(&mut self, ctx: &mut Ctx<G>) {
        ctx.settle_notice(self.id, true);
        ctx.play_ui(UiSound::Back);
        ctx.pop();
    }

    fn no(&mut self, ctx: &mut Ctx<G>) {
        ctx.settle_notice(self.id, false);
        ctx.play_ui(UiSound::Back);
        if let Some(Destination::Accept(id, _)) = &self.to {
            // No to an invite declines it.
            match ctx.rooms.answer_invite(id, InviteAnswer::Decline) {
                Ok(_) => ctx.pop_with("Invite declined."),
                Err(e) => {
                    let site = site(ctx);
                    ctx.pop_with(e.spoken(&site));
                }
            }
            return;
        }
        ctx.pop();
    }
}

impl<G: Game> Menu for NoticeScreen<G> {
    type Game = G;

    fn name(&self) -> &'static str {
        "notice"
    }

    fn menu(&self) -> &MenuCore<Self, G> {
        &self.menu
    }

    fn menu_mut(&mut self) -> &mut MenuCore<Self, G> {
        &mut self.menu
    }

    fn build_items(&mut self, ctx: &mut Ctx<G>) -> Vec<MenuItem<Self, G>> {
        let Some(notice) = ctx.notice(self.id).cloned() else {
            return vec![MenuItem::inert("That notice is over.")];
        };
        self.menu.title = notice.line.clone();
        self.to = Some(match &notice.notice {
            LobbyNotice::Invited(invite) => {
                Destination::Accept(invite.id.clone(), invite.kind.clone())
            }
            LobbyNotice::SatDown { room, kind, .. } | LobbyNotice::YourMove { room, kind, .. } => {
                Destination::Room(kind.clone(), room.clone())
            }
            LobbyNotice::Answered(invite) => match &invite.room {
                Some(room) => Destination::Room(invite.kind.clone(), room.clone()),
                None => Destination::Lobby,
            },
            LobbyNotice::OpenTable(table) => {
                Destination::Join(table.code.clone(), table.kind.clone())
            }
            LobbyNotice::FriendHere(player) => Destination::Player(player.clone()),
            LobbyNotice::Arrived { .. } | LobbyNotice::Standing(_) => Destination::Lobby,
            LobbyNotice::ChannelLine { .. } => Destination::Channel,
        });
        let no_help = if matches!(self.to, Some(Destination::Accept(..))) {
            "Declines the invite."
        } else {
            "Clears the notice."
        };
        vec![
            MenuItem::new("Go there", |s: &mut Self, ctx: &mut Ctx<G>| s.go(ctx)),
            MenuItem::new("Later", |s: &mut Self, ctx: &mut Ctx<G>| s.later(ctx))
                .help("Keeps it in Play online, under Notices."),
            MenuItem::new("No", |s: &mut Self, ctx: &mut Ctx<G>| s.no(ctx)).help(no_help),
        ]
    }

    fn update(&mut self, ctx: &mut Ctx<G>, _dt: f64) {
        let going = self.asking != Asking::Nothing;
        let came = collect(ctx, &mut self.asking);
        if going && came {
            ctx.settle_notice(self.id, false);
        }
    }

    fn on_escape(&mut self, ctx: &mut Ctx<G>) {
        self.later(ctx);
    }

    fn help(&self, ctx: &Ctx<G>) -> String {
        self.current_help(ctx).unwrap_or_else(|| {
            "Go there opens it. Later keeps it in Play online. No clears it.".into()
        })
    }
}
