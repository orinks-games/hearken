//! What the server says about a room, as the client reads it. The JSON is
//! the rooms protocol in `docs/online.md`; anything a server adds that is
//! not named here is kept in `extra` fields and ignored.

use hearken_replay::Seat;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A room's id: 24 hex characters from the server.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RoomId(pub String);

impl RoomId {
    /// Whether `text` has the shape of a room id.
    pub fn valid(text: &str) -> bool {
        text.len() == 24
            && text
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    }
}

impl std::fmt::Display for RoomId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where a room is in its life. The server's state machine; a client only
/// reads it. `Unknown` is a phase a newer server added.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    /// Seats are filling. The host can start once the minimum is met.
    Waiting,
    /// The match is on: turns and chat are taken.
    Playing,
    /// The rules ended. Read-only.
    Finished,
    /// A waiting room that emptied or sat idle. Read-only.
    Closed,
    /// A match nobody moved in for the idle limit. Read-only.
    Abandoned,
    /// A channel: members come and go, chat is taken, nothing is played.
    Open,
    #[serde(other)]
    Unknown,
}

impl Phase {
    /// Whether nothing more can happen in the room.
    pub fn is_final(self) -> bool {
        matches!(self, Phase::Finished | Phase::Closed | Phase::Abandoned)
    }

    /// The phase as the player hears it.
    pub fn spoken(self) -> &'static str {
        match self {
            Phase::Waiting => "waiting for players",
            Phase::Playing => "in play",
            Phase::Finished => "over",
            Phase::Closed => "closed",
            Phase::Abandoned => "abandoned",
            Phase::Open => "open",
            Phase::Unknown => "unknown",
        }
    }
}

/// How a player has set another: not heard, or kept apart altogether.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MuteMode {
    /// Their lines are left out of what you read. They are not told.
    Mute,
    /// Neither of you reads the other, and you are never seated together
    /// by Play with anyone or in a direct room.
    Block,
    /// Neither: forgets a mute or block.
    None,
}

impl MuteMode {
    pub fn spoken(self) -> &'static str {
        match self {
            MuteMode::Mute => "muted",
            MuteMode::Block => "blocked",
            MuteMode::None => "heard",
        }
    }
}

/// One player the reading player has muted or blocked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mute {
    pub handle: String,
    pub mode: MuteMode,
    /// The player's name as the server last saw it.
    #[serde(default)]
    pub name: Option<String>,
}

/// Whether `text` has the shape of a player's handle.
pub fn valid_handle(text: &str) -> bool {
    text.len() == 16
        && text
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// How many seats a room's kind has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seats {
    pub min: u8,
    pub max: u8,
}

/// A player in a room.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub seat: u8,
    pub name: String,
    /// What other players know this one by: stable for the account, for
    /// muting, blocking, reporting and direct rooms.
    #[serde(default)]
    pub handle: Option<String>,
    #[serde(default)]
    pub guest: bool,
    /// Polled or connected in the last half minute.
    #[serde(default)]
    pub here: bool,
    /// Left the match (forfeited, or ran out of time). Never set in a
    /// waiting room: a member who leaves one is simply gone.
    #[serde(default)]
    pub left: bool,
}

impl Member {
    pub fn seat(&self) -> Option<Seat> {
        Seat::new(self.seat)
    }
}

/// A room as the server describes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoomInfo {
    pub id: RoomId,
    pub kind: String,
    pub code: String,
    /// A channel's name, when it was joined by name.
    #[serde(default)]
    pub name: Option<String>,
    /// A room of two, found or made by the other player's handle.
    #[serde(default)]
    pub direct: bool,
    pub phase: Phase,
    #[serde(default)]
    pub open: bool,
    pub seats: Seats,
    #[serde(default)]
    pub host: u8,
    #[serde(default)]
    pub members: Vec<Member>,
    /// The seat the reading player holds.
    #[serde(default)]
    pub you: Option<u8>,
    /// The seed the match started from, as a decimal string, once playing.
    #[serde(default)]
    pub seed: Option<String>,
    #[serde(default)]
    pub context: Value,
    /// The last event's sequence number.
    #[serde(default)]
    pub seq: u64,
    /// Where the match stands, when the handler replays after each turn:
    /// `{ ticks, finished, outcome }`.
    #[serde(default)]
    pub snapshot: Option<Value>,
    /// What the handler made of the finished match.
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub turn_started_at: Option<u64>,
    /// Events the player has not read, in a My games list.
    #[serde(default)]
    pub unread: u64,
}

impl RoomInfo {
    /// The seed as a number, once the match has started.
    pub fn seed(&self) -> Option<u64> {
        self.seed.as_deref().and_then(|s| s.parse().ok())
    }

    /// The player's own seat.
    pub fn you(&self) -> Option<Seat> {
        self.you.and_then(Seat::new)
    }

    /// Whether the reading player is the host.
    pub fn is_host(&self) -> bool {
        self.you == Some(self.host)
    }

    /// The members still in their seats.
    pub fn seated(&self) -> impl Iterator<Item = &Member> {
        self.members.iter().filter(|m| !m.left)
    }

    /// The member in `seat`.
    pub fn member(&self, seat: Seat) -> Option<&Member> {
        self.members.iter().find(|m| m.seat == seat.number())
    }

    /// The host's name, when the host is seated.
    pub fn host_name(&self) -> Option<&str> {
        self.members
            .iter()
            .find(|m| m.seat == self.host)
            .map(|m| m.name.as_str())
    }

    /// How many more players the room needs before it can start.
    pub fn missing(&self) -> u8 {
        let seated = self.seated().count().min(usize::from(u8::MAX)) as u8;
        self.seats.min.saturating_sub(seated)
    }

    /// The outcome so far, from the snapshot the server keeps when its
    /// handler replays after each turn; `None` when it does not.
    pub fn outcome(&self) -> Option<&Value> {
        self.snapshot.as_ref().and_then(|s| s.get("outcome"))
    }

    /// The outcome so far as the rules' own type.
    pub fn outcome_as<O: serde::de::DeserializeOwned>(&self) -> Option<O> {
        serde_json::from_value(self.outcome()?.clone()).ok()
    }

    /// Whose move it is, when the rules put `next_seat` in their outcome.
    pub fn next_seat(&self) -> Option<Seat> {
        let n = self.outcome()?.get("next_seat")?.as_u64()?;
        Seat::new(u8::try_from(n).ok()?)
    }

    /// Whether it is the reading player's move, as far as the server knows.
    pub fn my_turn(&self) -> bool {
        self.phase == Phase::Playing && self.next_seat().is_some() && self.next_seat() == self.you()
    }

    /// The rules' summary line for `seat`, when the outcome has `summary`.
    pub fn summary_for(&self, seat: Seat) -> Option<String> {
        self.outcome()?
            .get("summary")?
            .as_array()?
            .get(seat.index())?
            .as_str()
            .map(|s| crate::clean_text(s, 200))
    }

    /// Whether this room is a channel (named, direct, or open) rather
    /// than a match.
    pub fn is_channel(&self) -> bool {
        self.phase == Phase::Open || self.direct || self.name.is_some()
    }

    /// The member with `handle`.
    pub fn member_by_handle(&self, handle: &str) -> Option<&Member> {
        self.members
            .iter()
            .find(|m| m.handle.as_deref() == Some(handle))
    }

    /// The names of the other seats, in seat order.
    pub fn others(&self) -> Vec<String> {
        self.members
            .iter()
            .filter(|m| Some(m.seat) != self.you)
            .map(|m| m.name.clone())
            .collect()
    }
}

/// What kind of event happened in a room.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    /// A player took a seat (waiting rooms).
    Join,
    /// A player left a waiting room.
    Leave,
    /// The match began: `seed`, `context`, `build`.
    Start,
    /// A seat's move: `inputs`.
    Turn,
    /// A seat left the match: `reason` is `left` or `turn-limit`.
    Left,
    /// A line of chat: `name`, `text`, and `tag` when the sender gave one.
    Chat,
    #[serde(other)]
    Unknown,
}

/// One event in a room's log.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    #[serde(rename = "type")]
    pub kind: EventKind,
    #[serde(default)]
    pub at: u64,
    #[serde(default)]
    pub seat: Option<u8>,
    #[serde(default)]
    pub name: Option<String>,
    /// The sender's handle, on joins, leaves and chat.
    #[serde(default)]
    pub handle: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    /// A short line the sender attached to a chat line: where they are,
    /// what they are doing. The game decides what goes here.
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub inputs: Option<Vec<(u16, i32)>>,
    #[serde(default)]
    pub reason: Option<String>,
}

impl Event {
    pub fn seat(&self) -> Option<Seat> {
        self.seat.and_then(Seat::new)
    }

    /// A chat line as it is spoken: "Ada: nice move", or with the tag,
    /// "Ada, Lake Road: nice move". `None` for any other event.
    pub fn chat_line(&self) -> Option<String> {
        if self.kind != EventKind::Chat {
            return None;
        }
        let name = self.name.as_deref().unwrap_or("Someone");
        let text = self.text.as_deref().unwrap_or("");
        Some(match self.tag.as_deref().filter(|t| !t.is_empty()) {
            Some(tag) => format!("{name}, {tag}: {text}"),
            None => format!("{name}: {text}"),
        })
    }

    /// An empty event standing at `seq`: where a read ended when the
    /// lines up to there were left out, so the client is not behind.
    pub(crate) fn marker(seq: u64) -> Event {
        Event {
            seq,
            kind: EventKind::Unknown,
            at: 0,
            seat: None,
            name: None,
            handle: None,
            text: None,
            tag: None,
            inputs: None,
            reason: None,
        }
    }

    /// Whether this event moves the match: a turn or a seat leaving.
    pub fn is_move(&self) -> bool {
        matches!(self.kind, EventKind::Turn | EventKind::Left)
    }
}
