//! `hearken new <name> [--template=blank|menu] [--title=TITLE]`: start a
//! game in a new folder.
//!
//! `menu` is `template/` (hello-hearken: a title menu, settings, help and a
//! practice room) with its names changed to the new game's. `blank` is one
//! screen that speaks, plays a sound and quits. Both come with a `sounds`
//! folder, the test that keeps the program from opening a console window,
//! and a CI workflow.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Template {
    Blank,
    Menu,
}

impl Template {
    pub fn parse(s: &str) -> Result<Template, String> {
        match s {
            "blank" => Ok(Template::Blank),
            "menu" => Ok(Template::Menu),
            _ => Err(format!("No template called {s}. Choose blank or menu.")),
        }
    }
}

/// A game's names, each in the form its place in the code wants.
#[derive(Debug, PartialEq, Eq)]
pub struct Names {
    /// The package and program: `my-game`.
    pub package: String,
    /// The library crate: `my_game`.
    pub krate: String,
    /// Shown and spoken: `My Game`.
    pub title: String,
    /// Types and the save folder: `MyGame`.
    pub camel: String,
    /// Environment variables: `MY_GAME`.
    pub env: String,
}

impl Names {
    /// From a package name (`my-game`): lowercase letters, digits and
    /// hyphens, starting with a letter. The title defaults to its words.
    pub fn new(package: &str, title: Option<&str>) -> Result<Names, String> {
        let ok = package.starts_with(|c: char| c.is_ascii_lowercase())
            && package
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && !package.ends_with('-')
            && !package.contains("--");
        if !ok {
            return Err(format!(
                "{package} can't be a game's name: use lowercase letters, digits and \
                 single hyphens, starting with a letter, like my-game."
            ));
        }
        let words: Vec<String> = package
            .split('-')
            .map(|w| {
                let mut c = w.chars();
                c.next()
                    .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
                    .unwrap_or_default()
            })
            .collect();
        Ok(Names {
            package: package.to_string(),
            krate: package.replace('-', "_"),
            title: title.map(str::to_string).unwrap_or_else(|| words.join(" ")),
            camel: words.concat(),
            env: package.replace('-', "_").to_ascii_uppercase(),
        })
    }

    /// The template's names replaced by this game's.
    fn apply(&self, text: &str) -> String {
        text.replace("hello-hearken", &self.package)
            .replace("hello_hearken", &self.krate)
            .replace("Hello Hearken", &self.title)
            .replace("HelloHearken", &self.camel)
            .replace("HELLO_HEARKEN", &self.env)
            .replace("HelloGame", &format!("{}Game", self.camel))
            .replace("HelloAction", &format!("{}Action", self.camel))
    }
}

/// `template/`, compiled in.
const MENU: [(&str, &str); 8] = [
    ("build.rs", include_str!("../../../template/build.rs")),
    ("help.md", include_str!("../../../template/help.md")),
    ("src/main.rs", include_str!("../../../template/src/main.rs")),
    ("src/lib.rs", include_str!("../../../template/src/lib.rs")),
    (
        "src/title.rs",
        include_str!("../../../template/src/title.rs"),
    ),
    (
        "src/practice.rs",
        include_str!("../../../template/src/practice.rs"),
    ),
    (
        "src/sound.rs",
        include_str!("../../../template/src/sound.rs"),
    ),
    (
        "tests/walkthrough.rs",
        include_str!("../../../template/tests/walkthrough.rs"),
    ),
];

/// The blank game, in the menu template's names so `apply` renames both.
const BLANK: [(&str, &str); 4] = [
    ("help.md", include_str!("new/blank/help.md")),
    ("src/main.rs", include_str!("new/blank/main.rs")),
    ("src/lib.rs", include_str!("new/blank/lib.rs")),
    ("tests/start.rs", include_str!("new/blank/start.rs")),
];

/// Files every new game gets.
const COMMON: [(&str, &str); 4] = [
    (".gitignore", "/target\n/dist\n"),
    ("sounds/README.md", include_str!("new/sounds-readme.md")),
    (
        "tests/windows_subsystem.rs",
        include_str!("new/windows_subsystem.rs"),
    ),
    (".github/workflows/ci.yml", include_str!("new/ci.yml")),
];

fn cargo_toml(names: &Names, template: Template) -> String {
    let description = match template {
        Template::Blank => "A Hearken game",
        Template::Menu => "A Hearken game: a title menu, settings, help and a practice room",
    };
    format!(
        r#"[package]
name = "{package}"
description = "{description}"
version = "0.1.0"
edition = "2021"
rust-version = "{rust}"
publish = false
build = "build.rs"

[dependencies]
# Cargo.lock pins the engine's commit; `cargo update -p hearken` moves it.
hearken = {{ git = "https://github.com/orinks-games/hearken", branch = "main" }}

[package.metadata.hearken]
# Shipped beside the program in a release.
include = ["sounds"]
"#,
        package = names.package,
        rust = env!("CARGO_PKG_RUST_VERSION"),
    )
}

/// Every file of a new game, as (path in the game folder, contents).
pub fn files(template: Template, names: &Names) -> Vec<(String, String)> {
    let own: &[(&str, &str)] = match template {
        Template::Blank => &BLANK,
        Template::Menu => &MENU,
    };
    let mut out = vec![("Cargo.toml".to_string(), cargo_toml(names, template))];
    if template == Template::Blank {
        out.push(("build.rs".to_string(), MENU[0].1.to_string()));
    }
    out.extend(
        own.iter()
            .chain(COMMON.iter())
            .map(|(path, text)| (path.to_string(), names.apply(text))),
    );
    out
}

/// Write a new game into `dir`, which must not exist or be empty.
pub fn create(dir: &Path, template: Template, names: &Names) -> Result<Vec<PathBuf>, String> {
    if dir.read_dir().is_ok_and(|mut d| d.next().is_some()) {
        return Err(format!(
            "{} already has files in it. Choose a new folder.",
            dir.display()
        ));
    }
    let mut written = Vec::new();
    for (rel, text) in files(template, names) {
        let path = dir.join(&rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
        }
        std::fs::write(&path, text)
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
        written.push(path);
    }
    Ok(written)
}

/// The `hearken new` command.
pub fn run(name: &str, flags: &[&str]) -> Result<(), String> {
    let mut template = Template::Menu;
    let mut title = None;
    for flag in flags {
        if let Some(t) = flag.strip_prefix("--template=") {
            template = Template::parse(t)?;
        } else if let Some(t) = flag.strip_prefix("--title=") {
            title = Some(t);
        } else {
            return Err(crate::USAGE.to_string());
        }
    }
    let names = Names::new(name, title)?;
    let dir = PathBuf::from(&names.package);
    create(&dir, template, &names)?;
    println!(
        "Made {} in {}. Start it with: cd {} && hearken play",
        names.title,
        dir.display(),
        names.package
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_take_every_form_and_bad_names_are_refused() {
        let n = Names::new("cave-run2", None).unwrap();
        assert_eq!(
            (
                n.krate.as_str(),
                n.title.as_str(),
                n.camel.as_str(),
                n.env.as_str()
            ),
            ("cave_run2", "Cave Run2", "CaveRun2", "CAVE_RUN2")
        );
        assert_eq!(Names::new("x", Some("The X")).unwrap().title, "The X");
        for bad in ["My-Game", "2fast", "a--b", "a-", "a_b", ""] {
            assert!(Names::new(bad, None).is_err(), "{bad}");
        }
    }

    #[test]
    fn no_template_name_survives_in_a_new_game() {
        let names = Names::new("cave-run", None).unwrap();
        for template in [Template::Blank, Template::Menu] {
            let files = files(template, &names);
            for (path, text) in &files {
                for old in ["hello", "Hello", "HELLO"] {
                    assert!(!text.contains(old), "{template:?} {path} still says {old}");
                }
            }
            let main = &files.iter().find(|(p, _)| p == "src/main.rs").unwrap().1;
            assert!(main.starts_with("#![cfg_attr(windows, windows_subsystem = \"windows\")]"));
            let test = &files
                .iter()
                .find(|(p, _)| p == "tests/windows_subsystem.rs")
                .unwrap()
                .1;
            assert!(test.contains("CARGO_BIN_EXE_cave-run"));
        }
    }

    #[test]
    fn create_refuses_a_folder_with_files() {
        let dir = tempfile::tempdir().unwrap();
        let names = Names::new("cave-run", None).unwrap();
        std::fs::write(dir.path().join("keep.txt"), "mine").unwrap();
        assert!(create(dir.path(), Template::Blank, &names).is_err());
        let fresh = dir.path().join("game");
        let written = create(&fresh, Template::Blank, &names).unwrap();
        assert!(written.iter().all(|p| p.is_file()));
        assert!(fresh.join("sounds/README.md").is_file());
    }
}
