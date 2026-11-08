//! The real Windows apply script and recovery, run against real folders.
//! Windows refuses to replace a file another handle holds open, which is
//! what a game still exiting or antivirus does to an update.

use std::fs;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::apply::{write_apply_script, APPLY_LOG, JOURNAL};
use crate::install::{Install, Os, Staged};
use crate::recover::{recover_interrupted, Recovery, LEFTOVERS};

/// A small program that exits at once, standing in for the game the
/// script starts again.
fn quick_program() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    PathBuf::from(root).join("System32").join("where.exe")
}

/// A game folder and a staged update for it, with the script written.
fn staged(root: &Path) -> (PathBuf, PathBuf, Install, PathBuf) {
    let game = root.join("game");
    fs::create_dir_all(game.join("saves")).unwrap();
    fs::write(game.join("saves/progress.json"), b"mine").unwrap();
    fs::write(game.join("data.txt"), b"old").unwrap();
    fs::write(game.join("sounds.pak"), b"old sounds").unwrap();
    fs::write(game.join("my-game.exe"), b"old program").unwrap();
    let staging = crate::install::make_staging_dir_in(root, "my-game").unwrap();
    let new = staging.join("unpacked\\g");
    fs::create_dir_all(&new).unwrap();
    fs::write(new.join("data.txt"), b"new").unwrap();
    fs::write(new.join("sounds.pak"), b"new sounds").unwrap();
    fs::copy(quick_program(), new.join("my-game.exe")).unwrap();
    let install = Install::Folder {
        dir: game.clone(),
        exe: "my-game.exe".into(),
    };
    let script = write_apply_script(
        &Staged::Folder(new),
        &install,
        &staging,
        999_999_999,
        Os::Windows,
    )
    .unwrap();
    (game, staging, install, script)
}

fn run(script: &Path) {
    let status = Command::new("cmd").arg("/c").arg(script).status().unwrap();
    let _ = status;
}

/// Opens `path` so nobody else can read, write or replace it.
fn lock(path: &Path) -> fs::File {
    fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .unwrap()
}

#[test]
fn the_windows_script_really_updates_a_folder() {
    let dir = tempfile::tempdir().unwrap();
    let (game, staging, install, script) = staged(dir.path());
    run(&script);
    assert_eq!(fs::read(game.join("data.txt")).unwrap(), b"new");
    assert_eq!(fs::read(game.join("sounds.pak")).unwrap(), b"new sounds");
    assert_eq!(
        fs::read(game.join("my-game.exe")).unwrap(),
        fs::read(quick_program()).unwrap()
    );
    assert_eq!(fs::read(game.join("saves/progress.json")).unwrap(), b"mine");
    assert!(!game.join(JOURNAL).exists());
    assert!(!staging.exists());
    let log = fs::read_to_string(game.join(APPLY_LOG)).unwrap();
    assert!(log.contains("the update is in"), "{log}");
    assert_eq!(recover_interrupted(&install).unwrap(), Recovery::Nothing);
    assert!(!game.join(APPLY_LOG).exists());
}

/// A file held open for a few seconds after the game is gone (antivirus
/// reading it) is waited out, and the update still goes in.
#[test]
fn a_file_locked_for_a_moment_is_retried() {
    let dir = tempfile::tempdir().unwrap();
    let (game, staging, _install, script) = staged(dir.path());
    let held = lock(&game.join("sounds.pak"));
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(4));
        drop(held);
    });
    run(&script);
    release.join().unwrap();
    assert_eq!(fs::read(game.join("sounds.pak")).unwrap(), b"new sounds");
    assert!(!game.join(JOURNAL).exists());
    assert!(!staging.exists());
}

/// The program held open, as a running game holds itself: the next start
/// puts the rest of the folder back, never failing on the program, and the
/// differing file it cannot replace is renamed aside and removed later.
#[test]
fn recovery_works_around_files_in_use() {
    let dir = tempfile::tempdir().unwrap();
    let game = dir.path().join("game");
    fs::create_dir_all(&game).unwrap();
    let staging = crate::install::make_staging_dir_in(dir.path(), "my-game").unwrap();
    let backup = staging.join("backup");
    fs::create_dir_all(&backup).unwrap();
    fs::write(game.join("my-game.exe"), b"old program").unwrap();
    fs::write(backup.join("my-game.exe"), b"old program").unwrap();
    fs::write(game.join("SDL2.dll"), b"new library").unwrap();
    fs::write(backup.join("SDL2.dll"), b"old library").unwrap();
    fs::write(game.join("data.txt"), b"new").unwrap();
    fs::write(backup.join("data.txt"), b"old").unwrap();
    fs::write(
        game.join(JOURNAL),
        format!("hearken-update-journal 2\n{}\n", backup.display()),
    )
    .unwrap();
    let install = Install::Folder {
        dir: game.clone(),
        exe: "my-game.exe".into(),
    };
    // Shared for reading and renaming, not for writing over: a loaded
    // program or library.
    let open = |p: &Path| {
        fs::OpenOptions::new()
            .read(true)
            .share_mode(0x1 | 0x4)
            .open(p)
            .unwrap()
    };
    let program = open(&game.join("my-game.exe"));
    let library = open(&game.join("SDL2.dll"));
    assert_eq!(recover_interrupted(&install).unwrap(), Recovery::Restored);
    assert_eq!(fs::read(game.join("data.txt")).unwrap(), b"old");
    assert_eq!(fs::read(game.join("SDL2.dll")).unwrap(), b"old library");
    assert!(!game.join(JOURNAL).exists());
    drop(program);
    drop(library);
    // The library renamed aside goes at the next start.
    let aside = game.join("SDL2.dll.hearken-old");
    if aside.exists() {
        assert!(game.join(LEFTOVERS).exists());
        assert_eq!(recover_interrupted(&install).unwrap(), Recovery::Nothing);
        assert!(!aside.exists());
    }
    assert!(!game.join(LEFTOVERS).exists());
}
