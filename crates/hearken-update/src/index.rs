//! The file index of a zip release, so a copy can download only the files
//! that changed.
//!
//! `hearken release` writes it beside the archive, as
//! `<archive name without .zip>.index.json`, and the signed manifest names
//! it with its size and SHA-256 (see [`crate::manifest::IndexRef`]), so it
//! is as trusted as the manifest. For each entry of the zip, in the order of
//! its central directory, it says:
//!
//! - its name, exactly as in the zip;
//! - its unpacked size and the SHA-256 of its unpacked bytes (a link's
//!   target, nothing for a folder);
//! - where it sits in the archive: its local header's offset, where its
//!   compressed bytes start, and where they end.
//!
//! It also gives the SHA-256 of the central directory (everything from its
//! start to the end of the archive), so the copy can read it from a range
//! request and trust it. A tar.gz has no index: it cannot be read in parts.

use std::fs;
use std::io::{self, Read};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::key::{from_hex, to_hex};
use crate::manifest::MAX_ARCHIVE_BYTES;

/// The index format this crate writes and reads.
pub const INDEX_FORMAT: u32 = 1;

/// The largest index read: room for the most entries an archive may have.
pub const MAX_INDEX_BYTES: u64 = 64 * 1024 * 1024;

/// The most entries an index may list, the same as an unpack allows.
pub const MAX_INDEX_ENTRIES: usize = 200_000;

/// A zip release's file index. Every field is covered by the manifest's
/// signature, through the index's signed SHA-256.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    pub format: u32,
    /// The archive this index describes: its size and SHA-256, as signed in
    /// the manifest.
    pub archive_size: u64,
    pub archive_sha256: String,
    /// The central directory: from where it starts to the end of the
    /// archive, and those bytes' SHA-256.
    pub central_start: u64,
    pub central_sha256: String,
    /// The zip's entries, in the order of its central directory.
    pub entries: Vec<IndexEntry>,
}

/// One entry of a zip release.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexEntry {
    /// The entry's name, as in the zip.
    pub path: String,
    /// Its unpacked size in bytes.
    pub size: u64,
    /// The SHA-256 of its unpacked bytes.
    pub sha256: String,
    /// Where its local header starts in the archive.
    pub header: u64,
    /// Where its compressed bytes start, past the local header.
    pub data: u64,
    /// Where its compressed bytes end.
    pub end: u64,
}

impl IndexEntry {
    /// Whether the entry is a folder.
    pub fn is_dir(&self) -> bool {
        self.path.ends_with('/') || self.path.ends_with('\\')
    }
}

fn is_sha256(hex: &str) -> bool {
    from_hex::<32>(hex).is_some() && !hex.chars().any(|c| c.is_uppercase())
}

impl Index {
    /// Whether every field is in shape and the entries fit in the archive.
    pub fn validate(&self) -> Result<(), String> {
        if self.format != INDEX_FORMAT {
            return Err(format!("unknown index format {}", self.format));
        }
        if self.archive_size == 0 || self.archive_size > MAX_ARCHIVE_BYTES {
            return Err("archive size".into());
        }
        if !is_sha256(&self.archive_sha256) || !is_sha256(&self.central_sha256) {
            return Err("sha256".into());
        }
        if self.central_start >= self.archive_size {
            return Err("central directory".into());
        }
        if self.entries.len() > MAX_INDEX_ENTRIES {
            return Err("too many entries".into());
        }
        for entry in &self.entries {
            if entry.path.is_empty() || entry.path.len() > 4096 || !is_sha256(&entry.sha256) {
                return Err(format!("entry {:?}", entry.path));
            }
            if !(entry.header <= entry.data
                && entry.data <= entry.end
                && entry.end <= self.central_start)
            {
                return Err(format!("entry {:?} is out of place", entry.path));
            }
        }
        Ok(())
    }

    /// The index in `bytes`, once it is in shape.
    pub fn parse(bytes: &[u8]) -> Result<Index, String> {
        let index: Index = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        index.validate()?;
        Ok(index)
    }

    /// The index as the file `hearken release` writes.
    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string(self).unwrap_or_default();
        text.push('\n');
        text
    }
}

/// The name an archive's index is published under:
/// `my-game-1.4.0-windows-x64.index.json` beside
/// `my-game-1.4.0-windows-x64.zip`. `None` for an archive that is not a zip.
pub fn index_name(archive: &str) -> Option<String> {
    let lower = archive.to_ascii_lowercase();
    lower
        .ends_with(".zip")
        .then(|| format!("{}.index.json", &archive[..archive.len() - 4]))
}

/// The SHA-256 of `reader`'s bytes and how many there were.
pub(crate) fn hash_reader(mut reader: impl Read) -> io::Result<(u64, String)> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    Ok((total, to_hex(&hasher.finalize())))
}

/// The index of the zip at `archive`, read from the zip itself.
pub fn build(archive: &Path) -> io::Result<Index> {
    let archive_size = fs::metadata(archive)?.len();
    let archive_sha256 = crate::download::sha256_file(archive)?;
    let file = fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(io::BufReader::new(file)).map_err(io::Error::other)?;
    if zip.offset() != 0 {
        return Err(io::Error::other(
            "the zip starts part-way into its file, which the index does not describe",
        ));
    }
    let central_start = zip.central_directory_start();
    let mut entries = Vec::with_capacity(zip.len());
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(io::Error::other)?;
        let path = entry.name().to_string();
        let header = entry.header_start();
        let data = entry.data_start();
        let end = data + entry.compressed_size();
        let (size, sha256) = hash_reader(&mut entry)?;
        entries.push(IndexEntry {
            path,
            size,
            sha256,
            header,
            data,
            end,
        });
    }
    let mut central = fs::File::open(archive)?;
    io::Seek::seek(&mut central, io::SeekFrom::Start(central_start))?;
    let (_, central_sha256) = hash_reader(central)?;
    let index = Index {
        format: INDEX_FORMAT,
        archive_size,
        archive_sha256,
        central_start,
        central_sha256,
        entries,
    };
    index.validate().map_err(io::Error::other)?;
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::tests::zip_file;

    #[test]
    fn an_index_lists_every_entry_where_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("g-1.0.0-windows-x64.zip");
        let big = vec![b'a'; 10_000];
        zip_file(
            &archive,
            &[("g/", b""), ("g/g.exe", b"MZ"), ("g/data/big.bin", &big)],
        );
        let index = build(&archive).unwrap();
        assert_eq!(index.entries.len(), 3);
        assert_eq!(index.archive_size, fs::metadata(&archive).unwrap().len());
        let bytes = fs::read(&archive).unwrap();
        let big_entry = &index.entries[2];
        assert_eq!(big_entry.path, "g/data/big.bin");
        assert_eq!(big_entry.size, 10_000);
        assert_eq!(big_entry.sha256, to_hex(&Sha256::digest(&big)));
        // The local header is where it says, and its data follows.
        assert_eq!(
            &bytes[big_entry.header as usize..big_entry.header as usize + 4],
            b"PK\x03\x04"
        );
        assert!(big_entry.data > big_entry.header && big_entry.end > big_entry.data);
        assert_eq!(
            &bytes[index.central_start as usize..index.central_start as usize + 4],
            b"PK\x01\x02"
        );
        assert!(index.entries[0].is_dir());
        // It reads back as written.
        assert_eq!(Index::parse(index.to_json().as_bytes()), Ok(index));
    }

    #[test]
    fn an_index_out_of_shape_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        zip_file(&archive, &[("a", b"1")]);
        let good = build(&archive).unwrap();
        let cases: Vec<fn(&mut Index)> = vec![
            |i| i.format = 2,
            |i| i.archive_size = 0,
            |i| i.archive_sha256 = "AB".repeat(32),
            |i| i.central_start = i.archive_size,
            |i| i.entries[0].end = i.central_start + 1,
            |i| i.entries[0].header = i.entries[0].data + 1,
            |i| i.entries[0].sha256 = "x".into(),
            |i| i.entries[0].path = String::new(),
        ];
        for (n, change) in cases.into_iter().enumerate() {
            let mut index = good.clone();
            change(&mut index);
            assert!(index.validate().is_err(), "case {n}");
            assert!(
                Index::parse(index.to_json().as_bytes()).is_err(),
                "case {n}"
            );
        }
        assert!(Index::parse(b"<html>").is_err());
    }

    #[test]
    fn only_a_zip_has_an_index() {
        assert_eq!(
            index_name("my-game-1.4.0-windows-x64.zip").as_deref(),
            Some("my-game-1.4.0-windows-x64.index.json")
        );
        assert_eq!(index_name("my-game-1.4.0-linux-x64.tar.gz"), None);
        let name = index_name("a-1.0.0-macos.ZIP").unwrap();
        assert!(crate::manifest::is_file_name(&name), "{name}");
    }
}
