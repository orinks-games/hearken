//! The lobby in every game: notices of what happens online wherever the
//! player is, one key that reaches the newest, and the Play online screens
//! (`screens::lobby`).
//!
//! A game opts in with [`Game::lobby`], naming its kinds of room, where the
//! player is, when notices must wait, and the notice key. The engine then
//! keeps the pulse going (`hearken_rooms::lobby`), turns what changed into
//! notices, and speaks each once with a cue, behind whatever is being said.
//!
//! A notice is an explicit state machine ([`NoticeState`]), changed only in
//! [`Ctx::set_notice_state`], which logs `[notice] 3 invited: waiting ->
//! said` and keeps it in `ctx.changes` for the agent tools:
//!
//! | State | Means | Leaves when |
//! |---|---|---|
//! | Waiting | Arrived, not said yet | Nothing blocks it: Said. A timed run is on: Held |
//! | Held | Kept back for a timed run | The game lets it through (results, pause, title): Said |
//! | Said | Cue and line given; the notice key reaches it | Go there or No: Done. Later, or a newer notice: Kept |
//! | Kept | Listed in Play online | Acted on there, or no longer true on the server: Done |
//! | Done | Over | Dropped |

use hearken_rooms::{InviteState, Lobby, LobbyNotice, RoomId, Whereabouts};
use serde::{Deserialize, Serialize};

use crate::bindings::Chord;
use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::InputEvent;
use crate::screens::rooms::RoomKind;
use crate::sound::UiSound;

/// Takes the player to a match: see [`LobbyConfig::go`].
pub type GoToRoom<G> = fn(&mut Ctx<G>, &RoomKind<G>, &RoomId);

/// What the engine needs from a game for the lobby.
pub struct LobbyConfig<G: Game> {
    /// The one key that reaches the newest notice, from any screen not
    /// taking typed text. Pick a key no screen of the game uses. `None`:
    /// no key; the game calls [`Ctx::open_notice`] itself.
    pub key: Option<Chord>,
    /// The kinds of room a player can be invited to, with how each opens.
    pub kinds: fn(&Ctx<G>) -> Vec<RoomKind<G>>,
    /// Where the player is, for the pulse, given the top screen's name.
    pub whereabouts: fn(&Ctx<G>, &str) -> Whereabouts,
    /// Whether notices wait, given the top screen's name: during a timed
    /// run, so nothing talks over play. They are said once this is false.
    pub hold: fn(&Ctx<G>, &str) -> bool,
    /// Takes the player to a match of `kind`: by default the Room screen
    /// pushed over wherever they are, then the game's match screen. A game
    /// that must leave where the player is first (asking, if a run is on)
    /// does that here.
    pub go: Option<GoToRoom<G>>,
}

impl<G: Game> LobbyConfig<G> {
    /// Notices with no key, the player always here, never held.
    pub fn new(kinds: fn(&Ctx<G>) -> Vec<RoomKind<G>>) -> Self {
        LobbyConfig {
            key: None,
            kinds,
            whereabouts: |_, _| Whereabouts::here(),
            hold: |_, _| false,
            go: None,
        }
    }

    pub fn key(mut self, key: Chord) -> Self {
        self.key = Some(key);
        self
    }

    pub fn whereabouts(mut self, f: fn(&Ctx<G>, &str) -> Whereabouts) -> Self {
        self.whereabouts = f;
        self
    }

    pub fn hold(mut self, f: fn(&Ctx<G>, &str) -> bool) -> Self {
        self.hold = f;
        self
    }

    pub fn go(mut self, f: GoToRoom<G>) -> Self {
        self.go = Some(f);
        self
    }
}

/// Which notices reach the player in a game: their setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoticeLevel {
    /// Every notice.
    #[default]
    All,
    /// Only the player's own games, invites from friends, and friends
    /// arriving.
    Mine,
    /// None. Play online still lists everything.
    Off,
}

impl NoticeLevel {
    pub const ALL: [NoticeLevel; 3] = [NoticeLevel::All, NoticeLevel::Mine, NoticeLevel::Off];

    pub fn spoken(self) -> &'static str {
        match self {
            NoticeLevel::All => "all",
            NoticeLevel::Mine => "friends and my games",
            NoticeLevel::Off => "off",
        }
    }

    pub fn stepped(self, delta: i32) -> NoticeLevel {
        let at = NoticeLevel::ALL.iter().position(|l| *l == self).unwrap_or(0) as i32;
        let n = NoticeLevel::ALL.len() as i32;
        NoticeLevel::ALL[((at + delta).rem_euclid(n)) as usize]
    }

    pub fn is_default(&self) -> bool {
        *self == NoticeLevel::default()
    }
}

/// Where a notice is in its life (see the module's table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeState {
    Waiting,
    Held,
    Said,
    Kept,
    Done,
}

impl NoticeState {
    pub fn name(self) -> &'static str {
        match self {
            NoticeState::Waiting => "waiting",
            NoticeState::Held => "held",
            NoticeState::Said => "said",
            NoticeState::Kept => "kept",
            NoticeState::Done => "done",
        }
    }

    /// The changes a notice may make. Anything else is a bug.
    fn allows(self, to: NoticeState) -> bool {
        use NoticeState::*;
        matches!(
            (self, to),
            (Waiting, Held | Said | Done)
                | (Held, Said | Done)
                | (Said, Kept | Done)
                | (Kept, Done)
        )
    }
}

/// One notice: what happened, the line for it, and where it stands.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub id: u64,
    pub notice: LobbyNotice,
    pub line: String,
    pub state: NoticeState,
}

/// The notices of this session, at `ctx.lobby`.
#[derive(Debug, Default)]
pub(crate) struct LobbyState {
    pub(crate) notices: Vec<Notice>,
    next: u64,
    /// The room the player was last sent to, and when: its moves need no
    /// notice while the table opens.
    going: Option<(RoomId, std::time::Instant)>,
}

/// How long a room the player was sent to counts as theirs before its
/// table is open.
const GOING_FOR: std::time::Duration = std::time::Duration::from_secs(30);

/// The most notices kept at once; the oldest done ones go first.
const MAX_NOTICES: usize = 24;

/// Screens that keep the pulse quick while open.
pub(crate) const LOBBY_SCREENS: &[&str] = &[
    "play_online",
    "whos_here",
    "lobby_player",
    "invites",
    "lobby_invite",
    "friends",
    "notices",
    "notice",
];

/// The kind's name as the player hears it, from the game's kinds.
pub(crate) fn kind_label<G: Game>(kinds: &[RoomKind<G>], kind: &str) -> String {
    kinds
        .iter()
        .find(|k| k.kind == kind)
        .map_or_else(|| kind.to_string(), |k| k.label.clone())
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// The line a notice is said with. Short: what happened, then the game.
pub fn notice_line(notice: &LobbyNotice, label: impl Fn(&str) -> String) -> String {
    let clean = |s: &str| hearken_rooms::clean_text(s, 64);
    match notice {
        LobbyNotice::Arrived { waiting, here } => {
            let mut parts = Vec::new();
            if *waiting > 0 {
                let verb = if *waiting == 1 { "waits" } else { "wait" };
                parts.push(format!("{} {verb} on you.", plural(*waiting, "game", "games")));
            }
            if *here > 0 {
                parts.push(format!("{} here.", plural(*here, "player", "players")));
            }
            parts.join(" ")
        }
        LobbyNotice::SatDown { kind, name, .. } => {
            format!("{} sat down. {}.", clean(name), label(kind))
        }
        LobbyNotice::YourMove { kind, name, .. } if !name.is_empty() => {
            format!("Your move. {}, {}.", label(kind), clean(name))
        }
        LobbyNotice::YourMove { kind, .. } => format!("Your move. {}.", label(kind)),
        LobbyNotice::Invited(invite) => {
            format!("{} invites you. {}.", clean(&invite.from.name), label(&invite.kind))
        }
        LobbyNotice::Answered(invite) => {
            let name = clean(&invite.to.name);
            let game = label(&invite.kind);
            match invite.state {
                InviteState::Accepted => format!("{name} said yes. {game}."),
                InviteState::Declined => format!("{name} said no. {game}."),
                _ => format!("No answer from {name}. {game}."),
            }
        }
        LobbyNotice::FriendHere(player) => format!("{} is here.", clean(&player.name)),
        LobbyNotice::OpenTable(table) => match &table.host {
            Some(host) => format!("Open table. {}, {}.", label(&table.kind), clean(host)),
            None => format!("Open table. {}.", label(&table.kind)),
        },
    }
}

/// Whether what a notice says is still so, by the latest pulse.
fn still_true(notice: &LobbyNotice, lobby: &Lobby) -> bool {
    match notice {
        LobbyNotice::Invited(invite) => lobby.incoming().any(|i| i.id == invite.id),
        LobbyNotice::YourMove { room, .. } => lobby.my_moves().any(|r| &r.id == room),
        LobbyNotice::SatDown { room, .. } => lobby.rooms.iter().any(|r| &r.id == room),
        LobbyNotice::Answered(invite) => match &invite.room {
            Some(room) => lobby.rooms.iter().any(|r| &r.id == room),
            None => true,
        },
        LobbyNotice::FriendHere(player) => lobby
            .friends
            .iter()
            .any(|f| f.handle == player.handle && f.state != hearken_rooms::Presence::Away),
        LobbyNotice::OpenTable(table) => lobby.open.iter().any(|t| t.room == table.room),
        LobbyNotice::Arrived { .. } => true,
    }
}

impl<G: Game> Ctx<G> {
    /// The one place a notice changes state: logged, and kept in
    /// `changes` for the agent tools.
    pub(crate) fn set_notice_state(&mut self, id: u64, to: NoticeState) -> bool {
        let Some(n) = self.lobby.notices.iter_mut().find(|n| n.id == id) else {
            return false;
        };
        let from = n.state;
        if from == to {
            return false;
        }
        if !from.allows(to) {
            log::warn!("[notice] {id} {}: no change from {} to {}", n.notice.name(), from.name(), to.name());
            return false;
        }
        n.state = to;
        let line = format!("[notice] {id} {}: {} -> {}", n.notice.name(), from.name(), to.name());
        log::info!("{line}");
        self.note_change(line);
        true
    }

    /// This session's notices that have not ended, oldest first.
    pub fn notices(&self) -> impl Iterator<Item = &Notice> {
        self.lobby
            .notices
            .iter()
            .filter(|n| n.state != NoticeState::Done)
    }

    /// The newest notice said, which the notice key reaches; else the
    /// newest kept one.
    pub fn latest_notice(&self) -> Option<&Notice> {
        let newest = |state| {
            self.lobby
                .notices
                .iter()
                .rev()
                .find(move |n: &&Notice| n.state == state)
        };
        newest(NoticeState::Said).or_else(|| newest(NoticeState::Kept))
    }

    /// Opens the newest notice: its line again, then Go there, Later or No.
    /// Does nothing with no notice. For a game with its own key for it.
    pub fn open_notice(&mut self) -> bool {
        let Some(id) = self.latest_notice().map(|n| n.id) else {
            return false;
        };
        self.push(crate::screens::lobby::NoticeScreen::new(id));
        true
    }

    /// A key press, against the game's notice key: `true` when it was the
    /// key and there was a notice to open.
    pub(crate) fn notice_key(&mut self, key: Option<Chord>, event: &InputEvent) -> bool {
        let InputEvent::KeyDown {
            key: pressed,
            mods,
            repeat: false,
            ..
        } = event
        else {
            return false;
        };
        let Some(chord) = key else {
            return false;
        };
        if chord.key != *pressed || chord.mods != *mods {
            return false;
        }
        self.open_notice()
    }

    /// Keeps the pulse going and turns what it brought into notices, said
    /// when the game lets them through. Once a frame, after the screens.
    pub(crate) fn update_lobby(&mut self, top: &'static str) {
        let Some(config) = G::lobby() else {
            return;
        };
        let on = self.rooms.is_offered() && self.online.is_connected();
        self.rooms.set_pulse(on);
        if !on {
            return;
        }
        let whereabouts = (config.whereabouts)(self, top);
        self.rooms.set_whereabouts(whereabouts);
        self.rooms.set_visibility(self.settings.online.show);
        self.rooms.set_pulse_fast(LOBBY_SCREENS.contains(&top));

        let fresh = self.rooms.take_lobby_notices();
        if !fresh.is_empty() {
            let kinds = (config.kinds)(self);
            let level = self.settings.online.notices;
            for notice in fresh {
                self.take_notice(notice, &kinds, level);
            }
            // What the server no longer says is over.
            if let Some(lobby) = self.rooms.lobby().cloned() {
                let over: Vec<u64> = self
                    .lobby
                    .notices
                    .iter()
                    .filter(|n| matches!(n.state, NoticeState::Said | NoticeState::Kept))
                    .filter(|n| !still_true(&n.notice, &lobby))
                    .map(|n| n.id)
                    .collect();
                for id in over {
                    self.set_notice_state(id, NoticeState::Done);
                }
            }
        }

        let held = (config.hold)(self, top);
        let ready: Vec<(u64, NoticeState, bool)> = self
            .lobby
            .notices
            .iter()
            .filter(|n| matches!(n.state, NoticeState::Waiting | NoticeState::Held))
            .map(|n| (n.id, n.state, self.at_its_table(&n.notice)))
            .collect();
        for (id, state, there) in ready {
            if there {
                // The player sat down at it since the notice came.
                self.set_notice_state(id, NoticeState::Done);
            } else if held {
                if state == NoticeState::Waiting {
                    self.set_notice_state(id, NoticeState::Held);
                }
            } else {
                self.say_notice(id);
            }
        }
        // Done notices beyond the cap go.
        while self.lobby.notices.len() > MAX_NOTICES {
            match self
                .lobby
                .notices
                .iter()
                .position(|n| n.state == NoticeState::Done)
            {
                Some(at) => {
                    self.lobby.notices.remove(at);
                }
                None => {
                    self.lobby.notices.remove(0);
                }
            }
        }
        self.lobby.notices.retain(|n| n.state != NoticeState::Done);
    }

    /// A table the player is at speaks for itself: its moves and seats
    /// need no notice.
    fn at_its_table(&self, notice: &LobbyNotice) -> bool {
        let Some(room) = notice.room() else {
            return false;
        };
        let going = self
            .lobby
            .going
            .as_ref()
            .is_some_and(|(id, at)| id == room && at.elapsed() < GOING_FOR);
        matches!(notice, LobbyNotice::YourMove { .. } | LobbyNotice::SatDown { .. })
            && (going || self.rooms.room(room).is_some_and(|r| r.is_watched()))
    }

    fn take_notice(&mut self, notice: LobbyNotice, kinds: &[RoomKind<G>], level: NoticeLevel) {
        if self.at_its_table(&notice) {
            return;
        }
        let mut line = notice_line(&notice, |k| kind_label(kinds, k));
        if let LobbyNotice::Arrived { .. } = notice {
            // The first time online, once ever: the player is listed.
            if !self.settings.online.shown_told
                && self.settings.online.show != hearken_rooms::Visibility::Nobody
            {
                self.settings.online.shown_told = true;
                self.save_settings();
                if !line.is_empty() {
                    line.push(' ');
                }
                line.push_str("Other players can see you are here. Settings, Online changes it.");
            }
        }
        let wanted = match level {
            NoticeLevel::All => true,
            NoticeLevel::Mine => self
                .rooms
                .lobby()
                .is_none_or(|lobby| notice.is_mine_or_friends(lobby)),
            NoticeLevel::Off => false,
        };
        if line.is_empty() || !wanted {
            // Still heard, so the server stops counting it as new.
            if let LobbyNotice::Invited(invite) = &notice {
                if level != NoticeLevel::Off {
                    self.rooms.mark_heard(&invite.id);
                }
            }
            return;
        }
        self.lobby.next += 1;
        let id = self.lobby.next;
        let name = notice.name();
        self.lobby.notices.push(Notice {
            id,
            notice,
            line,
            state: NoticeState::Waiting,
        });
        let change = format!("[notice] {id} {name}: new -> waiting");
        log::info!("{change}");
        self.note_change(change);
    }

    fn say_notice(&mut self, id: u64) {
        // The newest said is the one the key reaches; older ones are kept.
        let older: Vec<u64> = self
            .lobby
            .notices
            .iter()
            .filter(|n| n.state == NoticeState::Said && n.id != id)
            .map(|n| n.id)
            .collect();
        for old in older {
            self.set_notice_state(old, NoticeState::Kept);
        }
        if !self.set_notice_state(id, NoticeState::Said) {
            return;
        }
        let Some(n) = self.lobby.notices.iter().find(|n| n.id == id).cloned() else {
            return;
        };
        if let LobbyNotice::Invited(invite) = &n.notice {
            self.rooms.mark_heard(&invite.id);
        }
        self.play_ui(UiSound::Notify);
        self.say_queued(n.line);
        // An arrival with nothing waiting, or an answer with no match
        // (a no, or an invite that ran out), has nothing to go to: said,
        // it is over.
        let nowhere = match &n.notice {
            LobbyNotice::Arrived { waiting, .. } => *waiting == 0,
            LobbyNotice::Answered(invite) => invite.room.is_none(),
            _ => false,
        };
        if nowhere {
            self.set_notice_state(id, NoticeState::Done);
        }
    }

    /// Takes the player to room `id` of `kind`: the game's way if it has
    /// one, else the Room screen and the game's match screen over where
    /// they are.
    pub fn go_to_room(&mut self, kind: &str, id: &RoomId) {
        let Some(config) = G::lobby() else {
            return;
        };
        let kinds = (config.kinds)(self);
        let Some(kind) = kinds.into_iter().find(|k| k.kind == kind) else {
            return;
        };
        if self.rooms.room(id).is_none() {
            if let Some(info) = self
                .rooms
                .lobby()
                .and_then(|l| l.rooms.iter().find(|r| &r.id == id).cloned())
            {
                self.rooms.adopt(info);
            }
        }
        self.lobby.going = Some((id.clone(), std::time::Instant::now()));
        match config.go {
            Some(go) => go(self, &kind, id),
            None => crate::screens::rooms::enter_room(self, &kind, id),
        }
    }

    /// Marks notice `id` as acted on, or put off (`later`).
    pub(crate) fn settle_notice(&mut self, id: u64, later: bool) {
        let to = if later {
            NoticeState::Kept
        } else {
            NoticeState::Done
        };
        self.set_notice_state(id, to);
    }

    /// A notice by its id.
    pub fn notice(&self, id: u64) -> Option<&Notice> {
        self.lobby.notices.iter().find(|n| n.id == id)
    }

    /// Lines for the agent server's `screen`: each notice's state.
    pub fn describe_notices(&self) -> Vec<String> {
        self.lobby
            .notices
            .iter()
            .map(|n| format!("notice {} {} {}: {}", n.id, n.notice.name(), n.state.name(), n.line))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hearken_rooms::{Invite, InviteState, Side};

    fn side(name: &str) -> Side {
        Side {
            handle: "bbbbbbbbbbbbbbbb".into(),
            name: name.into(),
            guest: false,
        }
    }

    #[test]
    fn lines_are_short_and_name_the_game() {
        let label = |k: &str| if k == "pebbles" { "Pebbles".to_string() } else { k.to_string() };
        assert_eq!(
            notice_line(&LobbyNotice::Arrived { waiting: 2, here: 4 }, label),
            "2 games wait on you. 4 players here."
        );
        assert_eq!(
            notice_line(&LobbyNotice::Arrived { waiting: 1, here: 0 }, label),
            "1 game waits on you."
        );
        assert_eq!(notice_line(&LobbyNotice::Arrived { waiting: 0, here: 0 }, label), "");
        let invite = Invite {
            id: "x".into(),
            kind: "pebbles".into(),
            from: side("Marlow"),
            to: side("Ada"),
            state: InviteState::Sent,
            at: 0,
            changed_at: 0,
            mine: false,
            room: None,
        };
        assert_eq!(
            notice_line(&LobbyNotice::Invited(invite.clone()), label),
            "Marlow invites you. Pebbles."
        );
        let answered = Invite {
            state: InviteState::Declined,
            from: side("Ada"),
            to: side("Marlow"),
            mine: true,
            ..invite
        };
        assert_eq!(
            notice_line(&LobbyNotice::Answered(answered), label),
            "Marlow said no. Pebbles."
        );
        assert_eq!(
            notice_line(
                &LobbyNotice::YourMove {
                    room: RoomId("0".repeat(24)),
                    kind: "pebbles".into(),
                    name: "Marlow".into()
                },
                label
            ),
            "Your move. Pebbles, Marlow."
        );
    }

    #[test]
    fn the_notice_machine_refuses_a_jump_it_does_not_have() {
        use NoticeState::*;
        assert!(Waiting.allows(Held));
        assert!(Held.allows(Said));
        assert!(Said.allows(Kept));
        assert!(!Kept.allows(Said));
        assert!(!Done.allows(Waiting));
        assert!(!Said.allows(Held));
    }
}
