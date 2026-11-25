//! The lobby: who is here across the whole game, invites to a match, and
//! friends, kept up to date by a pulse.
//!
//! Once a game turns the pulse on ([`crate::Rooms::set_pulse`]), the client
//! tells the server where the player is about once a minute (more often
//! while a lobby screen is open) and gets back everything the lobby shows
//! in one answer: a [`Lobby`]. Comparing each answer with the last one
//! gives the [`LobbyNotice`]s a game speaks: someone sat down at your
//! table, it is your move, an invite came, a friend arrived.
//!
//! The server owns presence and invites; this module only reads them. Each
//! is an explicit state, named the same on both sides:
//!
//! - [`Presence`]: `Away`, `Here`, `Playing`, `AtTable`. The client works
//!   it out and sends it ([`Whereabouts`]); the server keeps the last word.
//! - [`InviteState`]: `Sent`, `Heard`, `Accepted`, `Declined`, `Withdrawn`,
//!   `Lapsed`. The server moves it; the client marks an invite `Heard`
//!   once its notice was given ([`crate::Rooms::mark_heard`]).
//! - [`Visibility`]: who may see the player. A filter, not a state.

use serde::{Deserialize, Serialize};

use crate::protocol::{RoomId, RoomInfo};

/// Where a player is, as the lobby knows it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Presence {
    /// The game is closed, or no pulse has come for two minutes.
    Away,
    /// In the game's menus.
    #[default]
    Here,
    /// In a run of a named game.
    Playing,
    /// At a table: a live match.
    #[serde(rename = "table")]
    AtTable,
}

impl Presence {
    /// The state's stable name, as the server and the logs have it.
    pub fn name(self) -> &'static str {
        match self {
            Presence::Away => "away",
            Presence::Here => "here",
            Presence::Playing => "playing",
            Presence::AtTable => "table",
        }
    }
}

/// Who may see the player as here: the player's setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    /// Everyone.
    #[default]
    Everyone,
    /// Only the players this player has added as friends.
    Friends,
    /// Nobody. The player still sees others, and invites still come.
    Nobody,
}

impl Visibility {
    pub const ALL: [Visibility; 3] = [Visibility::Everyone, Visibility::Friends, Visibility::Nobody];

    pub fn name(self) -> &'static str {
        match self {
            Visibility::Everyone => "everyone",
            Visibility::Friends => "friends",
            Visibility::Nobody => "nobody",
        }
    }

    /// The setting as a Settings row reads it.
    pub fn spoken(self) -> &'static str {
        match self {
            Visibility::Everyone => "everyone",
            Visibility::Friends => "friends only",
            Visibility::Nobody => "nobody",
        }
    }

    /// The next setting along, wrapping, for Left and Right on a row.
    pub fn stepped(self, delta: i32) -> Visibility {
        let at = Visibility::ALL.iter().position(|v| *v == self).unwrap_or(0) as i32;
        let n = Visibility::ALL.len() as i32;
        Visibility::ALL[((at + delta).rem_euclid(n)) as usize]
    }

    pub fn is_default(&self) -> bool {
        *self == Visibility::default()
    }
}

/// Where an invite is in its life. The server's state machine; a client
/// reads it, and marks `Sent` as `Heard` through its next pulse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InviteState {
    /// The invited player has not had the notice yet.
    Sent,
    /// They know of it.
    Heard,
    /// They said yes: a match exists with both seated.
    Accepted,
    /// They said no.
    Declined,
    /// The sender took it back.
    Withdrawn,
    /// Nobody answered in time.
    Lapsed,
    #[serde(other)]
    Unknown,
}

impl InviteState {
    pub fn name(self) -> &'static str {
        match self {
            InviteState::Sent => "sent",
            InviteState::Heard => "heard",
            InviteState::Accepted => "accepted",
            InviteState::Declined => "declined",
            InviteState::Withdrawn => "withdrawn",
            InviteState::Lapsed => "lapsed",
            InviteState::Unknown => "unknown",
        }
    }

    /// Whether nothing more can happen to the invite.
    pub fn is_final(self) -> bool {
        !matches!(self, InviteState::Sent | InviteState::Heard)
    }
}

/// What a player says to an invite.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InviteAnswer {
    /// Yes: the match starts with both seated.
    Accept,
    /// No.
    Decline,
    /// The sender takes it back.
    Withdraw,
}

/// Where the player is, as the pulse tells the server.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Whereabouts {
    pub presence: Presence,
    /// The game or part of the game the player is in, as an id
    /// (`"larkfield"`). `None` in the menus.
    pub game: Option<String>,
    /// The same as other players hear it (`"Larkfield"`).
    pub place: Option<String>,
}

impl Whereabouts {
    /// In the menus, in no game.
    pub fn here() -> Whereabouts {
        Whereabouts::default()
    }

    /// In `presence` at `game`, heard as `place`.
    pub fn at(presence: Presence, game: &str, place: &str) -> Whereabouts {
        Whereabouts {
            presence,
            game: Some(game.to_string()),
            place: Some(place.to_string()),
        }
    }
}

/// A player in the lobby's lists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Player {
    pub handle: String,
    pub name: String,
    #[serde(default)]
    pub guest: bool,
    pub state: Presence,
    #[serde(default)]
    pub game: Option<String>,
    /// Where they are, as players hear it.
    #[serde(default)]
    pub place: Option<String>,
    /// Whether the reading player has added them as a friend.
    #[serde(default)]
    pub friend: bool,
}

impl Player {
    /// "Marlow, in Larkfield", "Marlow", "Marlow, away".
    pub fn line(&self) -> String {
        let name = crate::clean_text(&self.name, 64);
        match (self.state, self.place.as_deref()) {
            (Presence::Away, _) => format!("{name}, away"),
            (_, Some(place)) if !place.is_empty() => {
                format!("{name}, in {}", crate::clean_text(place, 64))
            }
            _ => name,
        }
    }
}

/// One side of an invite.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Side {
    pub handle: String,
    pub name: String,
    #[serde(default)]
    pub guest: bool,
}

/// An invite to a match, as either player sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    pub id: String,
    /// The kind of room it is for (`"pebbles"`).
    pub kind: String,
    pub from: Side,
    pub to: Side,
    pub state: InviteState,
    #[serde(default)]
    pub at: u64,
    #[serde(default)]
    pub changed_at: u64,
    /// Whether the reading player sent it.
    #[serde(default)]
    pub mine: bool,
    /// The match it made, once accepted.
    #[serde(default)]
    pub room: Option<RoomId>,
}

impl Invite {
    /// The other player's name, from the reader's side.
    pub fn other(&self) -> &str {
        if self.mine {
            &self.to.name
        } else {
            &self.from.name
        }
    }

    /// Whether it waits on the reading player's answer.
    pub fn waits_on_me(&self) -> bool {
        !self.mine && !self.state.is_final()
    }
}

/// A table anyone may take: a waiting room of someone else's, open to all.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenTable {
    pub room: RoomId,
    pub kind: String,
    /// Its invite code: an open table is anyone's to join.
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub since: u64,
}

/// Everything one pulse brought back.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lobby {
    pub you: Side,
    /// The player's own presence as the server took it.
    #[serde(default)]
    pub state: Presence,
    /// Everyone else here whom the player may see, friends first.
    #[serde(default)]
    pub here: Vec<Player>,
    /// Everyone the player has added, here or not.
    #[serde(default)]
    pub friends: Vec<Player>,
    /// Invites to the player waiting on them, theirs waiting on others,
    /// and theirs that ended since the last pulse (once).
    #[serde(default)]
    pub invites: Vec<Invite>,
    /// The player's matches, waiting or in play.
    #[serde(default)]
    pub rooms: Vec<RoomInfo>,
    /// Tables others opened for anyone, oldest first.
    #[serde(default)]
    pub open: Vec<OpenTable>,
    /// Seconds until the server wants the next pulse.
    #[serde(default)]
    pub every: u64,
}

impl Lobby {
    /// Invites waiting on the player's answer.
    pub fn incoming(&self) -> impl Iterator<Item = &Invite> {
        self.invites.iter().filter(|i| i.waits_on_me())
    }

    /// The player's invites waiting on others.
    pub fn outgoing(&self) -> impl Iterator<Item = &Invite> {
        self.invites
            .iter()
            .filter(|i| i.mine && !i.state.is_final())
    }

    /// Matches where it is the player's move.
    pub fn my_moves(&self) -> impl Iterator<Item = &RoomInfo> {
        self.rooms.iter().filter(|r| r.my_turn())
    }

    /// What waits on the player: invites to answer and moves to make.
    pub fn waiting(&self) -> usize {
        self.incoming().count() + self.my_moves().count()
    }

    /// The player with `handle`, from the lists.
    pub fn player(&self, handle: &str) -> Option<&Player> {
        self.here
            .iter()
            .chain(self.friends.iter())
            .find(|p| p.handle == handle)
    }

    /// Whether the player has added `handle` as a friend.
    pub fn is_friend(&self, handle: &str) -> bool {
        self.friends.iter().any(|p| p.handle == handle)
    }
}

/// Something that happened between two pulses, for the game to tell the
/// player.
#[derive(Clone, Debug, PartialEq)]
pub enum LobbyNotice {
    /// The first pulse since the player connected: how many things wait on
    /// them, and how many other players are here.
    Arrived { waiting: usize, here: usize },
    /// Someone took a seat at the player's table.
    SatDown {
        room: RoomId,
        kind: String,
        name: String,
    },
    /// It became the player's move.
    YourMove {
        room: RoomId,
        kind: String,
        name: String,
    },
    /// An invite came for the player.
    Invited(Invite),
    /// The player's invite was accepted, declined or lapsed.
    Answered(Invite),
    /// A friend came in.
    FriendHere(Player),
    /// Someone opened a table for anyone.
    OpenTable(OpenTable),
}

impl LobbyNotice {
    /// The room it is about, when there is one.
    pub fn room(&self) -> Option<&RoomId> {
        match self {
            LobbyNotice::SatDown { room, .. } | LobbyNotice::YourMove { room, .. } => Some(room),
            LobbyNotice::Answered(invite) => invite.room.as_ref(),
            LobbyNotice::OpenTable(table) => Some(&table.room),
            _ => None,
        }
    }

    /// Whether it is about the player's own games, or a friend: what is
    /// left when the player narrows notices to friends and their games.
    pub fn is_mine_or_friends(&self, lobby: &Lobby) -> bool {
        match self {
            LobbyNotice::Arrived { .. }
            | LobbyNotice::SatDown { .. }
            | LobbyNotice::YourMove { .. }
            | LobbyNotice::Answered(_)
            | LobbyNotice::FriendHere(_) => true,
            LobbyNotice::Invited(invite) => lobby.is_friend(&invite.from.handle),
            LobbyNotice::OpenTable(_) => false,
        }
    }

    /// A stable name for logs and the agent tools.
    pub fn name(&self) -> &'static str {
        match self {
            LobbyNotice::Arrived { .. } => "arrived",
            LobbyNotice::SatDown { .. } => "sat-down",
            LobbyNotice::YourMove { .. } => "your-move",
            LobbyNotice::Invited(_) => "invited",
            LobbyNotice::Answered(_) => "answered",
            LobbyNotice::FriendHere(_) => "friend-here",
            LobbyNotice::OpenTable(_) => "open-table",
        }
    }
}

/// What changed from `old` to `new`. With no `old` (the first pulse) only
/// `Arrived` and the invites not heard yet.
pub fn changes(old: Option<&Lobby>, new: &Lobby) -> Vec<LobbyNotice> {
    let mut out = Vec::new();
    let Some(old) = old else {
        out.push(LobbyNotice::Arrived {
            waiting: new.waiting(),
            here: new.here.len(),
        });
        for invite in new.incoming().filter(|i| i.state == InviteState::Sent) {
            out.push(LobbyNotice::Invited(invite.clone()));
        }
        return out;
    };
    // Rooms an answered invite made are told by that answer, not again as
    // a seat taken.
    let answered: Vec<&RoomId> = new
        .invites
        .iter()
        .filter(|i| i.mine && i.state == InviteState::Accepted)
        .filter_map(|i| i.room.as_ref())
        .collect();
    for invite in &new.invites {
        let known = old.invites.iter().any(|i| i.id == invite.id);
        if invite.waits_on_me() && invite.state == InviteState::Sent && !known {
            out.push(LobbyNotice::Invited(invite.clone()));
        }
        if invite.mine
            && matches!(
                invite.state,
                InviteState::Accepted | InviteState::Declined | InviteState::Lapsed
            )
        {
            out.push(LobbyNotice::Answered(invite.clone()));
        }
    }
    for room in &new.rooms {
        let was = old.rooms.iter().find(|r| r.id == room.id);
        if room.is_host() && !answered.contains(&&room.id) {
            let before: Vec<&str> = was
                .map(|r| r.members.iter().map(|m| m.name.as_str()).collect())
                .unwrap_or_default();
            if was.is_some() || room.members.len() > 1 {
                for member in &room.members {
                    if Some(member.seat) == room.you || member.left {
                        continue;
                    }
                    if was.is_none() || !before.contains(&member.name.as_str()) {
                        out.push(LobbyNotice::SatDown {
                            room: room.id.clone(),
                            kind: room.kind.clone(),
                            name: member.name.clone(),
                        });
                    }
                }
            }
        }
        if room.my_turn() && !was.is_some_and(|r| r.my_turn()) {
            out.push(LobbyNotice::YourMove {
                room: room.id.clone(),
                kind: room.kind.clone(),
                name: room.others().join(" and "),
            });
        }
    }
    for friend in &new.friends {
        if friend.state == Presence::Away {
            continue;
        }
        // A player just added while here has not come in.
        let was_here = old
            .friends
            .iter()
            .chain(&old.here)
            .find(|f| f.handle == friend.handle)
            .is_some_and(|f| f.state != Presence::Away);
        if !was_here {
            out.push(LobbyNotice::FriendHere(friend.clone()));
        }
    }
    for table in &new.open {
        if !old.open.iter().any(|t| t.room == table.room) {
            out.push(LobbyNotice::OpenTable(table.clone()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lobby(value: serde_json::Value) -> Lobby {
        let mut base = json!({ "you": { "handle": "aaaaaaaaaaaaaaaa", "name": "Ada" } });
        for (k, v) in value.as_object().unwrap() {
            base[k] = v.clone();
        }
        serde_json::from_value(base).unwrap()
    }

    fn room(id: &str, members: &[&str], next: Option<u8>, phase: &str) -> serde_json::Value {
        json!({
            "id": id, "kind": "pebbles", "code": "ABCDEF", "phase": phase,
            "seats": { "min": 2, "max": 2 }, "host": 0, "you": 0,
            "members": members.iter().enumerate().map(|(i, n)| json!({ "seat": i, "name": n })).collect::<Vec<_>>(),
            "snapshot": next.map(|n| json!({ "outcome": { "next_seat": n } })),
        })
    }

    fn invite(id: &str, state: &str, mine: bool) -> serde_json::Value {
        json!({
            "id": id, "kind": "pebbles", "state": state, "mine": mine,
            "from": { "handle": "bbbbbbbbbbbbbbbb", "name": if mine { "Ada" } else { "Bo" } },
            "to": { "handle": "cccccccccccccccc", "name": if mine { "Bo" } else { "Ada" } },
            "room": if state == "accepted" { json!("0123456789abcdef01234567") } else { json!(null) },
        })
    }

    #[test]
    fn names_read_where_players_are() {
        let p: Player = serde_json::from_value(json!({
            "handle": "h", "name": "Marlow", "state": "playing", "place": "Larkfield"
        }))
        .unwrap();
        assert_eq!(p.line(), "Marlow, in Larkfield");
        let away: Player = serde_json::from_value(json!({ "handle": "h", "name": "Marlow", "state": "away" })).unwrap();
        assert_eq!(away.line(), "Marlow, away");
        let table: Player = serde_json::from_value(json!({ "handle": "h", "name": "Bo", "state": "table" })).unwrap();
        assert_eq!(table.state, Presence::AtTable);
        assert_eq!(Visibility::Everyone.stepped(-1), Visibility::Nobody);
    }

    #[test]
    fn the_first_pulse_arrives_with_what_waits() {
        let new = lobby(json!({
            "here": [{ "handle": "h", "name": "Bo", "state": "here" }],
            "invites": [invite("i1", "sent", false), invite("i2", "heard", false), invite("i3", "sent", true)],
            "rooms": [room("0123456789abcdef01234567", &["Ada", "Bo"], Some(0), "playing")],
        }));
        let got = changes(None, &new);
        assert_eq!(got[0], LobbyNotice::Arrived { waiting: 3, here: 1 });
        assert_eq!(got.len(), 2);
        assert!(matches!(&got[1], LobbyNotice::Invited(i) if i.id == "i1"));
    }

    #[test]
    fn later_pulses_tell_what_changed_once() {
        let id = "0123456789abcdef01234567";
        let old = lobby(json!({
            "rooms": [room(id, &["Ada"], None, "waiting")],
            "friends": [{ "handle": "f", "name": "Cy", "state": "away" }],
            "invites": [invite("i2", "heard", false)],
        }));
        let new = lobby(json!({
            "rooms": [room(id, &["Ada", "Bo"], Some(0), "playing")],
            "friends": [{ "handle": "f", "name": "Cy", "state": "here" }],
            "invites": [invite("i2", "heard", false), invite("i4", "sent", false), invite("i5", "declined", true)],
            "open": [{ "room": "fedcba9876543210fedcba98", "kind": "pebbles", "host": "Di" }],
        }));
        let names: Vec<&str> = changes(Some(&old), &new).iter().map(|n| n.name()).collect();
        assert_eq!(
            names,
            ["invited", "answered", "sat-down", "your-move", "friend-here", "open-table"]
        );
        // Nothing new: nothing said.
        let same = lobby(json!({
            "rooms": [room(id, &["Ada", "Bo"], Some(0), "playing")],
            "friends": [{ "handle": "f", "name": "Cy", "state": "here" }],
            "invites": [invite("i2", "heard", false), invite("i4", "heard", false)],
            "open": [{ "room": "fedcba9876543210fedcba98", "kind": "pebbles", "host": "Di" }],
        }));
        assert_eq!(changes(Some(&new), &same), Vec::new());
    }

    #[test]
    fn an_accepted_invite_is_told_once_not_as_a_seat_taken() {
        let id = "0123456789abcdef01234567";
        let old = lobby(json!({ "invites": [invite("i1", "heard", true)] }));
        let new = lobby(json!({
            "invites": [invite("i1", "accepted", true)],
            "rooms": [room(id, &["Ada", "Bo"], Some(1), "playing")],
        }));
        let names: Vec<&str> = changes(Some(&old), &new).iter().map(|n| n.name()).collect();
        assert_eq!(names, ["answered"]);
    }

    #[test]
    fn adding_a_player_who_is_here_is_no_arrival() {
        let old = lobby(json!({ "here": [{ "handle": "f", "name": "Cy", "state": "here" }] }));
        let new = lobby(json!({
            "here": [{ "handle": "f", "name": "Cy", "state": "here" }],
            "friends": [{ "handle": "f", "name": "Cy", "state": "here" }],
        }));
        assert_eq!(changes(Some(&old), &new), Vec::new());
    }
}
