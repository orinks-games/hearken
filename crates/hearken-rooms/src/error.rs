//! Why something in a room did not happen, and what the player hears.

use crate::protocol::Phase;
use crate::transport::Reply;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoomsError {
    /// The game has no server for rooms.
    NotOffered,
    /// No account or guest token: nothing can be sent.
    NotConnected,
    /// A kind that is not 1 to 32 of `a-z`, `0-9` and `-`.
    BadKind,
    /// A code that is not six letters from the room-code alphabet.
    BadCode,
    /// Too many requests are still on their way.
    Busy,
    /// No answer from the server: no network, or it timed out.
    Offline,
    /// The server no longer knows the token.
    SignedOut,
    /// The server asked the game to slow down.
    RateLimited,
    /// The server is down or broken.
    Unavailable,
    /// No room has that code, or it has closed.
    UnknownCode,
    /// Every seat is taken.
    Full,
    /// The match has started without you.
    Started,
    /// The room is not in a phase where that can happen.
    WrongPhase(Phase),
    /// Only the host can start.
    NotTheHost,
    /// Not enough players to start.
    NotEnoughPlayers,
    /// Too many open games already.
    TooManyRooms,
    /// Not in that room.
    NotAMember,
    /// The server refused in its own words: its `error` code and a line
    /// for the player when it gave one.
    Refused {
        code: String,
        message: Option<String>,
    },
}

impl RoomsError {
    /// A short stable name, for logs and a game's own matching.
    pub fn code(&self) -> &str {
        match self {
            RoomsError::NotOffered => "not-offered",
            RoomsError::NotConnected => "not-connected",
            RoomsError::BadKind => "bad-kind",
            RoomsError::BadCode => "bad-code",
            RoomsError::Busy => "busy",
            RoomsError::Offline => "offline",
            RoomsError::SignedOut => "signed-out",
            RoomsError::RateLimited => "rate-limited",
            RoomsError::Unavailable => "unavailable",
            RoomsError::UnknownCode => "unknown-code",
            RoomsError::Full => "full",
            RoomsError::Started => "started",
            RoomsError::WrongPhase(_) => "wrong-phase",
            RoomsError::NotTheHost => "not-the-host",
            RoomsError::NotEnoughPlayers => "not-enough-players",
            RoomsError::TooManyRooms => "too-many-rooms",
            RoomsError::NotAMember => "not-a-member",
            RoomsError::Refused { code, .. } => code,
        }
    }

    /// What the player hears: what happened and, where there is one, what
    /// to do. `site` is the server's name ("orinks.net").
    pub fn spoken(&self, site: &str) -> String {
        match self {
            RoomsError::NotOffered => "This game has no online play.".into(),
            RoomsError::NotConnected => {
                format!("Not connected to {site}. Connect an account in Settings.")
            }
            RoomsError::BadKind => "That game cannot be played online.".into(),
            RoomsError::BadCode => "A code is six letters and numbers.".into(),
            RoomsError::Busy => "Still sending. Try again in a moment.".into(),
            RoomsError::Offline => format!("Could not reach {site}. Check your connection."),
            RoomsError::SignedOut => {
                format!("Your {site} account was disconnected. Connect it again in Settings.")
            }
            RoomsError::RateLimited => format!("{site} is busy. Try again in a minute."),
            RoomsError::Unavailable => format!("{site} is not answering. Try again later."),
            RoomsError::UnknownCode => "No game has that code.".into(),
            RoomsError::Full => "That game is full.".into(),
            RoomsError::Started => "That game has already started.".into(),
            RoomsError::WrongPhase(phase) => format!("That game is {}.", phase.spoken()),
            RoomsError::NotTheHost => "Only the host can start the game.".into(),
            RoomsError::NotEnoughPlayers => "Not enough players yet.".into(),
            RoomsError::TooManyRooms => "You have too many open games. Finish or leave one.".into(),
            RoomsError::NotAMember => "You are not in that game.".into(),
            RoomsError::Refused {
                message: Some(m), ..
            } => end_sentence(m),
            RoomsError::Refused { .. } => format!("{site} did not accept that."),
        }
    }

    pub(crate) fn from_reply(reply: &Reply) -> RoomsError {
        let code = reply.body["error"].as_str().unwrap_or("").to_string();
        let message = reply.body["message"]
            .as_str()
            .filter(|m| !m.trim().is_empty())
            .map(|m| crate::clean_text(m, 200))
            .filter(|m| !m.is_empty());
        match (reply.status, code.as_str()) {
            (401, _) => RoomsError::SignedOut,
            (429, _) => RoomsError::RateLimited,
            (404, "unknown-code") => RoomsError::UnknownCode,
            (404, "unknown-kind") => RoomsError::BadKind,
            (403, "not-a-member") => RoomsError::NotAMember,
            (403, "not-the-host") => RoomsError::NotTheHost,
            (409, "full") => RoomsError::Full,
            (409, "started") => RoomsError::Started,
            (409, "wrong-phase") => RoomsError::WrongPhase(
                serde_json::from_value(reply.body["phase"].clone()).unwrap_or(Phase::Unknown),
            ),
            (422, "not-enough-players") => RoomsError::NotEnoughPlayers,
            (422, "too-many-rooms") => RoomsError::TooManyRooms,
            (400..=499, _) => RoomsError::Refused {
                code: if code.is_empty() {
                    format!("http-{}", reply.status)
                } else {
                    code
                },
                message,
            },
            _ => RoomsError::Unavailable,
        }
    }
}

impl std::fmt::Display for RoomsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for RoomsError {}

/// `text` ending in a full stop, question or exclamation mark.
pub(crate) fn end_sentence(text: &str) -> String {
    let text = text.trim();
    if text.ends_with(['.', '!', '?']) {
        text.to_string()
    } else {
        format!("{text}.")
    }
}
