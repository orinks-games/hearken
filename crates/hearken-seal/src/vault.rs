//! A game's data folder, read and written through the seal.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::format::{self, FolderId, NO_FOLDER};
use crate::key::Key;
use crate::ledger::{self, Entries, Entry};
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
    /// Read and authentic, or absent. `generation` is the ledger's own write
    /// count: 0 while it has never been written, and then `folder` is a
    /// new id that no file has been sealed with yet.
    Good {
        generation: u64,
        folder: FolderId,
        entries: Entries,
    },
    /// There, but it does not authenticate.
    Forged,
}

/// What the ledger says about one name.
struct History {
    folder: FolderId,
    entry: Option<Entry>,
    /// The ledger has been written, so the folder has sealed before.
    kept: bool,
}

/// A game's data folder, read and written through the seal.
///
/// Every file goes in and out whole. A write lands atomically (a hidden
/// temporary file, flushed, then renamed over the old one), so a crash
/// never leaves half a file. Names are single file names (see
/// [`Vault::read`]); the vault never touches anything outside its folder,
/// except for its witnesses (see [`Vault::with_witnesses`]).
pub struct Vault {
    dir: PathBuf,
    key: Option<Key>,
    ledger: Ledger,
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
            ledger: Ledger::Unread,
            witnesses: Vec::new(),
            witnessed: false,
        }
    }

    /// The same vault, leaving a small sealed marker at each of `paths` (full
    /// file paths, outside the data folder) once it seals anything.
    ///
    /// A marker says "this game has sealed data for this player before".
    /// While one is there, a plain file in a folder with no ledger reads as
    /// `Tampered(Unsealed)`, not as `Legacy`: so deleting the folder's
    /// ledger and files, or pointing the game at a fresh folder, and then
    /// dropping in a hand-written plain file no longer gets it imported.
    /// Writing a marker is best effort; a failure is logged. Only whether a
    /// marker exists counts, never what it holds.
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
    /// A plain file reads as `Legacy` only in a folder that has never
    /// sealed anything (no ledger) and when no witness says this game has
    /// sealed elsewhere for this player. Everywhere else it is
    /// `Tampered(Unsealed)`.
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
        let History {
            folder,
            entry,
            kept,
        } = match self.history(name) {
            Ok(Some(history)) => history,
            Ok(None) => return Opened::Tampered(Tamper::LedgerForged),
            Err(e) => return Opened::Failed(e),
        };
        let last = entry.map_or(0, |e| e.generation);
        let expected = entry.is_some_and(|e| !e.gone);
        let bytes = match found {
            None if expected => return Opened::Tampered(Tamper::Removed),
            None => return Opened::Missing,
            Some(bytes) => bytes,
        };
        if !format::is_sealed(&bytes) {
            return if expected || kept || self.witnessed_before() {
                Opened::Tampered(Tamper::Unsealed)
            } else {
                Opened::Legacy(bytes)
            };
        }
        if !kept {
            // The ledger is gone: no sealed file can be accounted for.
            return Opened::Tampered(Tamper::Unexpected);
        }
        let Some((generation, plain)) = format::open(&key.file_key(), &folder, name, &bytes) else {
            return Opened::Tampered(Tamper::Forged);
        };
        if generation == last && expected {
            Opened::Sealed(plain)
        } else if Some(generation) == last.checked_add(1) {
            // The game wrote the file, then stopped before the ledger: catch
            // the ledger up. Only a file sealed for this folder gets here.
            // A ledger copy kept at the old generation therefore takes any
            // file written next; that is a partial form of restoring the
            // whole folder, which is accepted (see the crate docs).
            let entry = Entry {
                generation,
                gone: false,
            };
            if let Err(e) = self.record(name, Some(entry)) {
                log::warn!("seal: could not bring the ledger up to date for {name}: {e}");
            }
            Opened::Sealed(plain)
        } else if generation <= last {
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
        let Some(History {
            folder,
            entry,
            kept,
        }) = self.history(name)?
        else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the data folder's ledger was tampered with; set the file aside first",
            ));
        };
        if !kept {
            // Keep the folder's id before sealing anything with it, so a
            // crash after this write still leaves a file the ledger can
            // account for.
            self.save_ledger()?;
        }
        let last = entry.map_or(0, |e| e.generation);
        if let Some(old) = existing.filter(|old| !format::is_sealed(old)) {
            let backup = self.dir.join(format!("{name}{}", name::BACKUP));
            if last == 0 && !backup.exists() {
                std::fs::write(&backup, old)?;
            }
        }
        let generation = last
            .checked_add(1)
            .ok_or_else(|| io::Error::other("the file has been written too many times"))?;
        let sealed = format::seal(&key.file_key(), &folder, name, generation, bytes)?;
        atomic_write(&self.dir, name, &sealed)?;
        self.record(
            name,
            Some(Entry {
                generation,
                gone: false,
            }),
        )
    }

    /// Deletes `name` on purpose (a reset the player chose). The ledger
    /// remembers that it is gone, so an old copy put back later reads as
    /// rolled back. A build that does not seal will not delete a sealed
    /// file.
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
    /// `.tampered-2`, `-3`...), so it can still be looked at, and marks it
    /// gone in the ledger, so the next read is `Missing` and the next write
    /// starts a new generation. A plain file put in its place later still
    /// reads as `Tampered(Unsealed)`, and an old sealed copy as
    /// `RolledBack`. Returns where it went (`None` when there was no file).
    /// When the ledger itself was forged, it is set aside too and the folder
    /// starts a new one, under a new folder id. A build that does not seal
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

    /// Marks `name` gone in the ledger, keeping its generation. A forged
    /// ledger is set aside and started again.
    fn forget(&mut self, name: &str) -> io::Result<()> {
        if !self.is_sealing() {
            return Ok(());
        }
        if self.history(name)?.is_none() {
            let ledger = self.dir.join(LEDGER);
            if std::fs::symlink_metadata(&ledger).is_ok() {
                let to = free_name(&self.dir, LEDGER);
                std::fs::rename(&ledger, &to)?;
                log::warn!("seal: set the forged ledger aside as {}", to.display());
            }
            self.ledger = fresh_ledger()?;
        }
        let tombstone = self
            .history(name)?
            .and_then(|h| h.entry)
            .map(|e| Entry { gone: true, ..e });
        self.record(name, tombstone)
    }

    /// What the ledger says about `name`, read on first use: `None` when the
    /// ledger is forged.
    fn history(&mut self, name: &str) -> io::Result<Option<History>> {
        if let Ledger::Unread = self.ledger {
            self.ledger = self.load_ledger()?;
        }
        Ok(match &self.ledger {
            Ledger::Good {
                generation,
                folder,
                entries,
            } => Some(History {
                folder: *folder,
                entry: entries.get(name).copied(),
                kept: *generation > 0,
            }),
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
            Ok(None) => return fresh_ledger(),
            Ok(Some(bytes)) => bytes,
            // A folder or the like where the ledger should be.
            Err(_) if path.exists() && !path.is_file() => return Ok(Ledger::Forged),
            Err(e) => return Err(e),
        };
        Ok(format::open(&key.file_key(), &NO_FOLDER, LEDGER, &bytes)
            .and_then(|(generation, text)| {
                ledger::from_text(&text).map(|(folder, entries)| Ledger::Good {
                    generation,
                    folder,
                    entries,
                })
            })
            .unwrap_or(Ledger::Forged))
    }

    /// Sets `name`'s entry in the ledger (`None` removes it) and saves the
    /// ledger. Needs a good ledger.
    fn record(&mut self, name: &str, entry: Option<Entry>) -> io::Result<()> {
        self.history(name)?;
        let Ledger::Good { entries, .. } = &mut self.ledger else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the data folder's ledger was tampered with",
            ));
        };
        match entry {
            Some(entry) => entries.insert(name.to_string(), entry),
            None => entries.remove(name),
        };
        self.save_ledger()
    }

    /// Writes the ledger as it is in memory, one generation on, then leaves
    /// the witnesses. Needs a good ledger.
    fn save_ledger(&mut self) -> io::Result<()> {
        let key = self
            .key
            .clone()
            .expect("only a sealing vault keeps a ledger");
        let Ledger::Good {
            generation,
            folder,
            entries,
        } = &mut self.ledger
        else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the data folder's ledger was tampered with",
            ));
        };
        *generation = generation.saturating_add(1);
        let text = ledger::to_text(folder, entries);
        let sealed = format::seal(
            &key.file_key(),
            &NO_FOLDER,
            LEDGER,
            *generation,
            text.as_bytes(),
        )?;
        std::fs::create_dir_all(&self.dir)?;
        atomic_write(&self.dir, LEDGER, &sealed)?;
        self.leave_witnesses(&key);
        Ok(())
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
                let sealed = format::seal(&key.file_key(), &NO_FOLDER, WITNESS, 0, b"")?;
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

/// An empty ledger that has never been written, under a new folder id.
fn fresh_ledger() -> io::Result<Ledger> {
    let mut folder = NO_FOLDER;
    getrandom::fill(&mut folder).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(Ledger::Good {
        generation: 0,
        folder,
        entries: Entries::new(),
    })
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
