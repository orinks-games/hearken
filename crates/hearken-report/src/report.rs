//! The report itself: its fields, the limits a relay enforces, and the
//! text a player can read before it is sent.

use std::fmt::{self, Display, Formatter, Write as _};

use serde::{Deserialize, Serialize};

/// The most of a log a report carries. A longer log keeps its end, where
/// the lines leading up to a problem are.
pub const MAX_LOG_BYTES: usize = 1 << 20;
/// The most characters a description can have.
pub const MAX_DESCRIPTION_CHARS: usize = 20_000;
/// The most attachments a report can have.
pub const MAX_ATTACHMENTS: usize = 4;
/// The most details a report can carry.
pub const MAX_DETAILS: usize = 8;

/// A problem report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    /// The game's name, such as `Freight Fate`.
    pub app: String,
    /// The game's version.
    pub version: String,
    /// The commit the game was built from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// The operating system and processor.
    pub os: String,
    /// What the player says happened.
    pub description: String,
    /// Where the developer can reply, if the player gave an address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    /// Whether the game crashed on its last run.
    #[serde(default)]
    pub crashed: bool,
    /// Where the player was, as the game knows it: `Game: Tightrope`,
    /// `Screen: driving`. The first detail is the headline, and goes in the
    /// subject.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<Detail>,
    /// Logs and other text files.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
}

/// One named fact about where the player was: `("Game", "Tightrope")`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Detail {
    /// What the value is, such as `Game`.
    pub name: String,
    /// The value, such as `Tightrope`.
    pub value: String,
}

/// A text file sent with a report, such as a log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    /// The file name, such as `session.log`.
    pub name: String,
    /// The file's text.
    pub text: String,
}

impl Report {
    /// A report from `app` at `version`, on the operating system and
    /// processor this copy was built for.
    pub fn new(
        app: impl Into<String>,
        version: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Report {
            app: app.into(),
            version: version.into(),
            commit: None,
            os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            description: description.into(),
            reply_to: None,
            crashed: false,
            details: Vec::new(),
            attachments: Vec::new(),
        }
    }

    /// The commit the game was built from.
    pub fn with_commit(mut self, commit: impl Into<String>) -> Self {
        self.commit = Some(commit.into());
        self
    }

    /// Replaces the operating system description, such as with one that
    /// names its version.
    pub fn with_os(mut self, os: impl Into<String>) -> Self {
        self.os = os.into();
        self
    }

    /// Where the developer can reply. A blank address leaves it unset.
    pub fn with_reply_to(mut self, address: impl Into<String>) -> Self {
        let address = address.into();
        let address = address.trim();
        self.reply_to = (!address.is_empty()).then(|| address.to_string());
        self
    }

    /// Whether the game crashed on its last run.
    pub fn crashed(mut self, crashed: bool) -> Self {
        self.crashed = crashed;
        self
    }

    /// Adds a detail of where the player was. The first one added is the
    /// headline. A blank name or value is left out.
    pub fn with_detail(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let name = name.into();
        let value = value.into();
        if !name.trim().is_empty() && !value.trim().is_empty() {
            self.details.push(Detail {
                name: name.trim().to_string(),
                value: value.trim().to_string(),
            });
        }
        self
    }

    /// Adds every pair of `details` with [`Report::with_detail`].
    pub fn with_details<N: Into<String>, V: Into<String>>(
        mut self,
        details: impl IntoIterator<Item = (N, V)>,
    ) -> Self {
        for (name, value) in details {
            self = self.with_detail(name, value);
        }
        self
    }

    /// The headline detail's value, such as `Tightrope`, if there is one.
    pub fn headline(&self) -> Option<&str> {
        self.details.first().map(|d| d.value.as_str())
    }

    /// Attaches a log, keeping its last [`MAX_LOG_BYTES`] bytes. An empty
    /// log is not attached.
    pub fn with_log(mut self, name: impl Into<String>, text: &str) -> Self {
        if !text.is_empty() {
            self.attachments.push(Attachment {
                name: name.into(),
                text: tail(text, MAX_LOG_BYTES).to_string(),
            });
        }
        self
    }

    /// The subject line of the report's email: `[Freight Fate 1.9.6]
    /// crash: The window closed`, or `[Wicket's 1.6.0, Tightrope] The rope
    /// swayed` when the report has a headline detail.
    pub fn subject(&self) -> String {
        let first_line = self
            .description
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or_default();
        let summary: String = first_line.chars().take(80).collect();
        let kind = if self.crashed { "crash: " } else { "" };
        let headline = self
            .headline()
            .map(|h| format!(", {h}"))
            .unwrap_or_default();
        format!("[{} {}{headline}] {kind}{summary}", self.app, self.version)
    }

    /// The description, then the game and system details, as the email
    /// body shows them.
    pub fn body(&self) -> String {
        let mut body = format!(
            "{}\n\nApp: {} {}",
            self.description.trim(),
            self.app,
            self.version
        );
        if let Some(commit) = &self.commit {
            let _ = write!(body, " (commit {commit})");
        }
        for detail in &self.details {
            let _ = write!(body, "\n{}: {}", detail.name, detail.value);
        }
        let _ = write!(
            body,
            "\nSystem: {}\nCrashed on its last run: {}",
            self.os,
            if self.crashed { "yes" } else { "no" }
        );
        if let Some(reply_to) = &self.reply_to {
            let _ = write!(body, "\nReply to: {reply_to}");
        }
        body.push('\n');
        body
    }

    /// The whole report as text, attachments included: what a player reads
    /// before sending, and what is saved to a file when sending fails.
    pub fn to_text(&self) -> String {
        let mut text = format!("{}\n\n{}", self.subject(), self.body());
        for attachment in &self.attachments {
            let _ = write!(
                text,
                "\n--- {} ---\n{}\n",
                attachment.name,
                attachment.text.trim_end()
            );
        }
        text
    }

    /// Checks the report against the limits a relay enforces, and returns
    /// the first problem found.
    pub fn validate(&self) -> Result<(), Invalid> {
        self.check(true)
    }

    /// Checks a report sent ahead of the web form, which has no
    /// description yet; everything else is held to the same limits.
    pub fn validate_sent_ahead(&self) -> Result<(), Invalid> {
        self.check(false)
    }

    fn check(&self, needs_description: bool) -> Result<(), Invalid> {
        check_line("app", &self.app, 64)?;
        check_line("version", &self.version, 64)?;
        if let Some(commit) = &self.commit {
            check_line("commit", commit, 64)?;
        }
        check_line("os", &self.os, 200)?;
        if needs_description && self.description.trim().is_empty() {
            return Err(Invalid::Empty("description"));
        }
        if self.description.chars().count() > MAX_DESCRIPTION_CHARS {
            return Err(Invalid::TooLong("description"));
        }
        if let Some(reply_to) = &self.reply_to {
            check_line("reply_to", reply_to, 254)?;
            if !looks_like_address(reply_to) {
                return Err(Invalid::NotAnAddress);
            }
        }
        if self.details.len() > MAX_DETAILS {
            return Err(Invalid::TooMany("details"));
        }
        for detail in &self.details {
            check_line("detail name", &detail.name, 40)?;
            check_line("detail", &detail.value, 200)?;
        }
        if self.attachments.len() > MAX_ATTACHMENTS {
            return Err(Invalid::TooMany("attachments"));
        }
        for attachment in &self.attachments {
            check_line("attachment name", &attachment.name, 64)?;
            // Plain file names only: the name ends up in a mail header.
            if !attachment
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
            {
                return Err(Invalid::BadName);
            }
            if attachment.text.len() > MAX_LOG_BYTES {
                return Err(Invalid::TooLong("attachment"));
            }
        }
        Ok(())
    }
}

/// Whether `text` has the shape of an email address: something, `@`, a
/// domain with a dot, and no whitespace. Only the relay can tell whether
/// the domain takes mail.
pub(crate) fn looks_like_address(text: &str) -> bool {
    text.split_once('@').is_some_and(|(user, domain)| {
        !user.is_empty() && domain.contains('.') && !text.contains(char::is_whitespace)
    })
}

/// Why a report is not valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invalid {
    /// A required field is blank.
    Empty(&'static str),
    /// A field is longer than allowed.
    TooLong(&'static str),
    /// There are more of something than allowed.
    TooMany(&'static str),
    /// A one-line field has a line break or another control character.
    ControlCharacter(&'static str),
    /// The reply address is not an email address.
    NotAnAddress,
    /// An attachment name has characters other than letters, digits, `.`,
    /// `-` and `_`.
    BadName,
}

impl Display for Invalid {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Invalid::Empty(field) => write!(f, "{field} is empty"),
            Invalid::TooLong(field) => write!(f, "{field} is too long"),
            Invalid::TooMany(field) => write!(f, "too many {field}"),
            Invalid::ControlCharacter(field) => write!(f, "{field} has a control character"),
            Invalid::NotAnAddress => f.write_str("reply_to is not an email address"),
            Invalid::BadName => f.write_str(
                "an attachment name has characters other than letters, digits, '.', '-' and '_'",
            ),
        }
    }
}

impl std::error::Error for Invalid {}

fn check_line(field: &'static str, value: &str, max_chars: usize) -> Result<(), Invalid> {
    if value.trim().is_empty() {
        return Err(Invalid::Empty(field));
    }
    if value.chars().count() > max_chars {
        return Err(Invalid::TooLong(field));
    }
    if value.chars().any(char::is_control) {
        return Err(Invalid::ControlCharacter(field));
    }
    Ok(())
}

/// The last `max` bytes of `text`, moved forward to a character boundary.
pub fn tail(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Report {
        Report::new(
            "Freight Fate",
            "1.9.6",
            "The dock menu is empty.\nIt happens at every warehouse.",
        )
        .with_os("windows x86_64")
    }

    #[test]
    fn the_subject_is_the_first_line_and_says_when_it_crashed() {
        assert_eq!(
            report().subject(),
            "[Freight Fate 1.9.6] The dock menu is empty."
        );
        assert_eq!(
            report().crashed(true).subject(),
            "[Freight Fate 1.9.6] crash: The dock menu is empty."
        );
    }

    #[test]
    fn a_headline_detail_goes_in_the_subject_and_every_detail_in_the_body() {
        let report = report()
            .with_detail("Game", "Tightrope")
            .with_detail(" Mode ", "challenge ")
            .with_detail("", "nothing")
            .with_detail("Blank", "  ");
        assert_eq!(report.headline(), Some("Tightrope"));
        assert_eq!(
            report.subject(),
            "[Freight Fate 1.9.6, Tightrope] The dock menu is empty."
        );
        assert_eq!(
            report.clone().crashed(true).subject(),
            "[Freight Fate 1.9.6, Tightrope] crash: The dock menu is empty."
        );
        assert_eq!(
            report.body(),
            "The dock menu is empty.\nIt happens at every warehouse.\n\nApp: Freight Fate 1.9.6\nGame: Tightrope\nMode: challenge\nSystem: windows x86_64\nCrashed on its last run: no\n"
        );
        assert!(report.validate().is_ok());
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains(r#""details":[{"name":"Game","value":"Tightrope"}"#));
        let back: Report = serde_json::from_str(&json).unwrap();
        assert_eq!(back, report);
        // A report from before details existed still reads.
        let stripped = json.replace(
            r#","details":[{"name":"Game","value":"Tightrope"},{"name":"Mode","value":"challenge"}]"#,
            "",
        );
        assert_ne!(stripped, json);
        let old: Report = serde_json::from_str(&stripped).unwrap();
        assert!(old.details.is_empty());
    }

    #[test]
    fn details_are_held_to_their_limits() {
        let mut many = report();
        for i in 0..=MAX_DETAILS {
            many = many.with_detail(format!("d{i}"), "v");
        }
        assert_eq!(many.validate(), Err(Invalid::TooMany("details")));
        let long = report().with_detail("Game", "x".repeat(201));
        assert_eq!(long.validate(), Err(Invalid::TooLong("detail")));
        let broken = report().with_detail("Game", "two\nlines");
        assert_eq!(broken.validate(), Err(Invalid::ControlCharacter("detail")));
    }

    #[test]
    fn the_body_lists_the_details() {
        let body = report()
            .with_commit("abc123")
            .with_reply_to("driver@example.com")
            .body();
        assert_eq!(
            body,
            "The dock menu is empty.\nIt happens at every warehouse.\n\nApp: Freight Fate 1.9.6 (commit abc123)\nSystem: windows x86_64\nCrashed on its last run: no\nReply to: driver@example.com\n"
        );
    }

    #[test]
    fn a_long_log_keeps_its_end() {
        let log = format!("é{}", "x".repeat(MAX_LOG_BYTES));
        let report = report().with_log("session.log", &log);
        assert_eq!(report.attachments[0].text.len(), MAX_LOG_BYTES);
        assert!(report.attachments[0].text.ends_with('x'));
        assert!(report.validate().is_ok());
    }

    #[test]
    fn blank_logs_and_reply_addresses_are_left_out() {
        let report = report().with_log("session.log", "").with_reply_to("  ");
        assert_eq!(report.attachments, []);
        assert_eq!(report.reply_to, None);
    }

    #[test]
    fn a_line_break_in_a_header_field_is_refused() {
        let mut report = report();
        report.version = "1.9.6\nBcc: someone@example.com".into();
        assert_eq!(report.validate(), Err(Invalid::ControlCharacter("version")));
    }

    #[test]
    fn the_reply_address_must_look_like_one() {
        assert_eq!(
            report().with_reply_to("not an address").validate(),
            Err(Invalid::NotAnAddress)
        );
        assert_eq!(
            report().with_reply_to("driver@localhost").validate(),
            Err(Invalid::NotAnAddress)
        );
        assert!(report()
            .with_reply_to("driver@example.com")
            .validate()
            .is_ok());
    }

    #[test]
    fn attachment_names_stay_plain() {
        assert_eq!(
            report().with_log("../secrets", "x").validate(),
            Err(Invalid::BadName)
        );
    }

    #[test]
    fn a_report_sent_ahead_may_have_no_description_but_nothing_else_slides() {
        let ahead = Report::new("Freight Fate", "1.9.6", "");
        assert_eq!(ahead.validate(), Err(Invalid::Empty("description")));
        assert!(ahead.validate_sent_ahead().is_ok());
        assert_eq!(
            ahead.with_reply_to("nope").validate_sent_ahead(),
            Err(Invalid::NotAnAddress)
        );
    }

    #[test]
    fn an_empty_description_is_refused() {
        assert_eq!(
            Report::new("Freight Fate", "1.9.6", " \n ").validate(),
            Err(Invalid::Empty("description"))
        );
    }

    #[test]
    fn the_text_ends_with_the_attachments() {
        let text = report().with_log("session.log", "line one\n").to_text();
        assert!(text.starts_with("[Freight Fate 1.9.6] The dock menu is empty.\n\nThe dock menu"));
        assert!(text.ends_with("\n--- session.log ---\nline one\n"));
    }

    #[test]
    fn a_report_survives_json() {
        let report = report()
            .with_commit("abc")
            .crashed(true)
            .with_log("session.log", "x");
        let json = serde_json::to_vec(&report).unwrap();
        assert_eq!(serde_json::from_slice::<Report>(&json).unwrap(), report);
    }

    #[test]
    fn the_json_matches_the_relay_protocol() {
        let json = serde_json::to_value(report().with_log("session.log", "x")).unwrap();
        assert_eq!(json["app"], "Freight Fate");
        assert_eq!(json["version"], "1.9.6");
        assert_eq!(json["os"], "windows x86_64");
        assert_eq!(json["crashed"], false);
        assert_eq!(json["attachments"][0]["name"], "session.log");
        assert_eq!(json["attachments"][0]["text"], "x");
        assert!(json.get("commit").is_none());
        assert!(json.get("reply_to").is_none());
    }

    #[test]
    fn tail_lands_on_a_character_boundary() {
        assert_eq!(tail("abc", 10), "abc");
        assert_eq!(tail("ééé", 3), "é");
        assert_eq!(tail("ééé", 4), "éé");
    }
}
