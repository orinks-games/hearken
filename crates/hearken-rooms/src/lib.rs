//! Online play between players for Hearken games: rooms with seats, turns
//! the server orders and verifies, chat, and presence.
//!
//! A game creates or joins a [`Room`] through [`Rooms`], waits for its
//! seats to fill, and plays: each player's turn goes to the server, which
//! stamps it with the player's seat and appends it to the room's log;
//! every client applies the turns in that order through a [`Match`], a
//! `Recording` of the game's own rules with `Seated` inputs. At the end the
//! server replays the whole match with the game's published rules module
//! and hands the outcome to the game's handler, exactly as it verifies a
//! solo run. Chat lines ride in the same log.
//!
//! Nothing here blocks the game loop: requests run on worker threads and
//! answer through tickets; `Rooms::update` polls watched rooms. Nothing
//! here needs hearken-app either: a game with its own screens embeds this
//! crate and gives it a bearer token; hearken-app wraps it as `ctx.rooms`.
//!
//! The server side is `server/` in the Hearken repository; `docs/online.md`
//! is the protocol.

mod client;
mod error;
mod matches;
pub mod protocol;
pub mod transport;

pub use client::{
    Answer, Connection, Notice, Room, Rooms, RoomsConfig, Ticket, MAX_IN_FLIGHT, POLL_ACTIVE,
    POLL_QUIET, QUIET_AFTER,
};
pub use error::RoomsError;
pub use matches::{Applied, Match, MatchError, Sync};
pub use protocol::{Event, EventKind, Member, Phase, RoomId, RoomInfo, Seats};
pub use transport::{Reply, Transport};

/// The longest chat line, in characters.
pub const MAX_CHAT_CHARS: usize = 200;

/// The letters an invite code is made of: ones a synthesized voice does
/// not blur (no O/0, I/1/L, S/5, Z/2), as the connect codes use.
pub const CODE_ALPHABET: &str = "ABCDEFGHJKMNPQRTUVWXY346789";
/// An invite code's length.
pub const CODE_LENGTH: usize = 6;

/// Whether `kind` can name a kind of room: 1 to 32 of `a-z`, `0-9` and `-`.
pub fn valid_kind(kind: &str) -> bool {
    let bytes = kind.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 32
        && bytes[0] != b'-'
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// What the player typed, as an invite code: any case, spaces and dashes
/// dropped; `None` when it is not one.
pub fn normalize_code(text: &str) -> Option<String> {
    let code: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    (code.len() == CODE_LENGTH && code.chars().all(|c| CODE_ALPHABET.contains(c))).then_some(code)
}

/// An invite code as it is read out: letters and digits one by one, in
/// two groups of three, so a voice does not run them into a word.
pub fn spell_code(code: &str) -> String {
    let mut out = String::new();
    for (i, c) in code.chars().enumerate() {
        if i == 3 {
            out.push_str(", ");
        } else if i > 0 {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// Text from the server or the player as it may be spoken: no control or
/// invisible characters, one space between words, at most `max` chars.
pub fn clean_text(text: &str, max: usize) -> String {
    let hidden = |c: char| {
        c.is_control()
            || matches!(c,
                '\u{200b}'..='\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'..='\u{2064}'
                | '\u{feff}'
                | '\u{fff9}'..='\u{fffb}')
    };
    let mut out = String::new();
    for word in text
        .chars()
        .filter(|c| !hidden(*c) || c.is_whitespace())
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect::<String>()
        .split(' ')
        .filter(|w| !w.is_empty())
    {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out.chars().take(max).collect::<String>().trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_kinds_are_checked() {
        assert_eq!(normalize_code(" m4k-7t8 ").as_deref(), Some("M4K7T8"));
        assert_eq!(normalize_code("M4K7T"), None);
        assert_eq!(normalize_code("O00000"), None);
        assert_eq!(spell_code("M4K7T8"), "M 4 K, 7 T 8");
        assert!(valid_kind("match-2"));
        assert!(!valid_kind("-x"));
        assert!(!valid_kind("Match"));
        assert!(!valid_kind(""));
    }

    #[test]
    fn text_is_cleaned() {
        assert_eq!(
            clean_text("  hi\u{200b} there\u{7}\n friend ", 200),
            "hi there friend"
        );
        assert_eq!(clean_text(&"x".repeat(300), 200).len(), 200);
    }
}
