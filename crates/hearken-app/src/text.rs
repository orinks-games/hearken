//! Small helpers for spoken text.

/// A spoken fragment ending in exactly one sentence mark, never two.
///
/// Menu and list labels are sometimes plain ("Play") and sometimes whole
/// sentences that already end in a period ("The door is locked.").
/// Appending "." unconditionally produces ".." which a screen reader voices as
/// "dot dot", so add the period only when one is not already there.
pub fn end_sentence(text: &str) -> String {
    let text = text.trim_end();
    if text.ends_with(['.', '!', '?', ':']) {
        text.to_string()
    } else {
        format!("{text}.")
    }
}

/// A single typed character the way an accessible text field announces it.
///
/// Shared by every text-entry field in the game so a character reviewed one at a time
/// always sounds the same way. A space reads as "space" rather than silence. A capital
/// letter gets a leading "cap" -- speech synthesis pronounces "J" and "j" identically,
/// so without the marker a player arrowing through a typed name has no way to hear that
/// the first letter capitalized the way they meant it to.
pub fn spoken_char(ch: char) -> String {
    if ch == ' ' {
        return "space".to_string();
    }
    if ch.is_alphabetic() && ch.is_uppercase() {
        return format!("cap {}", ch.to_lowercase());
    }
    ch.to_string()
}
