//! A game's sounds in one encrypted file.
//!
//! `hearken release` can ship a game's `sounds` folder as `sounds.pak`
//! instead: every sound, and the list of their names, encrypted with
//! ChaCha20 under a key made fresh for that release and built into its
//! program. A player who opens the zip sees one file they cannot play or
//! list, so a game's secrets are not spoiled by browsing its folder.
//!
//! This stops casual looking, not a determined person: the key is inside
//! the program they were given, and anyone willing to dig it out of the
//! binary can read the pack. No scheme that lets the game play its own
//! sounds on the player's computer can do better than that.
//!
//! The file:
//!
//! ```text
//! "HKPK"  version (1 byte)  3 reserved bytes  index length (u32 LE)
//! index, encrypted with nonce 0:
//!     "HKIX"  count (u32 LE)
//!     per sound: key length (u16 LE), key (UTF-8), offset (u64 LE), length (u64 LE)
//! sounds, each encrypted with nonce n + 1 for the n-th sound in the index
//! ```
//!
//! A key is a sound's path under the folder without its extension, with `/`
//! between folders: the same key the engine asks the loose folder for.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use chacha20::cipher::{KeyIvInit, StreamCipher};
use chacha20::ChaCha20;

/// The pack's file name, beside the game's program.
pub const FILE_NAME: &str = "sounds.pak";

/// Sound file extensions, in the order a key looks for them: when two files
/// share a key, the earlier extension wins, in the pack as in the folder.
pub const EXTENSIONS: [&str; 5] = ["ogg", "opus", "wav", "flac", "mp3"];

const MAGIC: &[u8; 4] = b"HKPK";
const INDEX_MAGIC: &[u8; 4] = b"HKIX";
const VERSION: u8 = 1;
const HEADER_LEN: u64 = 12;

/// The key this build reads packs with: set by `hearken release` through
/// `HEARKEN_PACK_KEY`, and `None` in every other build.
pub fn build_key() -> Option<[u8; 32]> {
    include!(concat!(env!("OUT_DIR"), "/pack_key.rs"))
}

/// A new random key, from the operating system.
pub fn new_key() -> io::Result<[u8; 32]> {
    let mut key = [0u8; 32];
    getrandom(&mut key)?;
    Ok(key)
}

/// A key as the 64 hex digits `HEARKEN_PACK_KEY` takes.
pub fn key_hex(key: &[u8; 32]) -> String {
    key.iter().map(|b| format!("{b:02x}")).collect()
}

fn getrandom(buf: &mut [u8]) -> io::Result<()> {
    getrandom::fill(buf).map_err(|e| io::Error::other(e.to_string()))
}

fn nonce(n: u64) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[4..].copy_from_slice(&n.to_le_bytes());
    nonce
}

fn crypt(key: &[u8; 32], n: u64, data: &mut [u8]) {
    let mut cipher = ChaCha20::new(key.into(), &nonce(n).into());
    cipher.apply_keystream(data);
}

/// What [`write`] did.
#[derive(Debug, Default)]
pub struct Written {
    /// How many sounds went into the pack.
    pub sounds: usize,
    /// Files under the folder that are not sounds, relative to it. The pack
    /// leaves them out; a release ships them loose.
    pub others: Vec<PathBuf>,
}

/// Packs every sound under `dir` into `out` with `key`.
pub fn write(dir: &Path, out: &Path, key: &[u8; 32]) -> io::Result<Written> {
    let mut found: HashMap<String, (usize, PathBuf)> = HashMap::new();
    let mut written = Written::default();
    let mut files = Vec::new();
    walk(dir, &mut files)?;
    for path in files {
        let rel = path.strip_prefix(dir).unwrap_or(&path);
        let ext = rel
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        let rank = ext
            .as_deref()
            .and_then(|e| EXTENSIONS.iter().position(|x| *x == e));
        let Some(rank) = rank else {
            written.others.push(rel.to_path_buf());
            continue;
        };
        let key_name = rel
            .with_extension("")
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        match found.get(&key_name) {
            Some((have, _)) if *have <= rank => {}
            _ => {
                found.insert(key_name, (rank, path.clone()));
            }
        }
    }
    let mut sounds: Vec<(String, PathBuf, u64)> = Vec::with_capacity(found.len());
    for (name, (_, path)) in found {
        let len = std::fs::metadata(&path)?.len();
        sounds.push((name, path, len));
    }
    sounds.sort_by(|a, b| a.0.cmp(&b.0));
    written.others.sort();

    let mut index = Vec::new();
    index.extend_from_slice(INDEX_MAGIC);
    index.extend_from_slice(&(sounds.len() as u32).to_le_bytes());
    let mut offset = 0u64;
    for (name, _, len) in &sounds {
        let name_len = u16::try_from(name.len())
            .map_err(|_| io::Error::other(format!("{name} is too long a name to pack")))?;
        index.extend_from_slice(&name_len.to_le_bytes());
        index.extend_from_slice(name.as_bytes());
        index.extend_from_slice(&offset.to_le_bytes());
        index.extend_from_slice(&len.to_le_bytes());
        offset += len;
    }
    crypt(key, 0, &mut index);
    let index_len = u32::try_from(index.len()).map_err(|_| io::Error::other("index too large"))?;

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = io::BufWriter::new(File::create(out)?);
    file.write_all(MAGIC)?;
    file.write_all(&[VERSION, 0, 0, 0])?;
    file.write_all(&index_len.to_le_bytes())?;
    file.write_all(&index)?;
    for (n, (_, path, len)) in sounds.iter().enumerate() {
        let mut data = std::fs::read(path)?;
        if data.len() as u64 != *len {
            return Err(io::Error::other(format!(
                "{} changed while it was packed",
                path.display()
            )));
        }
        crypt(key, n as u64 + 1, &mut data);
        file.write_all(&data)?;
    }
    file.flush()?;
    written.sounds = sounds.len();
    Ok(written)
}

/// Every file under `dir`, in no particular order.
fn walk(dir: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            walk(&path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
}

struct Entry {
    nonce: u64,
    offset: u64,
    len: u64,
}

/// An open pack: its index read, its sounds read when asked for.
pub struct Pack {
    file: File,
    key: [u8; 32],
    data_start: u64,
    entries: HashMap<String, Entry>,
}

impl Pack {
    /// Opens the pack at `path`. Fails when it is not a pack, or `key` is
    /// not the one it was written with.
    pub fn open(path: &Path, key: &[u8; 32]) -> io::Result<Pack> {
        let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_string());
        let mut file = File::open(path)?;
        let mut header = [0u8; HEADER_LEN as usize];
        file.read_exact(&mut header)
            .map_err(|_| bad("not a sound pack"))?;
        if &header[..4] != MAGIC {
            return Err(bad("not a sound pack"));
        }
        if header[4] != VERSION {
            return Err(bad("a sound pack of a version this build cannot read"));
        }
        let index_len = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
        let mut index = vec![0u8; index_len];
        file.read_exact(&mut index)
            .map_err(|_| bad("a cut-short sound pack"))?;
        crypt(key, 0, &mut index);
        if index.len() < 8 || &index[..4] != INDEX_MAGIC {
            return Err(bad("a sound pack made for another build of the game"));
        }
        let count = u32::from_le_bytes(index[4..8].try_into().unwrap()) as usize;
        let mut entries = HashMap::with_capacity(count);
        let mut at = 8usize;
        let mut take = |n: usize| -> io::Result<&[u8]> {
            let slice = index
                .get(at..at + n)
                .ok_or_else(|| bad("a damaged sound pack"))?;
            at += n;
            Ok(slice)
        };
        for n in 0..count {
            let name_len = u16::from_le_bytes(take(2)?.try_into().unwrap()) as usize;
            let name = String::from_utf8(take(name_len)?.to_vec())
                .map_err(|_| bad("a damaged sound pack"))?;
            let offset = u64::from_le_bytes(take(8)?.try_into().unwrap());
            let len = u64::from_le_bytes(take(8)?.try_into().unwrap());
            entries.insert(
                name,
                Entry {
                    nonce: n as u64 + 1,
                    offset,
                    len,
                },
            );
        }
        Ok(Pack {
            file,
            key: *key,
            data_start: HEADER_LEN + index_len as u64,
            entries,
        })
    }

    /// How many sounds the pack holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether the pack holds `key`.
    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// The sound file for `key`, or `None` when the pack has none.
    pub fn read(&mut self, key: &str) -> io::Result<Option<Vec<u8>>> {
        Ok(self.read_sealed(key)?.map(|sealed| self.unseal(&sealed)))
    }

    /// `key`'s sound as it is in the pack, still encrypted: what a caller
    /// keeps in memory so that sounds sit there decrypted only while in use.
    pub fn read_sealed(&mut self, key: &str) -> io::Result<Option<Sealed>> {
        let Some(entry) = self.entries.get(key) else {
            return Ok(None);
        };
        let mut data = vec![0u8; entry.len as usize];
        self.file
            .seek(SeekFrom::Start(self.data_start + entry.offset))?;
        self.file.read_exact(&mut data)?;
        Ok(Some(Sealed {
            nonce: entry.nonce,
            bytes: data.into(),
        }))
    }

    /// A sealed sound's file, decrypted.
    pub fn unseal(&self, sealed: &Sealed) -> Vec<u8> {
        let mut data = sealed.bytes.to_vec();
        crypt(&self.key, sealed.nonce, &mut data);
        data
    }
}

/// One sound from a pack, still encrypted. Cheap to clone.
#[derive(Clone)]
pub struct Sealed {
    nonce: u64,
    bytes: std::sync::Arc<[u8]>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let put = |rel: &str, bytes: &[u8]| {
            let path = dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        };
        put("ui/click.ogg", b"OggS click");
        put("grandmas-grudge/secret.ogg", b"OggS the secret line");
        put("grandmas-grudge/secret.wav", b"RIFF loses to the ogg");
        put("music/theme.WAV", b"RIFF theme");
        put("CREDITS.md", b"thanks");
        dir
    }

    #[test]
    fn packs_and_reads_back_every_sound() {
        let dir = tree();
        let out = dir.path().join("out").join(FILE_NAME);
        let key = new_key().unwrap();
        let written = write(dir.path(), &out, &key).unwrap();
        assert_eq!(written.sounds, 3);
        assert_eq!(written.others, [PathBuf::from("CREDITS.md")]);

        let mut pack = Pack::open(&out, &key).unwrap();
        assert_eq!(pack.len(), 3);
        assert_eq!(
            pack.read("grandmas-grudge/secret").unwrap().as_deref(),
            Some(&b"OggS the secret line"[..])
        );
        assert_eq!(
            pack.read("ui/click").unwrap().as_deref(),
            Some(&b"OggS click"[..])
        );
        assert_eq!(
            pack.read("music/theme").unwrap().as_deref(),
            Some(&b"RIFF theme"[..])
        );
        assert_eq!(pack.read("ui/missing").unwrap(), None);

        let sealed = pack.read_sealed("ui/click").unwrap().unwrap();
        assert_ne!(&*sealed.bytes, b"OggS click");
        assert_eq!(pack.unseal(&sealed), b"OggS click");
    }

    #[test]
    fn the_build_key_is_the_one_it_was_built_with() {
        // Meaningful when the tests are built with HEARKEN_PACK_KEY set.
        match std::env::var("HEARKEN_PACK_KEY")
            .ok()
            .filter(|k| !k.is_empty())
        {
            Some(hex) => assert_eq!(build_key().map(|k| key_hex(&k)), Some(hex)),
            None => assert_eq!(build_key(), None),
        }
    }

    #[test]
    fn hides_names_and_sounds_from_a_look_inside() {
        let dir = tree();
        let out = dir.path().join(FILE_NAME);
        write(dir.path(), &out, &new_key().unwrap()).unwrap();
        let raw = std::fs::read(&out).unwrap();
        let has = |needle: &[u8]| raw.windows(needle.len()).any(|w| w == needle);
        assert!(!has(b"grandmas"));
        assert!(!has(b"secret"));
        assert!(!has(b"OggS"));
    }

    #[test]
    fn another_key_cannot_open_it() {
        let dir = tree();
        let out = dir.path().join(FILE_NAME);
        write(dir.path(), &out, &new_key().unwrap()).unwrap();
        let err = Pack::open(&out, &new_key().unwrap()).err().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
