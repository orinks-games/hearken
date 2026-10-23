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

mod project;
mod release;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use project::Project;

const USAGE: &str = "Usage: hearken play [game] [--download | --source]
       hearken release [game]

play     starts a game. With Rust installed it builds the latest source as a
         developer build; otherwise it downloads the latest release.
         --download and --source choose.
release  builds the zip players get, without the agent server, into dist/.

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
        (Some("play" | "release"), _, Err(e)) => Err(e),
        (Some("play"), 0 | 1, Ok(game)) => play(game, &flags),
        (Some("release"), 0 | 1, Ok(game)) if flags.is_empty() => release_game(game),
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
    let (dir, exe, title) = match (game, from_source) {
        (Some(Game::Repo(repo)), false) => {
            let (dir, exe) = download(&repo)?;
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

fn release_game(game: Option<Game>) -> Result<(), String> {
    let project = match game {
        // A release builds exactly what is checked out: no pull.
        Some(Game::Repo(repo)) => Project::load(&checkout(&repo, false)?, None)?,
        Some(Game::Folder(dir)) => Project::load(&dir, None)?,
        None => Project::load(Path::new("."), None)?,
    };
    let archive = release::release(&project)?;
    println!("Packed {}", archive.display());
    Ok(())
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
                    return Err(format!("Could not clone {url}."));
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
        .map_err(|e| format!("{repo} has no release to download ({e})."))?
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

/// The release file for this OS and architecture: the `release::platform`
/// name, or Freight Fate's `windows-portable`.
fn pick_asset(names: &[&str], os: &str, arch: &str) -> Option<usize> {
    let platform = release::platform(os, arch);
    let wanted = match os {
        "windows" => vec!["windows".to_string(), ".zip".to_string()],
        "linux" => vec![format!("{platform}."), ".tar.gz".to_string()],
        "macos" => vec![format!("{platform}.zip")],
        _ => return None,
    };
    names.iter().position(|n| {
        let n = n.to_lowercase();
        wanted.iter().all(|w| n.contains(w.as_str()))
    })
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
