use std::fmt;

/// A spoken line carrying both of its renderings.
///
/// `Display` renders the normal form, and plain text converts into one via
/// `From`, so a call site can pass a `&str` where a message is accepted. The
/// delivery layer picks the rendering the player's speech mode asks for.
/// `terse == None` means the line reads the same in both modes; `terse ==
/// Some("")` means terse mode drops the line whole (an earcon or silence
/// carries it instead, and it never reaches a review log -- as far as the
/// game is concerned it was not said).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SpokenMessage {
    pub normal: String,
    pub terse: Option<String>,
}

impl SpokenMessage {
    /// A line that reads the same in both modes.
    pub fn new(normal: impl Into<String>) -> Self {
        Self {
            normal: normal.into(),
            terse: None,
        }
    }

    /// A line with a terse form of its own.
    pub fn with_terse(normal: impl Into<String>, terse: impl Into<String>) -> Self {
        Self {
            normal: normal.into(),
            terse: Some(terse.into()),
        }
    }

    /// Construct from an already-split normal/terse pair.
    pub fn pair(normal: impl Into<String>, terse: Option<String>) -> Self {
        Self {
            normal: normal.into(),
            terse,
        }
    }

    pub fn render(&self, terse: bool) -> &str {
        match (&self.terse, terse) {
            (Some(short), true) => short,
            _ => &self.normal,
        }
    }

    /// Both renderings extended with one more sentence.
    ///
    /// Plain concatenation would flatten the pair back to a bare string and
    /// silently lose the terse form; this keeps the suffix on both. A
    /// dropped line (`terse == Some("")`) keeps only the suffix in terse
    /// mode: the base line was color, but the suffix was appended because it
    /// reports something that happened.
    pub fn plus(&self, suffix: &str) -> SpokenMessage {
        let terse = match &self.terse {
            None => None,
            Some(short) if !short.is_empty() => Some(format!("{short} {suffix}")),
            Some(_) => Some(suffix.to_string()),
        };
        SpokenMessage {
            normal: format!("{} {suffix}", self.normal),
            terse,
        }
    }
}

impl fmt::Display for SpokenMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.normal)
    }
}

impl From<&str> for SpokenMessage {
    fn from(normal: &str) -> Self {
        SpokenMessage::new(normal)
    }
}

impl From<String> for SpokenMessage {
    fn from(normal: String) -> Self {
        SpokenMessage::new(normal)
    }
}

impl PartialEq<str> for SpokenMessage {
    fn eq(&self, other: &str) -> bool {
        self.normal == other
    }
}

impl PartialEq<&str> for SpokenMessage {
    fn eq(&self, other: &&str) -> bool {
        self.normal == *other
    }
}

impl AsRef<str> for SpokenMessage {
    fn as_ref(&self) -> &str {
        &self.normal
    }
}

/// Color, confirmation, or coaching: spoken in normal mode, carried by an
/// earcon or by silence in terse mode.
pub fn terse_silent(normal: impl Into<String>) -> SpokenMessage {
    SpokenMessage::with_terse(normal, "")
}
