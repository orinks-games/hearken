//! The `hearken` command.
//!
//! `hearken play [game]` starts a game with one command. With Rust installed
//! it plays the latest source as a developer build (agent server and all):
//! it finds or clones the checkout, pulls, builds and starts it. Without
//! Rust, or with `--download`, it fetches the game's latest GitHub release
//! for this computer instead, so a player needs nothing but this command.
//!
//! `hearken release [game]` builds the zip players get: no agent server,
//! the game's files beside it, portable when the game says so.
//!
//! A game is named by its GitHub repository (`owner/name`, or its URL) or
//! by a folder on this computer; nothing about any game is built in. Inside
//! a game's own workspace the name can be left out.

mod doctor;
mod macos_app;
mod macos_sign;
mod new;
mod project;
mod release;
mod rules_module;
mod seal_key;
mod update_key;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use project::Project;

const USAGE: &str = "Usage: hearken new <name> [--template=menu|blank] [--title=TITLE]
       hearken play [game] [--download | --source]
       hearken release [game] [--channel=NAME] [--notes=FILE] [--portable | --no-portable]
       hearken seal-key [game] [--print]
       hearken update-key [game] [--print]
       hearken doctor

new        makes a game in a new folder called <name> (lowercase, hyphens):
           menu (the default) has a title menu, settings, help and a
           practice room; blank is one screen. Both have a sounds folder,
           tests and a CI workflow.
play       starts a game. With Rust installed it builds the latest source as a
           developer build; otherwise it downloads the latest release.
           --download and --source choose.
release    builds the zip players get, without the agent server, into dist/,
           and the rules module for verified runs when the game names one.
           A game with update = true also gets a signed update manifest
           for --channel (stable unless named), with --notes as its notes.
           --portable or --no-portable builds the other kind of copy for this
           release only: portable keeps saves beside the game, installed
           keeps them in the user's folder. The archive name then ends in
           -portable or -installed and no update manifest is written.
seal-key   shows where a game's seal key is kept and its ID; --print shows
           the key itself, for a CI secret.
update-key shows where a game's update key is kept and its public half;
           --print shows the secret, for a CI secret.
doctor     checks that this computer has what building a game from source
           needs, and says how to get what is missing.

A game is its GitHub repository, as owner/name or its URL, or a folder on
this computer. Inside a game's own folder it can be left out.";

/// Where a game comes from.
#[derive(Debug, PartialEq)]
enum Game {
    /// A game's workspace, or any folder inside it, on this computer.
    Folder(PathBuf),
    /// A GitHub repository, `owner/name`.
    Repo(String),
}

impl Game {
    /// A folder that exists, else a GitHub repository as `owner/name`,
    /// `github.com/owner/name` or its URL.
    fn parse(arg: &str) -> Result<Game, String> {
        let path = Path::new(arg);
        if path.is_dir() {
            return Ok(Game::Folder(path.to_path_buf()));
        }
        let repo = arg
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("github.com/")
            .trim_end_matches('/')
            .trim_end_matches(".git");
        match repo.split('/').collect::<Vec<_>>().as_slice() {
            [owner, name]
                if !owner.is_empty() && !name.is_empty() && !repo.contains(char::is_whitespace) =>
            {
                Ok(Game::Repo(format!("{owner}/{name}")))
            }
            _ => Err(format!(
                "{arg} is neither a folder here nor a GitHub repository (owner/name)."
            )),
        }
    }
}

/// The repository's own name: `name` in `owner/name`.
fn repo_name(repo: &str) -> &str {
    repo.rsplit('/').next().unwrap_or(repo)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let (names, flags): (Vec<&str>, Vec<&str>) =
        words.iter().skip(1).partition(|w| !w.starts_with("--"));
    let game = names.first().map(|n| Game::parse(n)).transpose();
    let result = match (words.first().copied(), names.len(), game) {
        (Some("play" | "release" | "seal-key" | "update-key"), _, Err(e)) => Err(e),
        (Some("play"), 0 | 1, Ok(game)) => play(game, &flags),
        (Some("release"), 0 | 1, Ok(game)) => match release_options(&flags) {
            Some(options) => release_game(game, &options),
            None => Err(USAGE.to_string()),
        },
        (Some("seal-key"), 0 | 1, Ok(game)) if matches!(flags[..], [] | ["--print"]) => {
            show_seal_key(game, !flags.is_empty())
        }
        (Some("update-key"), 0 | 1, Ok(game)) if matches!(flags[..], [] | ["--print"]) => {
            show_update_key(game, !flags.is_empty())
        }
        (Some("doctor"), 0, _) if flags.is_empty() => doctor::run(),
        // A new game's name is neither a folder nor a repository yet.
        (Some("new"), 1, _) => new::run(names[0], &flags),
        (Some("--version" | "-V"), 0, _) => {
            println!("hearken {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn play(game: Option<Game>, flags: &[&str]) -> Result<(), String> {
    let from_source = match flags {
        [] => {
            !matches!(game, Some(Game::Repo(_))) || run_ok(Command::new("cargo").arg("--version"))
        }
        ["--source"] => true,
        ["--download"] => false,
        _ => return Err(USAGE.to_string()),
    };
    // No flag was given and Rust was not found: downloading is a fallback
    // the player did not ask for, so a failure says so.
    let fell_back = flags.is_empty() && !from_source;
    let (dir, exe, title) = match (game, from_source) {
        (Some(Game::Repo(repo)), false) => {
            let (dir, exe) =
                download(&repo).map_err(|e| if fell_back { fallback_error(&e) } else { e })?;
            (dir, exe, repo_name(&repo).to_string())
        }
        (_, false) => return Err("Name the game's repository (owner/name) to download it.".into()),
        (None, true) => {
            // The game being worked on here: built as it is, no pull.
            let project = Project::load(Path::new("."), None)?;
            project.build(true)?;
            (project.dir.clone(), project.exe(), project.package)
        }
        (Some(Game::Folder(dir)), true) => {
            let project = Project::load(&dir, None)?;
            project.build(true)?;
            (project.dir.clone(), project.exe(), project.package)
        }
        (Some(Game::Repo(repo)), true) => {
            let dir = checkout(&repo, true)?;
            let project = Project::load(&dir, None)?;
            project.build(true)?;
            (dir, project.exe(), project.package)
        }
    };
    // Started from its own folder so the game finds its sounds and data, and
    // cut loose from this terminal so the prompt comes straight back.
    Command::new(&exe)
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Could not start {}: {e}", exe.display()))?;
    println!("{title} is starting.");
    Ok(())
}

/// `--channel=NAME` and `--notes=FILE`, each at most once, and at most one
/// of `--portable` and `--no-portable`; `None` for anything else.
fn release_options(flags: &[&str]) -> Option<release::Options> {
    let mut options = release::Options::default();
    let (mut channel, mut notes) = (None, None);
    for flag in flags {
        if let Some(portable) = match *flag {
            "--portable" => Some(true),
            "--no-portable" => Some(false),
            _ => None,
        } {
            if options.portable.is_some() {
                return None;
            }
            options.portable = Some(portable);
            continue;
        }
        let (slot, value) = match flag.split_once('=') {
            Some(("--channel", value)) => (&mut channel, value),
            Some(("--notes", value)) => (&mut notes, value),
            _ => return None,
        };
        if slot.is_some() || value.is_empty() {
            return None;
        }
        *slot = Some(value.to_string());
    }
    if let Some(channel) = channel {
        options.channel = channel;
    }
    options.notes = notes.map(PathBuf::from);
    Some(options)
}

/// The game a command that does not play it is about. A release builds
/// exactly what is checked out: no pull.
fn project_for(game: Option<Game>) -> Result<Project, String> {
    match game {
        Some(Game::Repo(repo)) => Project::load(&checkout(&repo, false)?, None),
        Some(Game::Folder(dir)) => Project::load(&dir, None),
        None => Project::load(Path::new("."), None),
    }
}

fn release_game(game: Option<Game>, options: &release::Options) -> Result<(), String> {
    let project = project_for(game)?;
    let archive = release::release(&project, options)?;
    println!("Packed {}", archive.display());
    Ok(())
}

fn show_seal_key(game: Option<Game>, print: bool) -> Result<(), String> {
    seal_key::command(&project_for(game)?, print)
}

fn show_update_key(game: Option<Game>, print: bool) -> Result<(), String> {
    update_key::command(&project_for(game)?, print)
}

/// The repository's checkout: the one holding the current folder, or else
/// a folder named after it in the home folder, cloned if missing. `pull`
/// brings it up to date first.
fn checkout(repo: &str, pull: bool) -> Result<PathBuf, String> {
    let name = repo_name(repo);
    let dir = match checkout_here(repo) {
        Some(dir) => dir,
        None => {
            let home = dirs::home_dir().ok_or("Could not find your home folder.")?;
            let dir = home.join(name);
            if !dir.exists() {
                println!("Getting {name} into {}...", dir.display());
                let url = format!("https://github.com/{repo}.git");
                if !run_ok(Command::new("git").arg("clone").arg(&url).arg(&dir)) {
                    return Err(clone_error(&url));
                }
            }
            dir
        }
    };
    if pull {
        println!("Getting the latest {name}...");
        if !run_ok(
            Command::new("git")
                .args(["pull", "--ff-only"])
                .current_dir(&dir),
        ) {
            println!("Could not pull, so building the copy already here.");
        }
    }
    Ok(dir)
}

/// What a failed clone says. Git's own message is shown above it.
fn clone_error(url: &str) -> String {
    format!(
        "Could not clone {url}. A private repository needs read access to it and Git signed in \
         to GitHub. The name may also be misspelled."
    )
}

/// What a failed look for the latest release says. `status` is the HTTP
/// status when GitHub answered, and `detail` is the underlying error.
fn release_list_error(repo: &str, status: Option<u16>, detail: &str) -> String {
    if status == Some(404) {
        format!(
            "{repo} has no published release to download. The repository may not have one yet, \
             it may be private, or its name may be misspelled. A private repository can only be \
             played by building it from source with --source, with Git signed in to GitHub."
        )
    } else {
        format!(
            "Could not reach the release list for {repo} ({detail}). Check your internet \
             connection and try again. If GitHub is limiting requests, try again later."
        )
    }
}

/// What a download that nobody asked for says when it fails: why hearken
/// tried it, and how to build from source instead.
fn fallback_error(error: &str) -> String {
    format!(
        "{error}\nRust was not found on this computer, so hearken looked for a release to \
         download instead. To build the game from source, install Rust (see Getting started in \
         Hearken's README, https://github.com/orinks-games/hearken#getting-started), open a new \
         terminal, and run the command again. Run hearken doctor to see what this computer is \
         missing."
    )
}

/// The git checkout holding the current folder, when it is `repo`'s.
fn checkout_here(repo: &str) -> Option<PathBuf> {
    let out = |args: &[&str]| {
        let o = Command::new("git").args(args).output().ok()?;
        o.status
            .success()
            .then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let remote = out(&["remote", "get-url", "origin"])?.to_lowercase();
    let matches = remote
        .trim_end_matches(".git")
        .ends_with(&repo.to_lowercase());
    matches.then(|| out(&["rev-parse", "--show-toplevel"]).map(PathBuf::from))?
}

/// Downloads and unpacks the latest release, once per version.
fn download(repo: &str) -> Result<(PathBuf, PathBuf), String> {
    let name = repo_name(repo);
    let api = format!("https://api.github.com/repos/{repo}/releases/latest");
    let release: serde_json::Value = ureq::get(&api)
        .set("User-Agent", "hearken-cli")
        .call()
        .map_err(|e| {
            let status = match &e {
                ureq::Error::Status(code, _) => Some(*code),
                _ => None,
            };
            release_list_error(repo, status, &e.to_string())
        })?
        .into_json()
        .map_err(|e| format!("Could not read the release list: {e}"))?;
    let tag = release["tag_name"].as_str().unwrap_or("latest");
    let assets: Vec<(&str, &str)> = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| Some((a["name"].as_str()?, a["browser_download_url"].as_str()?)))
        .collect();
    let names: Vec<&str> = assets.iter().map(|(n, _)| *n).collect();
    let pick = pick_asset(&names, std::env::consts::OS, std::env::consts::ARCH)
        .ok_or_else(|| format!("{name} {tag} has no download for this computer."))?;
    let (asset, url) = assets[pick];

    let base = dirs::data_local_dir().ok_or("Could not find your data folder.")?;
    let owner = repo.split('/').next().unwrap_or(name);
    let dir = base
        .join("hearken")
        .join("games")
        .join(owner)
        .join(name)
        .join(tag);
    if !dir.exists() {
        println!("Downloading {name} {tag}...");
        let partial = dir.with_extension("partial");
        let _ = std::fs::remove_dir_all(&partial);
        std::fs::create_dir_all(&partial).map_err(|e| e.to_string())?;
        let archive = partial.join(asset);
        let mut body = ureq::get(url)
            .set("User-Agent", "hearken-cli")
            .call()
            .map_err(|e| format!("The download failed: {e}"))?
            .into_reader();
        let mut file = std::fs::File::create(&archive).map_err(|e| e.to_string())?;
        std::io::copy(&mut body, &mut file).map_err(|e| format!("The download failed: {e}"))?;
        drop(file);
        // tar unpacks both .zip (Windows' bsdtar) and .tar.gz.
        if !run_ok(
            Command::new("tar")
                .arg("-xf")
                .arg(&archive)
                .current_dir(&partial),
        ) {
            return Err(format!("Could not unpack {asset}."));
        }
        let _ = std::fs::remove_file(&archive);
        std::fs::rename(&partial, &dir).map_err(|e| e.to_string())?;
    }
    let found = programs(&dir);
    let file_names: Vec<String> = found
        .iter()
        .map(|p| {
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let i = pick_program(&file_names, name).map_err(|e| format!("{asset}: {e}"))?;
    let exe = found[i].clone();
    let exe_dir = exe.parent().unwrap_or(&dir).to_path_buf();
    Ok((exe_dir, exe))
}

/// The release file for this OS and architecture. Linux takes a `.tar.gz`,
/// Windows and macOS a `.zip`, named for the OS (`linux`, `windows`/`win64`,
/// `macos`/`mac`/`osx`/`darwin`). Among those, in order: one built for this
/// processor; on a Mac a `universal` one; one that names no processor, as
/// Freight Fate's `windows-portable` and `macos` do, taken as x64; and on an
/// Arm Windows PC or Mac, which run x64 programs, an x64 one.
fn pick_asset(names: &[&str], os: &str, arch: &str) -> Option<usize> {
    let (ext, os_words): (&str, &[&str]) = match os {
        "linux" => (".tar.gz", &["linux"]),
        "windows" => (".zip", &["windows", "win64", "win"]),
        "macos" => (".zip", &["macos", "mac", "osx", "darwin"]),
        _ => return None,
    };
    let runs_x64 = arch == "x86_64" || (arch == "aarch64" && os != "linux");
    let rank = |name: &str| -> Option<u8> {
        let name = name.to_lowercase();
        let words: Vec<&str> = name
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .collect();
        if !name.ends_with(ext) || !words.iter().any(|w| os_words.contains(w)) {
            return None;
        }
        let built_for = words.iter().find_map(|w| match *w {
            "x64" | "x86_64" | "amd64" => Some("x86_64"),
            "arm64" | "aarch64" => Some("aarch64"),
            "x86" | "i386" | "i686" => Some("x86"),
            "universal" => Some("universal"),
            _ => None,
        });
        match built_for {
            Some(a) if a == arch => Some(0),
            Some("universal") if os == "macos" => Some(1),
            None if runs_x64 => Some(2),
            Some("x86_64") if runs_x64 => Some(3),
            _ => None,
        }
    };
    // A release built as the other kind (`-portable` or `-installed`) comes
    // after the game's own whatever the processor, so the pick does not
    // depend on the order the names are listed in.
    let other_kind = |name: &str| {
        let name = name.to_lowercase();
        name.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .any(|w| w == "portable" || w == "installed")
    };
    names
        .iter()
        .enumerate()
        .filter_map(|(i, n)| Some((other_kind(n), rank(n)?, i)))
        .min()
        .map(|(_, _, i)| i)
}

/// The programs in an unpacked release, from the shallowest folder that
/// holds any.
fn programs(dir: &Path) -> Vec<PathBuf> {
    let mut level = vec![dir.to_path_buf()];
    while !level.is_empty() {
        let mut found = Vec::new();
        let mut deeper = Vec::new();
        for d in level {
            for entry in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    deeper.push(path);
                } else if is_program(&path) {
                    found.push(path);
                }
            }
        }
        if !found.is_empty() {
            return found;
        }
        level = deeper;
    }
    Vec::new()
}

#[cfg(windows)]
fn is_program(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
}

#[cfg(not(windows))]
fn is_program(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.extension().is_none()
        && std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

/// The game's program among a release's programs: the only one, or the one
/// named like the repository (`freight-fate` finds `FreightFate.exe`).
fn pick_program(names: &[String], repo: &str) -> Result<usize, String> {
    let squash = |s: &str| {
        s.chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_lowercase()
    };
    match names {
        [] => return Err("it holds no program to run.".into()),
        [_] => return Ok(0),
        _ => {}
    }
    let want = squash(repo);
    let stem = |n: &str| squash(n.rsplit_once('.').map_or(n, |(s, _)| s));
    names.iter().position(|n| stem(n) == want).ok_or_else(|| {
        format!(
            "it holds several programs ({}) and none is named like {repo}.",
            names.join(", ")
        )
    })
}

fn run_ok(cmd: &mut Command) -> bool {
    cmd.status().is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FREIGHT_FATE: &[&str] = &[
        "checksums.txt",
        "FreightFate-v1.9.4.2-linux-aarch64.AppImage",
        "FreightFate-v1.9.4.2-linux-arm64.tar.gz",
        "FreightFate-v1.9.4.2-linux-x64.tar.gz",
        "FreightFate-v1.9.4.2-macos-arm64.zip",
        "FreightFate-v1.9.4.2-macos.zip",
        "FreightFate-v1.9.4.2-windows-portable.zip",
    ];

    #[test]
    fn picks_the_archive_for_each_platform() {
        let pick = |os, arch| pick_asset(FREIGHT_FATE, os, arch).map(|i| FREIGHT_FATE[i]);
        assert_eq!(
            pick("windows", "x86_64"),
            Some("FreightFate-v1.9.4.2-windows-portable.zip")
        );
        assert_eq!(
            pick("linux", "x86_64"),
            Some("FreightFate-v1.9.4.2-linux-x64.tar.gz")
        );
        assert_eq!(
            pick("linux", "aarch64"),
            Some("FreightFate-v1.9.4.2-linux-arm64.tar.gz")
        );
        assert_eq!(
            pick("macos", "aarch64"),
            Some("FreightFate-v1.9.4.2-macos-arm64.zip")
        );
        assert_eq!(
            pick("macos", "x86_64"),
            Some("FreightFate-v1.9.4.2-macos.zip")
        );
        assert_eq!(pick("freebsd", "x86_64"), None);
    }

    #[test]
    fn picks_a_hearken_release() {
        let names = &[
            "wickets-0.2.0-windows-x64.zip",
            "wickets-0.2.0-linux-x64.tar.gz",
        ];
        assert_eq!(pick_asset(names, "windows", "x86_64"), Some(0));
        assert_eq!(pick_asset(names, "linux", "x86_64"), Some(1));
        assert_eq!(pick_asset(names, "linux", "aarch64"), None);
    }

    #[test]
    fn picks_this_processor_before_another() {
        // Listed in name order, so arm64 comes first.
        let names = &[
            "game-1.0.0-windows-arm64.zip",
            "game-1.0.0-windows-x64.zip",
            "game-1.0.0-macos-arm64.zip",
            "game-1.0.0-macos.zip",
            "game-1.0.0-linux-arm64.tar.gz",
            "game-1.0.0-linux-x64.tar.gz",
        ];
        assert_eq!(pick_asset(names, "windows", "x86_64"), Some(1));
        assert_eq!(pick_asset(names, "windows", "aarch64"), Some(0));
        assert_eq!(pick_asset(names, "linux", "aarch64"), Some(4));
    }

    #[test]
    fn an_arm_windows_pc_or_mac_falls_back_to_x64() {
        let names = &[
            "game-windows-x64.zip",
            "game-macos.zip",
            "game-linux-x64.tar.gz",
        ];
        assert_eq!(pick_asset(names, "windows", "aarch64"), Some(0));
        assert_eq!(pick_asset(names, "macos", "aarch64"), Some(1));
        // Linux on Arm does not run x64 programs.
        assert_eq!(pick_asset(names, "linux", "aarch64"), None);
    }

    #[test]
    fn a_universal_mac_build_suits_either_mac() {
        let names = &["game-macos-universal.zip", "game-windows-x64.zip"];
        assert_eq!(pick_asset(names, "macos", "aarch64"), Some(0));
        assert_eq!(pick_asset(names, "macos", "x86_64"), Some(0));
    }

    #[test]
    fn a_game_is_a_repository_in_any_spelling() {
        for arg in [
            "orinks-games/echo-parlor",
            "github.com/orinks-games/echo-parlor",
            "https://github.com/orinks-games/echo-parlor.git",
            "https://github.com/orinks-games/echo-parlor/",
        ] {
            assert_eq!(
                Game::parse(arg),
                Ok(Game::Repo("orinks-games/echo-parlor".into())),
                "{arg}"
            );
        }
        assert!(
            Game::parse("echo-parlor").is_err(),
            "a bare name is not enough"
        );
        assert!(Game::parse("a/b/c").is_err());
        assert!(Game::parse("owner/my game").is_err());
    }

    #[test]
    fn release_takes_a_channel_and_notes() {
        assert_eq!(release_options(&[]), Some(release::Options::default()));
        let options = release_options(&["--channel=beta", "--notes=NOTES.md"]).unwrap();
        assert_eq!(options.channel, "beta");
        assert_eq!(options.notes, Some(PathBuf::from("NOTES.md")));
        assert_eq!(release_options(&["--channel=a", "--channel=b"]), None);
        assert_eq!(release_options(&["--notes="]), None);
        assert_eq!(release_options(&["--print"]), None);
    }

    #[test]
    fn release_takes_portable_or_not_but_not_both() {
        assert_eq!(release_options(&[]).unwrap().portable, None);
        assert_eq!(
            release_options(&["--portable"]).unwrap().portable,
            Some(true)
        );
        assert_eq!(
            release_options(&["--no-portable", "--channel=beta"])
                .unwrap()
                .portable,
            Some(false)
        );
        assert_eq!(release_options(&["--portable", "--no-portable"]), None);
        assert_eq!(release_options(&["--portable", "--portable"]), None);
        assert_eq!(release_options(&["--no-portable", "--no-portable"]), None);
        assert_eq!(release_options(&["--portable=yes"]), None);
    }

    #[test]
    fn the_games_own_kind_is_downloaded_when_both_are_there() {
        for names in [
            [
                "game-1.0.0-windows-x64-portable.zip",
                "game-1.0.0-windows-x64.zip",
            ],
            [
                "game-1.0.0-windows-x64.zip",
                "game-1.0.0-windows-x64-portable.zip",
            ],
        ] {
            let pick = pick_asset(&names, "windows", "x86_64").unwrap();
            assert_eq!(names[pick], "game-1.0.0-windows-x64.zip");
        }
        let names = [
            "game-1.0.0-linux-x64-installed.tar.gz",
            "game-1.0.0-linux-x64.tar.gz",
        ];
        assert_eq!(pick_asset(&names, "linux", "x86_64"), Some(1));
        // Only the other kind is there: still better than nothing.
        assert_eq!(
            pick_asset(
                &["game-1.0.0-windows-x64-portable.zip"],
                "windows",
                "x86_64"
            ),
            Some(0)
        );
    }

    #[test]
    fn a_missing_release_says_why_and_what_to_try() {
        let missing = release_list_error("me/game", Some(404), "ignored");
        assert!(missing.contains("no published release"), "{missing}");
        assert!(missing.contains("private"), "{missing}");
        assert!(missing.contains("misspelled"), "{missing}");
        assert!(!missing.contains("ignored"), "{missing}");
        for status in [Some(403), None] {
            let other = release_list_error("me/game", status, "connection refused");
            assert!(
                other.contains("Could not reach the release list"),
                "{other}"
            );
            assert!(other.contains("connection refused"), "{other}");
            assert!(!other.contains("misspelled"), "{other}");
        }
    }

    #[test]
    fn a_download_nobody_asked_for_points_at_rust() {
        let text = fallback_error("me/game has no published release to download.");
        assert!(
            text.starts_with("me/game has no published release"),
            "{text}"
        );
        assert!(text.contains("Rust was not found"), "{text}");
        assert!(text.contains("install Rust"), "{text}");
        assert!(text.contains("https://github.com/orinks-games/hearken#getting-started"));
        assert!(text.contains("open a new terminal"), "{text}");
        assert!(text.contains("hearken doctor"), "{text}");
    }

    #[test]
    fn a_failed_clone_mentions_private_repositories_and_spelling() {
        let text = clone_error("https://github.com/me/game.git");
        assert!(text.contains("Could not clone https://github.com/me/game.git."));
        assert!(text.contains("private"), "{text}");
        assert!(text.contains("Git signed in to GitHub"), "{text}");
        assert!(text.contains("misspelled"), "{text}");
    }

    #[test]
    fn a_game_can_be_a_folder() {
        let dir = std::env::temp_dir();
        assert_eq!(Game::parse(dir.to_str().unwrap()), Ok(Game::Folder(dir)));
    }

    #[test]
    fn the_program_is_the_only_one_or_the_one_named_like_the_repo() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(pick_program(&names(&["wickets.exe"]), "echo-parlor"), Ok(0));
        assert_eq!(
            pick_program(&names(&["updater.exe", "FreightFate.exe"]), "freight-fate"),
            Ok(1)
        );
        assert!(pick_program(&names(&["a.exe", "b.exe"]), "game").is_err());
        assert!(pick_program(&[], "game").is_err());
    }
}
