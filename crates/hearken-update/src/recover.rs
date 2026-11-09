//! Putting a folder install back after an update that did not finish, and
//! passing on what the apply script wrote while it ran.
//!
//! The game calls [`recover_interrupted`] as it starts. By then the game is
//! running from the very folder it puts back, and on Windows a running
//! program, and every library it has loaded, cannot be replaced by a
//! rename. So a file that already matches its backup is left alone, and
//! one that does not is renamed aside first (Windows allows that for a
//! file in use) and removed at a later start.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::apply::{JOURNAL, JOURNAL_HEADER, JOURNAL_HEADER_2, MAX_JOURNAL_BYTES, SAVES};
use crate::install::Install;

/// The file in a folder install where the apply script writes each step it
/// takes and anything that failed. The next start copies it into the
/// game's log and removes it.
pub const APPLY_LOG: &str = ".hearken-update.log";

/// The most of an apply log that is passed on: its end, where a failure is.
const MAX_APPLY_LOG_BYTES: u64 = 256 * 1024;

/// The file listing files a recovery renamed aside but could not remove,
/// because the running game still had them open. A later start removes
/// them.
pub const LEFTOVERS: &str = ".hearken-update-leftovers";

/// The ending of a file renamed aside so the backup could take its place.
const ASIDE: &str = ".hearken-old";

/// What [`recover_interrupted`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recovery {
    /// No update was left part-way.
    Nothing,
    /// An update was left part-way, and the version before it is back.
    Restored,
    /// An update was left part-way, and its backup is gone, so the folder
    /// is as the update left it.
    NoBackup,
}

/// Puts a folder install back as it was before an update that did not
/// finish being copied in: the apply script failed, or was stopped before
/// it removed its [`JOURNAL`]. The game calls this as it starts, before it
/// reads anything from its folder. The backup and its staging folder are
/// removed once the folder is back; doing it again after being stopped
/// part-way is safe.
///
/// First, whatever the apply script wrote to [`APPLY_LOG`] goes into this
/// program's log, so a failed update says which file and why.
///
/// The program is copied in last, by a rename, and only after everything
/// else is in. So a program that differs from its backup means the update
/// was copied in whole and only the journal was left: that update is kept.
pub fn recover_interrupted(install: &Install) -> io::Result<Recovery> {
    let Install::Folder { dir, exe } = install else {
        return Ok(Recovery::Nothing);
    };
    for line in take_apply_log(dir) {
        if line.contains("ERROR") || line.contains("failed") {
            log::warn!("update script: {line}");
        } else {
            log::info!("update script: {line}");
        }
    }
    remove_leftovers(dir);
    let journal = dir.join(JOURNAL);
    let meta = match fs::symlink_metadata(&journal) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Recovery::Nothing),
        Err(e) => return Err(e),
    };
    let read = if meta.is_file() && meta.len() <= MAX_JOURNAL_BYTES {
        fs::read_to_string(&journal)
            .ok()
            .and_then(|text| journal_backup(&text))
            .filter(|(backup, _)| backup_usable(backup, &meta))
    } else {
        None
    };
    let (backup, added) = match read {
        Some((backup, added)) => (Some(backup), added),
        None => (None, None),
    };
    let Some(backup) = backup else {
        log::warn!(
            "an update to {} did not finish and its backup is gone",
            dir.display()
        );
        remove_path(&journal)?;
        return Ok(Recovery::NoBackup);
    };
    if program_replaced(&backup, dir, exe) {
        log::info!(
            "the update to {} was copied in; only its journal was left",
            dir.display()
        );
        fs::remove_file(&journal)?;
        if let Some(staging) = backup.parent() {
            let _ = fs::remove_dir_all(staging);
        }
        return Ok(Recovery::Nothing);
    }
    log::info!(
        "an update to {} did not finish; putting {} back",
        dir.display(),
        backup.display()
    );
    let mut aside = Vec::new();
    let restored = restore_into(&backup, dir, &mut aside);
    // Renamed-aside files are listed even when the restore stops, so a
    // later start removes them.
    finish_aside(dir, aside);
    restored?;
    match added {
        // Only what the update added goes: the backup holds only what it
        // replaced, and everything else was never touched.
        Some(added) => {
            for path in added {
                if !through_links(dir, &path) {
                    let target = dir.join(&path);
                    remove_path(&target).map_err(|e| with_path(e, "removing", &target))?;
                }
            }
        }
        // An older journal's backup is the whole folder.
        None => prune_to(&backup, dir, true)?,
    }
    fs::remove_file(&journal)?;
    if let Some(staging) = backup.parent() {
        let _ = fs::remove_dir_all(staging);
    }
    Ok(Recovery::Restored)
}

/// The lines of the apply script's log, which is removed. Only its end is
/// read when it is long.
pub fn take_apply_log(dir: &Path) -> Vec<String> {
    let path = dir.join(APPLY_LOG);
    let Ok(meta) = fs::symlink_metadata(&path) else {
        return Vec::new();
    };
    if !meta.is_file() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if let Ok(mut file) = fs::File::open(&path) {
        let skip = meta.len().saturating_sub(MAX_APPLY_LOG_BYTES);
        if skip > 0 {
            let _ = io::copy(&mut (&mut file).take(skip), &mut io::sink());
        }
        let _ = file.take(MAX_APPLY_LOG_BYTES).read_to_end(&mut bytes);
    }
    let _ = fs::remove_file(&path);
    String::from_utf8_lossy(&bytes)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// Whether the program in `dir` is not the one in the backup: the apply
/// script got as far as renaming the new program into place.
fn program_replaced(backup: &Path, dir: &Path, exe: &str) -> bool {
    let kept = backup.join(exe);
    let now = dir.join(exe);
    let is_file = |p: &Path| fs::symlink_metadata(p).is_ok_and(|m| m.is_file());
    is_file(&kept) && is_file(&now) && !same_contents(&kept, &now).unwrap_or(true)
}

/// Whether two files hold the same bytes.
fn same_contents(a: &Path, b: &Path) -> io::Result<bool> {
    if fs::metadata(a)?.len() != fs::metadata(b)?.len() {
        return Ok(false);
    }
    let (mut a, mut b) = (fs::File::open(a)?, fs::File::open(b)?);
    let (mut x, mut y) = (vec![0u8; 64 * 1024], vec![0u8; 64 * 1024]);
    loop {
        let n = read_full(&mut a, &mut x)?;
        let m = read_full(&mut b, &mut y)?;
        if n != m || x[..n] != y[..m] {
            return Ok(false);
        }
        if n == 0 {
            return Ok(true);
        }
    }
}

fn read_full(file: &mut fs::File, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

/// An error that names the file it is about.
fn with_path(e: io::Error, doing: &str, path: &Path) -> io::Error {
    io::Error::new(e.kind(), format!("{doing} {}: {e}", path.display()))
}

/// Whether a folder on the way from `dir` to `rel` is a link (or not a
/// folder at all): removing through it could reach outside the game.
fn through_links(dir: &Path, rel: &Path) -> bool {
    let mut at = dir.to_path_buf();
    let mut parts = rel.components().peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            return false;
        }
        at.push(part);
        if !fs::symlink_metadata(&at).is_ok_and(|m| m.is_dir()) {
            return true;
        }
    }
    false
}

/// The backup a journal names and, for a journal that lists them, what
/// the update adds (`None` for an older journal, whose backup is the whole
/// folder). A listed path that could reach outside the folder, or into its
/// saves, makes the journal unreadable.
pub(crate) fn journal_backup(text: &str) -> Option<(PathBuf, Option<Vec<PathBuf>>)> {
    let mut lines = text.lines();
    let header = lines.next()?;
    let listed = match header {
        JOURNAL_HEADER => false,
        JOURNAL_HEADER_2 => true,
        _ => return None,
    };
    let path = PathBuf::from(lines.next()?);
    if !path.is_absolute() {
        return None;
    }
    if !listed {
        return Some((path, None));
    }
    let mut added = Vec::new();
    for line in lines {
        let rel = crate::extract::safe_path(line)?;
        let first = rel.components().next()?;
        if first.as_os_str() == SAVES || first.as_os_str() == JOURNAL {
            return None;
        }
        added.push(rel);
    }
    Some((path, Some(added)))
}

/// Whether `backup` looks like one an apply script made: a real folder
/// named `backup` in a staging folder, which on Unix only its owner, the
/// one who wrote the journal, can open.
fn backup_usable(backup: &Path, journal: &fs::Metadata) -> bool {
    let named = backup.file_name().is_some_and(|n| n == "backup");
    let Some(staging) = backup.parent() else {
        return false;
    };
    let in_staging = staging
        .file_name()
        .is_some_and(|n| n.to_string_lossy().contains("-update-"));
    let real_dir = |p: &Path| fs::symlink_metadata(p).is_ok_and(|m| m.is_dir());
    if !(named && in_staging && real_dir(backup) && real_dir(staging)) {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let Ok(meta) = fs::symlink_metadata(staging) else {
            return false;
        };
        if meta.mode() & 0o077 != 0 || meta.uid() != journal.uid() {
            return false;
        }
    }
    let _ = journal;
    true
}

pub(crate) fn remove_path(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// The name a restored file is copied to before it is renamed over the one
/// it replaces, so each file is either the old one or the restored one.
const RESTORING: &str = ".hearken-restore";

/// Copies everything in `from` over `to`, file by file. A file that
/// already matches is left alone. One that cannot be replaced because it
/// is in use is renamed aside, into `aside`, and the restored copy takes
/// its name.
fn restore_into(from: &Path, to: &Path, aside: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        let source = entry.path();
        let target = to.join(&name);
        let source_meta = fs::symlink_metadata(&source)?;
        let target_meta = fs::symlink_metadata(&target).ok();
        if source_meta.is_dir() && target_meta.as_ref().is_some_and(|m| m.is_dir()) {
            restore_into(&source, &target, aside)?;
            continue;
        }
        let both_files = source_meta.is_file() && target_meta.as_ref().is_some_and(|m| m.is_file());
        if both_files && same_contents(&source, &target).unwrap_or(false) {
            continue;
        }
        let mut temp_name = name.clone();
        temp_name.push(RESTORING);
        let temp = to.join(temp_name);
        remove_path(&temp)?;
        crate::install::copy_tree(&source, &temp).map_err(|e| with_path(e, "copying", &source))?;
        // A rename replaces a file but not a folder.
        if target_meta
            .as_ref()
            .is_some_and(|m| m.is_dir() || source_meta.is_dir())
        {
            remove_path(&target).map_err(|e| with_path(e, "removing", &target))?;
        }
        match fs::rename(&temp, &target) {
            Ok(()) => {}
            Err(e) if both_files && e.kind() == io::ErrorKind::PermissionDenied => {
                let mut old_name = name.clone();
                old_name.push(ASIDE);
                let old = to.join(old_name);
                let moved = remove_path(&old)
                    .and_then(|()| fs::rename(&target, &old))
                    .and_then(|()| {
                        aside.push(old.clone());
                        fs::rename(&temp, &target)
                    });
                if let Err(e) = moved {
                    let _ = fs::remove_file(&temp);
                    return Err(with_path(e, "replacing", &target));
                }
            }
            Err(e) => {
                let _ = fs::remove_file(&temp);
                return Err(with_path(e, "replacing", &target));
            }
        }
    }
    Ok(())
}

/// Removes the files renamed aside; any still in use are listed in
/// [`LEFTOVERS`] for a later start.
fn finish_aside(dir: &Path, aside: Vec<PathBuf>) {
    let mut left = String::new();
    for path in aside {
        if fs::remove_file(&path).is_err() {
            if let Ok(rel) = path.strip_prefix(dir) {
                let parts: Vec<_> = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                left.push_str(&parts.join("/"));
                left.push('\n');
            }
        }
    }
    if !left.is_empty() {
        let mut text = fs::read_to_string(dir.join(LEFTOVERS)).unwrap_or_default();
        text.push_str(&left);
        if let Err(e) = fs::write(dir.join(LEFTOVERS), text) {
            log::warn!("could not note files left from an update: {e}");
        }
    }
}

/// Removes the files a past recovery renamed aside but could not remove.
/// Only names ending in the aside mark, inside the folder, are touched.
fn remove_leftovers(dir: &Path) {
    let list = dir.join(LEFTOVERS);
    let Ok(meta) = fs::symlink_metadata(&list) else {
        return;
    };
    if !meta.is_file() || meta.len() > MAX_JOURNAL_BYTES {
        let _ = fs::remove_file(&list);
        return;
    }
    let text = fs::read_to_string(&list).unwrap_or_default();
    let mut still = String::new();
    for line in text.lines() {
        let Some(rel) = crate::extract::safe_path(line) else {
            continue;
        };
        if !line.ends_with(ASIDE) || through_links(dir, &rel) {
            continue;
        }
        let path = dir.join(&rel);
        if fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) && fs::remove_file(&path).is_err()
        {
            still.push_str(line);
            still.push('\n');
        }
    }
    let _ = if still.is_empty() {
        fs::remove_file(&list)
    } else {
        fs::write(&list, still)
    };
}

/// Removes what is in `to` but not in `from`: what the update added. Saves
/// and the journal stay.
fn prune_to(from: &Path, to: &Path, top: bool) -> io::Result<()> {
    for entry in fs::read_dir(to)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == SAVES || (top && (name == JOURNAL || name == LEFTOVERS)) {
            continue;
        }
        let source = from.join(&name);
        let target = entry.path();
        match fs::symlink_metadata(&source) {
            Ok(meta) if meta.is_dir() => {
                if fs::symlink_metadata(&target).is_ok_and(|m| m.is_dir()) {
                    prune_to(&source, &target, false)?;
                }
            }
            Ok(_) => {}
            Err(_) => remove_path(&target)?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(dir: &Path) -> Install {
        Install::Folder {
            dir: dir.to_path_buf(),
            exe: "my-game".into(),
        }
    }

    /// A game folder with a journal naming a backup, as an apply script
    /// that failed leaves it.
    fn interrupted(root: &Path) -> (PathBuf, PathBuf) {
        let game = root.join("game");
        fs::create_dir_all(&game).unwrap();
        let staging = crate::install::make_staging_dir_in(root, "my-game").unwrap();
        let backup = staging.join("backup");
        fs::create_dir_all(&backup).unwrap();
        fs::write(
            game.join(JOURNAL),
            format!("{JOURNAL_HEADER_2}\n{}\n", backup.display()),
        )
        .unwrap();
        (game, backup)
    }

    /// The running program already matches its backup (the script copied
    /// the backup back before starting it): it is left exactly where it
    /// is, never renamed over, which Windows refuses for a running program.
    #[cfg(unix)]
    #[test]
    fn a_program_that_matches_its_backup_is_not_replaced() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let (game, backup) = interrupted(dir.path());
        fs::write(game.join("my-game"), b"old program").unwrap();
        fs::write(backup.join("my-game"), b"old program").unwrap();
        fs::write(game.join("data.txt"), b"new").unwrap();
        fs::write(backup.join("data.txt"), b"old").unwrap();
        let inode = fs::metadata(game.join("my-game")).unwrap().ino();
        assert_eq!(
            recover_interrupted(&folder(&game)).unwrap(),
            Recovery::Restored
        );
        assert_eq!(fs::metadata(game.join("my-game")).unwrap().ino(), inode);
        assert_eq!(fs::read(game.join("data.txt")).unwrap(), b"old");
        assert!(!game.join(JOURNAL).exists());
    }

    /// The new program is in place and only the journal was left: the
    /// update finished, so it is kept rather than undone and offered again.
    #[test]
    fn a_replaced_program_keeps_the_update() {
        let dir = tempfile::tempdir().unwrap();
        let (game, backup) = interrupted(dir.path());
        fs::write(game.join("my-game"), b"new program").unwrap();
        fs::write(backup.join("my-game"), b"old program").unwrap();
        fs::write(game.join("data.txt"), b"new").unwrap();
        fs::write(backup.join("data.txt"), b"old").unwrap();
        assert_eq!(
            recover_interrupted(&folder(&game)).unwrap(),
            Recovery::Nothing
        );
        assert_eq!(fs::read(game.join("data.txt")).unwrap(), b"new");
        assert_eq!(fs::read(game.join("my-game")).unwrap(), b"new program");
        assert!(!game.join(JOURNAL).exists());
        assert!(!backup.exists());
    }

    /// What the script wrote is read once, its end only when it is long,
    /// and the file is gone after.
    #[test]
    fn the_apply_log_is_taken_once() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(APPLY_LOG),
            "started\r\n\r\n  ERROR 32 copying sounds.pak  \r\n",
        )
        .unwrap();
        assert_eq!(
            take_apply_log(dir.path()),
            ["started", "ERROR 32 copying sounds.pak"]
        );
        assert!(!dir.path().join(APPLY_LOG).exists());
        assert!(take_apply_log(dir.path()).is_empty());

        let long = "x".repeat(MAX_APPLY_LOG_BYTES as usize + 10) + "\nlast line\n";
        fs::write(dir.path().join(APPLY_LOG), long).unwrap();
        let lines = take_apply_log(dir.path());
        assert_eq!(lines.last().map(String::as_str), Some("last line"));
        assert!(lines.iter().map(|l| l.len() + 1).sum::<usize>() <= MAX_APPLY_LOG_BYTES as usize);
    }

    /// A file renamed aside and listed is removed at the next start; a
    /// listed name without the aside mark, or outside the folder, is not.
    #[test]
    fn leftovers_are_removed_later_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        fs::create_dir_all(game.join("lib")).unwrap();
        fs::write(game.join("lib/sdl.dll.hearken-old"), b"x").unwrap();
        fs::write(game.join("keep.txt"), b"x").unwrap();
        fs::write(dir.path().join("outside.hearken-old"), b"x").unwrap();
        fs::write(
            game.join(LEFTOVERS),
            "lib/sdl.dll.hearken-old\nkeep.txt\n../outside.hearken-old\n",
        )
        .unwrap();
        assert_eq!(
            recover_interrupted(&folder(&game)).unwrap(),
            Recovery::Nothing
        );
        assert!(!game.join("lib/sdl.dll.hearken-old").exists());
        assert!(game.join("keep.txt").exists());
        assert!(dir.path().join("outside.hearken-old").exists());
        assert!(!game.join(LEFTOVERS).exists());
    }

    /// Files removed by a recovery that could not remove them are listed.
    #[test]
    fn aside_files_that_stay_are_listed() {
        let dir = tempfile::tempdir().unwrap();
        // A folder cannot be removed as a file, standing in for one in use.
        fs::create_dir_all(dir.path().join("lib/a.dll.hearken-old")).unwrap();
        finish_aside(dir.path(), vec![dir.path().join("lib/a.dll.hearken-old")]);
        assert_eq!(
            fs::read_to_string(dir.path().join(LEFTOVERS)).unwrap(),
            "lib/a.dll.hearken-old\n"
        );
    }
}
