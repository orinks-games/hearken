//! Problem reports from a Hearken game.
//!
//! A [`Report`] holds what the player says happened, the game and system it
//! happened on, and the game's log. [`Report::send`] posts it to a relay,
//! which emails it to the developer, so the game never holds a mail
//! password and the relay only ever emails one address. [`Session`] marks a
//! run as running until it exits cleanly, so the next start can tell the
//! last run crashed and offer to report it.
//!
//! The wire format is the one [mailbag](https://github.com/accessware/mailbag)
//! speaks: a gzipped JSON object posted with `Content-Type:
//! application/json`, answered with `202` when the email went out, `400`
//! for a malformed report, `403` for a game the relay does not take, `413`
//! for one that is too big, `422` when the reply address's domain takes no
//! mail, `429` when a limit is reached and `502` when the email could not
//! be sent. A `mailbag-server` takes these reports as they are, and
//! `server/` in the Hearken repository has a relay that runs where the
//! game's other server code does.
//!
//! Nothing here knows any game or opens a window. The engine's screens are
//! in `hearken-app` behind its `report` feature.

mod report;
mod send;
mod session;

pub use report::{
    tail, Attachment, Detail, Invalid, Report, MAX_ATTACHMENTS, MAX_DESCRIPTION_CHARS, MAX_DETAILS,
    MAX_LOG_BYTES,
};
#[cfg(feature = "http")]
pub use send::HttpSender;
pub use send::{Answer, Opened, SendError, Sender};
pub use session::Session;
