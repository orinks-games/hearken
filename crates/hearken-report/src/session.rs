//! Whether the last run exited cleanly.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

const RUNNING: &str = "running";

/// Tells whether the game's last run ended without exiting normally, such
/// as by crashing, and where it was.
///
/// [`Session::begin`] marks a file as running, and [`Session::end`] clears
/// the mark. A mark still there at the next start means the last run never
/// got to `end`: it crashed, was killed, or the computer lost power. The
/// file stays locked while the game runs, so a second copy that finds it
/// locked does not mistake the first copy for a crash. [`Session::note`]
/// keeps a line of text under the mark, such as which game was open, and
/// the next start reads it back as [`Session::last_note`].
#[derive(Debug)]
pub struct Session {
    file: Option<File>,
    ended_unexpectedly: bool,
    last_note: Option<String>,
}

impl Session {
    /// Marks this run as running in the file at `path`, and finds out how
    /// the last run ended.
    ///
    /// Call it as early as the data folder is known, so a crash during
    /// startup counts. If the file cannot be written or locked, the session
    /// never reports a crash.
    pub fn begin(path: &Path) -> Session {
        let unknown = Session {
            file: None,
            ended_unexpectedly: false,
            last_note: None,
        };
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let mut file = match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
        {
            Ok(file) => file,
            Err(e) => {
                log::warn!("could not open {}: {e}", path.display());
                return unknown;
            }
        };
        match file.try_lock() {
            Ok(()) => {}
            // Another copy of the game holds the lock, so the mark is its
            // own and not a crash.
            Err(TryLockError::WouldBlock) => return unknown,
            Err(TryLockError::Error(e)) => {
                log::warn!("could not lock {}: {e}", path.display());
                return unknown;
            }
        }
        let mut contents = String::new();
        let _ = file.read_to_string(&mut contents);
        let (mark, note) = contents.split_once('\n').unwrap_or((&contents, ""));
        let ended_unexpectedly = mark.trim() == RUNNING;
        let last_note =
            (ended_unexpectedly && !note.trim().is_empty()).then(|| note.trim().to_string());
        let marked = write_mark(&mut file, "");
        Session {
            file: marked.then_some(file),
            ended_unexpectedly,
            last_note,
        }
    }

    /// Whether the last run ended without calling [`Session::end`].
    pub fn ended_unexpectedly(&self) -> bool {
        self.ended_unexpectedly
    }

    /// The last run's note, when it ended unexpectedly and had left one.
    pub fn last_note(&self) -> Option<&str> {
        self.last_note.as_deref()
    }

    /// Keeps `text` under the running mark for the next start to read if
    /// this run never ends: where the player is, in a few lines. Replaces
    /// the last note; call it when that changes, not every frame.
    pub fn note(&mut self, text: &str) {
        if let Some(file) = self.file.as_mut() {
            if !write_mark(file, text.trim()) {
                log::warn!("could not note where the game is in the session marker");
            }
        }
    }

    /// Records that this run exited normally. Call it as the game exits; a
    /// run that never calls it counts as having ended unexpectedly.
    pub fn end(self) {
        // Emptied rather than deleted: Windows cannot delete an open file.
        if let Some(file) = self.file {
            let _ = file.set_len(0);
            let _ = file.sync_all();
        }
    }
}

/// Writes the running mark, then `note` on the lines after it.
fn write_mark(file: &mut File, note: &str) -> bool {
    let mut text = RUNNING.to_string();
    if !note.is_empty() {
        text.push('\n');
        text.push_str(note);
    }
    file.set_len(0).is_ok()
        && file.seek(SeekFrom::Start(0)).is_ok()
        && file.write_all(text.as_bytes()).is_ok()
        && file.sync_all().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_comes_back_after_a_crash_and_not_after_a_clean_exit() {
        let (_dir, path) = marker();
        let mut first = Session::begin(&path);
        assert_eq!(first.last_note(), None);
        first.note("Game\tTightrope");
        first.note("Game\tBelfry\nMode\tchallenge");
        drop(first);
        let next = Session::begin(&path);
        assert!(next.ended_unexpectedly());
        assert_eq!(next.last_note(), Some("Game\tBelfry\nMode\tchallenge"));
        // The note is gone with the mark once this run is noticed.
        drop(next);
        let again = Session::begin(&path);
        assert!(again.ended_unexpectedly());
        assert_eq!(again.last_note(), None);
        again.end();
        let clean = Session::begin(&path);
        assert!(!clean.ended_unexpectedly());
        assert_eq!(clean.last_note(), None);
    }

    fn marker() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.running");
        (dir, path)
    }

    #[test]
    fn a_first_run_and_a_run_after_a_normal_exit_are_not_crashes() {
        let (_dir, path) = marker();
        let first = Session::begin(&path);
        assert!(!first.ended_unexpectedly());
        first.end();
        assert!(!Session::begin(&path).ended_unexpectedly());
    }

    #[test]
    fn a_run_that_never_ended_is_reported_once() {
        let (_dir, path) = marker();
        drop(Session::begin(&path));
        let next = Session::begin(&path);
        assert!(next.ended_unexpectedly());
        next.end();
        assert!(!Session::begin(&path).ended_unexpectedly());
    }

    #[test]
    fn a_second_copy_of_the_game_is_not_mistaken_for_a_crash() {
        let (_dir, path) = marker();
        let first = Session::begin(&path);
        let second = Session::begin(&path);
        assert!(!second.ended_unexpectedly());
        second.end();
        first.end();
        assert!(!Session::begin(&path).ended_unexpectedly());
    }

    #[test]
    fn a_folder_that_cannot_be_written_never_reports_a_crash() {
        let (dir, _) = marker();
        let not_a_dir = dir.path().join("file");
        fs::write(&not_a_dir, "x").unwrap();
        let session = Session::begin(&not_a_dir.join("session.running"));
        assert!(!session.ended_unexpectedly());
        session.end();
    }
}
