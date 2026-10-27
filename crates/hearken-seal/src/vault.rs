//! A game's data folder, read and written through the seal.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::format;
use crate::key::Key;
use crate::ledger::{self, Entries};
use crate::name;
use crate::LEDGER;

/// What reading a data file found.
#[derive(Debug)]
pub enum Opened {
    /// No file, and none was ever sealed here.
    Missing,
    /// An authentic sealed file: its bytes.
    Sealed(Vec<u8>),
    /// A plain file, in a build that does not seal (a developer build).
    Plain(Vec<u8>),
    /// A plain file in a sealing build, with no sealed history: written by
    /// an older version of the game, or by hand. The game decides whether
    /// to import it.
    Legacy(Vec<u8>),
    /// A sealed file in a build that cannot open it (a developer build
    /// running against a release's saves). The vault will not overwrite it.
    Locked,
    /// The file was changed, swapped, restored, removed or forged.
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
    /// A plain file where a sealed one was written.
    Unsealed,
    /// An older sealed copy than the one last written.
    RolledBack,
    /// A sealed copy this folder's history does not account for.
    Unexpected,
    /// Deleted after it was sealed.
    Removed,
    /// The folder's ledger itself does not authenticate.
    LedgerForged,
}

impl std::fmt::Display for Tamper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Tamper::Forged => "it does not match its seal",
            Tamper::Unsealed => "a plain file replaced a sealed one",
            Tamper::RolledBack => "an older copy was put back",
            Tamper::Unexpected => "a copy from elsewhere was put in",
            Tamper::Removed => "it was deleted",
            Tamper::LedgerForged => "the folder's ledger does not match its seal",
        })
    }
}

/// The ledger as the vault last saw it.
enum Ledger {
    /// Not read yet.
    Unread,
    /// Read and authentic (or absent: empty). `generation` is the ledger's
    /// own write count.
    Good { generation: u64, entries: Entries },
    /// There, but it does not authenticate.
    Forged,
}

/// A game's data folder, read and written through the seal.
///
/// Every file goes in and out whole. A write lands atomically (a hidden
/// temporary file, flushed, then renamed over the old one), so a crash
/// never leaves half a file. Names are single file names (see
/// [`Vault::read`]); the vault never touches anything outside its folder.
pub struct Vault {
    dir: PathBuf,
    key: Option<Key>,
    ledger: Ledger,
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
            ledger: Ledger::Unread,
        }
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
    pub fn read(&mut self, name: &str) -> Opened {
        if let Err(e) = name::check(name) {
            return Opened::Failed(e);
        }
        let found = match read_file(&self.dir.join(name)) {
            Ok(found) => found,
            Err(e) => return Opened::Failed(e),
        };
        let Some(key) = self.key.clone() else {
            return match found {
                None => Opened::Missing,
                Some(bytes) if format::is_sealed(&bytes) => Opened::Locked,
                Some(bytes) => Opened::Plain(bytes),
            };
        };
        let last = match self.entries() {
            Ok(Some(entries)) => entries.get(name).copied().unwrap_or(0),
            Ok(None) => return Opened::Tampered(Tamper::LedgerForged),
            Err(e) => return Opened::Failed(e),
        };
        let bytes = match found {
            None if last == 0 => return Opened::Missing,
            None => return Opened::Tampered(Tamper::Removed),
            Some(bytes) => bytes,
        };
        if !format::is_sealed(&bytes) {
            return if last == 0 {
                Opened::Legacy(bytes)
            } else {
                Opened::Tampered(Tamper::Unsealed)
            };
        }
        let Some((generation, plain)) = format::open(&key.file_key(), name, &bytes) else {
            return Opened::Tampered(Tamper::Forged);
        };
        if generation == last {
            Opened::Sealed(plain)
        } else if Some(generation) == last.checked_add(1) {
            // The game wrote the file, then stopped before the ledger: catch
            // the ledger up.
            if let Err(e) = self.record(name, Some(generation)) {
                log::warn!("seal: could not bring the ledger up to date for {name}: {e}");
            }
            Opened::Sealed(plain)
        } else if generation < last {
            Opened::Tampered(Tamper::RolledBack)
        } else {
            Opened::Tampered(Tamper::Unexpected)
        }
    }

    /// Writes `name` atomically: sealed when sealing, plain otherwise. A
    /// build that does not seal refuses (`PermissionDenied`) to overwrite a
    /// sealed file. A sealing build overwriting a plain file first keeps it
    /// as `<name>.plain-backup`. A sealing build whose ledger was forged
    /// refuses (`InvalidData`) until the game has set the file aside.
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
        let last = match self.entries()? {
            Some(entries) => entries.get(name).copied().unwrap_or(0),
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "the data folder's ledger was tampered with; set the file aside first",
                ))
            }
        };
        if let Some(old) = existing.filter(|old| !format::is_sealed(old)) {
            let backup = self.dir.join(format!("{name}{}", name::BACKUP));
            if last == 0 && !backup.exists() {
                std::fs::write(&backup, old)?;
            }
        }
        let generation = last
            .checked_add(1)
            .ok_or_else(|| io::Error::other("the file has been written too many times"))?;
        let sealed = format::seal(&key.file_key(), name, generation, bytes)?;
        atomic_write(&self.dir, name, &sealed)?;
        self.record(name, Some(generation))
    }

    /// Deletes `name` on purpose (a reset the player chose) and forgets its
    /// history. A build that does not seal will not delete a sealed file.
    pub fn remove(&mut self, name: &str) -> io::Result<()> {
        name::check(name)?;
        let path = self.dir.join(name);
        if !self.is_sealing() && read_file(&path)?.as_deref().is_some_and(format::is_sealed) {
            return Err(locked(name));
        }
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        self.forget(name)
    }

    /// Moves a file the game will not use to `<name>.tampered` (or
    /// `.tampered-2`, `-3`...), so it can still be looked at, and forgets its
    /// history so the next write starts clean. Returns where it went (`None`
    /// when there was no file). When the ledger itself was forged, it is set
    /// aside too and the folder starts a new one. A build that does not seal
    /// will not move a sealed file.
    pub fn set_aside(&mut self, name: &str) -> io::Result<Option<PathBuf>> {
        name::check(name)?;
        let path = self.dir.join(name);
        if std::fs::symlink_metadata(&path).is_err() {
            self.forget(name)?;
            return Ok(None);
        }
        if !self.is_sealing() && read_file(&path)?.as_deref().is_some_and(format::is_sealed) {
            return Err(locked(name));
        }
        let to = free_name(&self.dir, name);
        std::fs::rename(&path, &to)?;
        log::warn!("seal: set {name} aside as {}", to.display());
        self.forget(name)?;
        Ok(Some(to))
    }

    /// Drops `name` from the ledger. A forged ledger is set aside and
    /// started again.
    fn forget(&mut self, name: &str) -> io::Result<()> {
        if !self.is_sealing() {
            return Ok(());
        }
        if self.entries()?.is_none() {
            let ledger = self.dir.join(LEDGER);
            if std::fs::symlink_metadata(&ledger).is_ok() {
                let to = free_name(&self.dir, LEDGER);
                std::fs::rename(&ledger, &to)?;
                log::warn!("seal: set the forged ledger aside as {}", to.display());
            }
            self.ledger = Ledger::Good {
                generation: 0,
                entries: Entries::new(),
            };
        }
        self.record(name, None)
    }

    /// The ledger's entries, read on first use: `None` when it is forged.
    fn entries(&mut self) -> io::Result<Option<&Entries>> {
        if let Ledger::Unread = self.ledger {
            self.ledger = self.load_ledger()?;
        }
        Ok(match &self.ledger {
            Ledger::Good { entries, .. } => Some(entries),
            _ => None,
        })
    }

    fn load_ledger(&self) -> io::Result<Ledger> {
        let key = self
            .key
            .as_ref()
            .expect("only a sealing vault keeps a ledger");
        let path = self.dir.join(LEDGER);
        let bytes = match read_file(&path) {
            Ok(None) => {
                return Ok(Ledger::Good {
                    generation: 0,
                    entries: Entries::new(),
                })
            }
            Ok(Some(bytes)) => bytes,
            // A folder or the like where the ledger should be.
            Err(_) if path.exists() && !path.is_file() => return Ok(Ledger::Forged),
            Err(e) => return Err(e),
        };
        Ok(format::open(&key.file_key(), LEDGER, &bytes)
            .and_then(|(generation, text)| {
                ledger::from_text(&text).map(|entries| Ledger::Good {
                    generation,
                    entries,
                })
            })
            .unwrap_or(Ledger::Forged))
    }

    /// Sets `name`'s generation in the ledger (`None` removes it) and saves
    /// the ledger. Needs a good ledger.
    fn record(&mut self, name: &str, generation: Option<u64>) -> io::Result<()> {
        let key = self
            .key
            .clone()
            .expect("only a sealing vault keeps a ledger");
        self.entries()?;
        let Ledger::Good {
            generation: count,
            entries,
        } = &mut self.ledger
        else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the data folder's ledger was tampered with",
            ));
        };
        match generation {
            Some(g) => entries.insert(name.to_string(), g),
            None => entries.remove(name),
        };
        *count = count.saturating_add(1);
        let text = ledger::to_text(entries);
        let sealed = format::seal(&key.file_key(), LEDGER, *count, text.as_bytes())?;
        std::fs::create_dir_all(&self.dir)?;
        atomic_write(&self.dir, LEDGER, &sealed)
    }
}

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
