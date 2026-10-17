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
/// the first letter capitalized the way they meant it to. Punctuation is named: most
/// voices say nothing for a bare colon or slash, so typing a web address character by
/// character would otherwise be a run of silent clicks.
pub fn spoken_char(ch: char) -> String {
    let name = match ch {
        ' ' => "space",
        ':' => "colon",
        ';' => "semicolon",
        '/' => "slash",
        '\\' => "backslash",
        '.' => "dot",
        ',' => "comma",
        '-' => "dash",
        '_' => "underscore",
        '?' => "question mark",
        '!' => "exclamation mark",
        '=' => "equals",
        '&' => "and",
        '%' => "percent",
        '#' => "number sign",
        '@' => "at",
        '~' => "tilde",
        '+' => "plus",
        '*' => "star",
        '\'' => "apostrophe",
        '"' => "quote",
        '(' => "left paren",
        ')' => "right paren",
        _ => "",
    };
    if !name.is_empty() {
        return name.to_string();
    }
    if ch.is_alphabetic() && ch.is_uppercase() {
        return format!("cap {}", ch.to_lowercase());
    }
    ch.to_string()
}

/// `phrase` with its first letter capitalised, to open a sentence: "the A
/// button" becomes "The A button".
pub fn sentence_start(phrase: &str) -> String {
    let mut chars = phrase.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_sentence_adds_one_mark_only() {
        assert_eq!(end_sentence("Sleep 10 hours"), "Sleep 10 hours.");
        assert_eq!(end_sentence("Done. "), "Done.");
        assert_eq!(end_sentence("Ready?"), "Ready?");
        assert_eq!(end_sentence("Note:"), "Note:");
    }

    #[test]
    fn spoken_char_marks_space_and_capitals() {
        assert_eq!(spoken_char(' '), "space");
        assert_eq!(spoken_char('J'), "cap j");
        assert_eq!(spoken_char('j'), "j");
        assert_eq!(spoken_char('7'), "7");
        assert_eq!(spoken_char(','), "comma");
        assert_eq!(spoken_char(':'), "colon");
        assert_eq!(spoken_char('/'), "slash");
    }

    #[test]
    fn spoken_char_names_every_punctuation_mark_it_knows() {
        let names = [
            ('.', "dot"),
            (';', "semicolon"),
            ('\\', "backslash"),
            ('-', "dash"),
            ('_', "underscore"),
            ('?', "question mark"),
            ('!', "exclamation mark"),
            ('=', "equals"),
            ('&', "and"),
            ('%', "percent"),
            ('#', "number sign"),
            ('@', "at"),
            ('~', "tilde"),
            ('+', "plus"),
            ('*', "star"),
            ('\'', "apostrophe"),
            ('"', "quote"),
            ('(', "left paren"),
            (')', "right paren"),
        ];
        for (ch, name) in names {
            assert_eq!(spoken_char(ch), name, "{ch:?}");
        }
        // Anything unnamed is itself.
        assert_eq!(spoken_char('$'), "$");
        assert_eq!(spoken_char('é'), "é");
        assert_eq!(spoken_char('É'), "cap é");
    }
}
