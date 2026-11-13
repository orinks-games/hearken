//! Whether the last run exited cleanly.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

const RUNNING: &str = "running";

/// Tells whether the game's last run ended without exiting normally, such
/// as by crashing.
///
/// [`Session::begin`] marks a file as running, and [`Session::end`] clears
/// the mark. A mark still there at the next start means the last run never
/// got to `end`: it crashed, was killed, or the computer lost power. The
/// file stays locked while the game runs, so a second copy that finds it
/// locked does not mistake the first copy for a crash.
#[derive(Debug)]
pub struct Session {
    file: Option<File>,
    ended_unexpectedly: bool,
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
        let ended_unexpectedly =
            file.read_to_string(&mut contents).is_ok() && contents.trim() == RUNNING;
        let marked = file.set_len(0).is_ok()
            && file.seek(SeekFrom::Start(0)).is_ok()
            && file.write_all(RUNNING.as_bytes()).is_ok()
            && file.sync_all().is_ok();
        Session {
            file: marked.then_some(file),
            ended_unexpectedly,
        }
    }

    /// Whether the last run ended without calling [`Session::end`].
    pub fn ended_unexpectedly(&self) -> bool {
        self.ended_unexpectedly
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

#[cfg(test)]
mod tests {
    use super::*;

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
