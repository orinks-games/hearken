//! A game's data folder, read and written through the seal.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::format;
use crate::key::Key;
use crate::name;

/// What reading a data file found.
#[derive(Debug)]
pub enum Opened {
    /// No file.
    Missing,
    /// An authentic sealed file: its bytes.
    Sealed(Vec<u8>),
    /// A plain file, in a build that does not seal (a developer build).
    Plain(Vec<u8>),
    /// A plain file in a sealing build that no witness says this game has
    /// sealed for this player before: written by an older version of the
    /// game, or by hand. The game decides whether to import it.
    Legacy(Vec<u8>),
    /// A sealed file in a build that cannot open it (a developer build
    /// running against a release's saves). The vault will not overwrite it.
    Locked,
    /// The file was changed, swapped or forged.
    Tampered(Tamper),
    /// The disk would not give the file up (permissions, I/O, a folder
    /// where the file should be).
    Failed(io::Error),
}

impl Opened {
    /// The bytes of `Sealed` or `Plain`: the only verdicts a game can trust
    /// as they are.
    pub fn trusted(self) -> Option<Vec<u8>> {
        match self {
            Opened::Sealed(bytes) | Opened::Plain(bytes) => Some(bytes),
            _ => None,
        }
    }
}

/// How a sealed file was tampered with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tamper {
    /// Does not authenticate: edited, cut short, another file's, another
    /// game's or another key's.
    Forged,
    /// A plain file where a witness says this game has sealed before.
    Unsealed,
}

impl std::fmt::Display for Tamper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Tamper::Forged => "it does not match its seal",
            Tamper::Unsealed => "a plain file replaced a sealed one",
        })
    }
}

/// A game's data folder, read and written through the seal.
///
/// Every file goes in and out whole. A write lands atomically (a hidden
/// temporary file, flushed, then renamed over the old one), so a crash
/// never leaves half a file. Names are single file names (see
/// [`Vault::read`]); the vault keeps nothing in the folder but the game's
/// own files, and never touches anything outside it except its witnesses
/// (see [`Vault::with_witnesses`]).
pub struct Vault {
    dir: PathBuf,
    key: Option<Key>,
    witnesses: Vec<PathBuf>,
    witnessed: bool,
}

impl Vault {
    /// The folder at `dir`, sealing with this build's key, if it has one.
    pub fn new(dir: impl Into<PathBuf>) -> Vault {
        Vault::with_key(dir, crate::build_key())
    }

    /// The folder at `dir` with a given key, or none: for tests and tools.
    pub fn with_key(dir: impl Into<PathBuf>, key: Option<Key>) -> Vault {
        Vault {
            dir: dir.into(),
            key,
            witnesses: Vec::new(),
            witnessed: false,
        }
    }

    /// The same vault, leaving a small sealed marker at each of `paths` (full
    /// file paths, outside the data folder) once it seals anything.
    ///
    /// A marker says "this game has sealed data for this player before".
    /// While one is there, a plain file reads as `Tampered(Unsealed)`, not
    /// as `Legacy`: so swapping a sealed file for a hand-written plain one,
    /// or pointing the game at a fresh folder holding one, no longer gets it
    /// imported. Writing a marker is best effort; a failure is logged. Only
    /// whether a marker exists counts, never what it holds.
    pub fn with_witnesses(mut self, paths: Vec<PathBuf>) -> Vault {
        self.witnesses = paths;
        self
    }

    /// The marker files this vault may leave outside its folder.
    pub fn witnesses(&self) -> &[PathBuf] {
        &self.witnesses
    }

    /// Whether files are sealed (a release built with `seal = true`).
    pub fn is_sealing(&self) -> bool {
        self.key.is_some()
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Reads `name`; never trusts a file that fails the seal.
    ///
    /// A name is one file name: ASCII letters, digits, `.`, `_` and `-`, not
    /// starting with `.`, at most 64 bytes. Names are compared exactly, so
    /// a game should not use two that differ only in case. Any other name
    /// reads as `Failed` with `InvalidInput`.
    ///
    /// A plain file reads as `Legacy` only while no witness says this game
    /// has sealed for this player. After that it is `Tampered(Unsealed)`.
    pub fn read(&mut self, name: &str) -> Opened {
        if let Err(e) = name::check(name) {
            return Opened::Failed(e);
        }
        let found = match read_file(&self.dir.join(name)) {
            Ok(found) => found,
            Err(e) => return Opened::Failed(e),
        };
        let Some(bytes) = found else {
            return Opened::Missing;
        };
        let Some(key) = &self.key else {
            return if format::is_sealed(&bytes) {
                Opened::Locked
            } else {
                Opened::Plain(bytes)
            };
        };
        if !format::is_sealed(&bytes) {
            return if self.witnessed_before() {
                Opened::Tampered(Tamper::Unsealed)
            } else {
                Opened::Legacy(bytes)
            };
        }
        match format::open(&key.file_key(), name, &bytes) {
            Some(plain) => Opened::Sealed(plain),
            None => Opened::Tampered(Tamper::Forged),
        }
    }

    /// Writes `name` atomically: sealed when sealing, plain otherwise. A
    /// build that does not seal refuses (`PermissionDenied`) to overwrite a
    /// sealed file. A sealing build overwriting a plain file first keeps it
    /// as `<name>.plain-backup`, unless one is there already.
    pub fn write(&mut self, name: &str, bytes: &[u8]) -> io::Result<()> {
        name::check(name)?;
        std::fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(name);
        let existing = read_file(&path)?;
        let Some(key) = self.key.clone() else {
            if existing.as_deref().is_some_and(format::is_sealed) {
                return Err(locked(name));
            }
            return atomic_write(&self.dir, name, bytes);
        };
        if let Some(old) = existing.filter(|old| !format::is_sealed(old)) {
            let backup = self.dir.join(format!("{name}{}", name::BACKUP));
            if std::fs::symlink_metadata(&backup).is_err() {
                std::fs::write(&backup, old)?;
            }
        }
        let sealed = format::seal(&key.file_key(), name, bytes)?;
        atomic_write(&self.dir, name, &sealed)?;
        self.leave_witnesses(&key);
        Ok(())
    }

    /// Deletes `name` on purpose (a reset the player chose). A build that
    /// does not seal will not delete a sealed file.
    pub fn remove(&mut self, name: &str) -> io::Result<()> {
        name::check(name)?;
        let path = self.dir.join(name);
        if !self.is_sealing() && read_file(&path)?.as_deref().is_some_and(format::is_sealed) {
            return Err(locked(name));
        }
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// Moves a file the game will not use to `<name>.tampered` (or
    /// `.tampered-2`, `-3`...), so it can still be looked at, and the next
    /// read is `Missing`. Returns where it went (`None` when there was no
    /// file). A build that does not seal will not move a sealed file.
    pub fn set_aside(&mut self, name: &str) -> io::Result<Option<PathBuf>> {
        name::check(name)?;
        let path = self.dir.join(name);
        if std::fs::symlink_metadata(&path).is_err() {
            return Ok(None);
        }
        if !self.is_sealing() && read_file(&path)?.as_deref().is_some_and(format::is_sealed) {
            return Err(locked(name));
        }
        let to = free_name(&self.dir, name);
        std::fs::rename(&path, &to)?;
        log::warn!("seal: set {name} aside as {}", to.display());
        Ok(Some(to))
    }

    /// Writes any witness that is not there yet, once per run.
    fn leave_witnesses(&mut self, key: &Key) {
        if self.witnessed {
            return;
        }
        self.witnessed = true;
        for path in &self.witnesses {
            if std::fs::symlink_metadata(path).is_ok() {
                continue;
            }
            let written = (|| {
                let sealed = format::seal(&key.file_key(), WITNESS, b"")?;
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, sealed)
            })();
            if let Err(e) = written {
                log::warn!("seal: could not leave a witness at {}: {e}", path.display());
            }
        }
    }

    /// Whether any witness says this game has sealed for this player.
    fn witnessed_before(&self) -> bool {
        self.witnesses
            .iter()
            .any(|path| std::fs::symlink_metadata(path).is_ok())
    }
}

/// The name a witness is sealed under.
const WITNESS: &str = "hearken-seal.witness";

fn locked(name: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("{name} is sealed by a release of the game; this build will not change it"),
    )
}

/// The file's bytes, or `None` when there is no file. A folder where the
/// file should be is an error, not a missing file.
fn read_file(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            // A dangling link is a file that is gone.
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// Writes `bytes` to `dir/name` through a hidden temporary file, flushed
/// before it is renamed over the old one.
fn atomic_write(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    let tmp = dir.join(name::temp(name));
    let written = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, dir.join(name))
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// `dir/<name>.tampered`, or the first of `.tampered-2`, `-3`... not taken.
fn free_name(dir: &Path, name: &str) -> PathBuf {
    let mut n = 1u32;
    loop {
        let candidate = if n == 1 {
            dir.join(format!("{name}{}", name::TAMPERED))
        } else {
            dir.join(format!("{name}{}-{n}", name::TAMPERED))
        };
        if std::fs::symlink_metadata(&candidate).is_err() {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests;
