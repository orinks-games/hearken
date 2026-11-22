//! Translation tooling for Hearken games: build a game's template of
//! spoken lines from its source, keep every language file in step with it,
//! and check translations for mistakes the game cannot fix.
//!
//! The runtime side (matching finished lines, loading catalogs) is
//! `hearken-l10n`; this crate is for a game's own tools binary or tests.
//! A game describes itself once as a [`Project`]:
//!
//! ```no_run
//! use hearken_l10n_tools::{extract, check, Project};
//!
//! let project = Project {
//!     root: ".".into(),
//!     lang_dir: "lang".into(),
//!     template_name: "my-game.pot".into(),
//!     header: Project::header_for("My Game", "https://example.com/issues"),
//!     sources: vec!["src".into()],
//!     ..Project::default()
//! };
//! std::process::exit(extract::run(&project, false, "cargo run -p my-tools").unwrap());
//! # let _ = check::run(&project, true);
//! ```

pub mod check;
pub mod extract;
pub mod po;
pub mod rust_strings;
pub mod util;

use std::collections::HashSet;
use std::path::PathBuf;

pub use po::Entry;

/// A rule a game adds to the checker: `(language code, English, translation)`
/// to the problems found, each a short phrase ("names a song").
pub type Rule = fn(&str, &str, &str) -> Vec<String>;

/// One template and its language files.
#[derive(Default)]
pub struct Project {
    /// The checkout the paths below are relative to (for `#:` references
    /// and for finding `sources`).
    pub root: PathBuf,
    /// Where the template, `extra.pot` and each `<code>.po` live.
    pub lang_dir: PathBuf,
    /// The template's file name, such as `my-game.pot`.
    pub template_name: String,
    /// Header fields every file starts with (see [`Project::header_for`]).
    pub header: String,
    /// Folders scanned for `.rs` files, relative to `root`.
    pub sources: Vec<PathBuf>,
    /// More path fragments to pass over, on top of
    /// [`rust_strings::SKIP_PATH_PARTS`].
    pub skip_paths: Vec<String>,
    /// Lines the game found itself: help pages, story, data files. Each
    /// entry's `refs` names where it came from.
    pub entries: Vec<Entry>,
    /// English lines to leave out because another template has them.
    pub exclude: HashSet<String>,
    /// The game's own checks, run on every translated entry.
    pub rules: Vec<Rule>,
}

impl Project {
    /// The usual PO header for `name`, with bug reports going to `bugs`.
    pub fn header_for(name: &str, bugs: &str) -> String {
        format!(
            "Project-Id-Version: {name}\n\
             Report-Msgid-Bugs-To: {bugs}\n\
             MIME-Version: 1.0\n\
             Content-Type: text/plain; charset=UTF-8\n\
             Content-Transfer-Encoding: 8bit\n"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_extracts_merges_and_checks() {
        let dir = std::env::temp_dir().join(format!("hearken-l10n-tools-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("lang")).unwrap();
        std::fs::write(
            dir.join("src/lib.rs"),
            "fn f(ctx: &mut Ctx, n: u32) {\n    ctx.say(format!(\"You have {n} lamps.\"));\n    ctx.say(\"Shared line.\");\n    log::info!(\"Not spoken at all.\");\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("lang/es.po"),
            "msgid \"You have {n} lamps.\"\nmsgstr \"Tienes {m} faroles.\"\n",
        )
        .unwrap();
        let mut page = Entry::new("Press Enter to start.");
        page.refs = vec!["help.md".into()];
        let project = Project {
            root: dir.clone(),
            lang_dir: dir.join("lang"),
            template_name: "demo.pot".into(),
            header: Project::header_for("Demo", "https://example.com"),
            sources: vec!["src".into()],
            entries: vec![page],
            exclude: ["Shared line.".to_string()].into_iter().collect(),
            ..Project::default()
        };
        assert_eq!(extract::run(&project, true, "x").unwrap(), 1);
        assert_eq!(extract::run(&project, false, "x").unwrap(), 0);
        assert_eq!(extract::run(&project, true, "x").unwrap(), 0);
        let pot = std::fs::read_to_string(dir.join("lang/demo.pot")).unwrap();
        assert!(pot.contains("msgid \"You have {n} lamps.\""));
        assert!(pot.contains("msgid \"Press Enter to start.\""));
        assert!(!pot.contains("Shared line."));
        assert!(!pot.contains("Not spoken"));
        let es = std::fs::read_to_string(dir.join("lang/es.po")).unwrap();
        assert!(es.contains("Tienes {m} faroles."), "{es}");
        assert!(dir.join("lang/fr.po").exists());
        let (problems, total, translated) =
            check::check_file(&project, "es", &dir.join("lang/es.po")).unwrap();
        assert_eq!((total, translated), (2, 1));
        assert_eq!(problems.len(), 2, "{problems:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
