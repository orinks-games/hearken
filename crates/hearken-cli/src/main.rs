//! The `hearken` command.
//!
//! `hearken play <game>` starts a game with one command. With Rust installed
//! it plays the latest source as a developer build (agent server and all):
//! it finds or clones the checkout, pulls, builds and starts it. Without
//! Rust, or with `--download`, it fetches the game's latest GitHub release
//! for this computer instead, so a player needs nothing but this command.
//!
//! `hearken release <game>` builds the zip players get: no agent server,
//! the game's files beside it, portable when the game says so.
//!
//! Inside a game's own workspace the name can be left out.

mod project;
mod release;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use project::Project;

/// A game the command knows by name.
struct Game {
    name: &'static str,
    /// The GitHub repository, `owner/name`.
    repo: &'static str,
    /// The Cargo package that starts the game.
    package: &'static str,
    /// Its program, for finding it in a downloaded release.
    bin: &'static str,
}

const GAMES: &[Game] = &[
    Game {
        name: "echo-parlor",
        repo: "orinks-games/echo-parlor",
        package: "echo-parlor",
        bin: "echo-parlor",
    },
    Game {
        name: "freight-fate",
        repo: "orinks-games/freight-fate",
        package: "freight-fate",
        bin: "freightfate",
    },
    Game {
        name: "hello-hearken",
        repo: "orinks-games/hearken",
        package: "hello-hearken",
        bin: "hello-hearken",
    },
];

const USAGE: &str = "Usage: hearken play [game] [--download | --source]
       hearken release [game]
       hearken list

play     starts a game. With Rust installed it builds the latest source as a
         developer build; otherwise it downloads the latest release.
         --download and --source choose.
release  builds the zip players get, without the agent server, into dist/.

Inside a game's own folder the name can be left out.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let (names, flags): (Vec<&str>, Vec<&str>) =
        words.iter().skip(1).partition(|w| !w.starts_with("--"));
    let result = match (words.first().copied(), names.as_slice()) {
        (Some("list"), []) => {
            for game in GAMES {
                println!("{}", game.name);
            }
            Ok(())
        }
        (Some("play"), [] | [_]) => play(names.first().copied(), &flags),
        (Some("release"), [] | [_]) if flags.is_empty() => release_game(names.first().copied()),
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

fn find(name: &str) -> Result<&'static Game, String> {
    GAMES.iter().find(|g| g.name == name).ok_or_else(|| {
        let names: Vec<_> = GAMES.iter().map(|g| g.name).collect();
        format!("No game named {name}. Games: {}", names.join(", "))
    })
}

fn play(name: Option<&str>, flags: &[&str]) -> Result<(), String> {
    let game = name.map(find).transpose()?;
    let from_source = match flags {
        [] => game.is_none() || run_ok(Command::new("cargo").arg("--version")),
        ["--source"] => true,
        ["--download"] => false,
        _ => return Err(USAGE.to_string()),
    };
    let (dir, exe, title) = match (game, from_source) {
        (None, _) if flags == ["--download"] => {
            return Err("Name the game to download.".to_string())
        }
        (None, _) => {
            // The game being worked on here: built as it is, no pull.
            let project = Project::load(Path::new("."), None)?;
            project.build(true)?;
            (project.dir.clone(), project.exe(), project.package)
        }
        (Some(game), true) => {
            let dir = checkout(game, true)?;
            let project = Project::load(&dir, Some(game.package))?;
            project.build(true)?;
            (dir, project.exe(), game.name.to_string())
        }
        (Some(game), false) => {
            let (dir, exe) = download(game)?;
            (dir, exe, game.name.to_string())
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

fn release_game(name: Option<&str>) -> Result<(), String> {
    let project = match name.map(find).transpose()? {
        // A release builds exactly what is checked out: no pull.
        Some(game) => Project::load(&checkout(game, false)?, Some(game.package))?,
        None => Project::load(Path::new("."), None)?,
    };
    let archive = release::release(&project)?;
    println!("Packed {}", archive.display());
    Ok(())
}

/// The game's checkout: the one holding the current folder, or else a
/// folder named after the repository in the home folder, cloned if missing.
/// `pull` brings it up to date first.
fn checkout(game: &Game, pull: bool) -> Result<PathBuf, String> {
    let dir = match checkout_here(game) {
        Some(dir) => dir,
        None => {
            let home = dirs::home_dir().ok_or("Could not find your home folder.")?;
            let dir = home.join(game.repo.rsplit('/').next().unwrap_or(game.name));
            if !dir.exists() {
                println!("Getting {} into {}...", game.name, dir.display());
                let url = format!("https://github.com/{}.git", game.repo);
                if !run_ok(Command::new("git").arg("clone").arg(&url).arg(&dir)) {
                    return Err(format!("Could not clone {url}."));
                }
            }
            dir
        }
    };
    if pull {
        println!("Getting the latest {}...", game.name);
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

/// The git checkout holding the current folder, when it is this game's.
fn checkout_here(game: &Game) -> Option<PathBuf> {
    let out = |args: &[&str]| {
        let o = Command::new("git").args(args).output().ok()?;
        o.status
            .success()
            .then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let remote = out(&["remote", "get-url", "origin"])?.to_lowercase();
    let repo = game.repo.to_lowercase();
    let matches = remote.trim_end_matches(".git").ends_with(&repo);
    matches.then(|| out(&["rev-parse", "--show-toplevel"]).map(PathBuf::from))?
}

/// Downloads and unpacks the latest release, once per version.
fn download(game: &Game) -> Result<(PathBuf, PathBuf), String> {
    let api = format!("https://api.github.com/repos/{}/releases/latest", game.repo);
    let release: serde_json::Value = ureq::get(&api)
        .set("User-Agent", "hearken-cli")
        .call()
        .map_err(|e| format!("{} has no release to download ({e}).", game.name))?
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
        .ok_or_else(|| format!("{} {tag} has no download for this computer.", game.name))?;
    let (asset, url) = assets[pick];

    let base = dirs::data_local_dir().ok_or("Could not find your data folder.")?;
    let dir = base.join("hearken").join("games").join(game.name).join(tag);
    if !dir.exists() {
        println!("Downloading {} {tag}...", game.name);
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
    let exe = find_exe(&dir, game.bin)
        .ok_or_else(|| format!("{asset} holds no program named {}.", game.bin))?;
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

/// The game's program anywhere under `dir`, matched without regard to case.
fn find_exe(dir: &Path, bin: &str) -> Option<PathBuf> {
    let want = format!("{bin}{}", std::env::consts::EXE_SUFFIX).to_lowercase();
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_exe(&path, bin) {
                return Some(found);
            }
        } else if entry.file_name().to_string_lossy().to_lowercase() == want {
            return Some(path);
        }
    }
    None
}

fn run_ok(cmd: &mut Command) -> bool {
    cmd.status().is_ok_and(|s| s.success())
}

#[cfg(test)]
mod tests {
    use super::pick_asset;

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
            "echo-parlor-0.2.0-windows-x64.zip",
            "echo-parlor-0.2.0-linux-x64.tar.gz",
        ];
        assert_eq!(pick_asset(names, "windows", "x86_64"), Some(0));
        assert_eq!(pick_asset(names, "linux", "x86_64"), Some(1));
        assert_eq!(pick_asset(names, "linux", "aarch64"), None);
    }
}
