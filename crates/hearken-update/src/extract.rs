//! Unpacking an archive as untrusted input.
//!
//! The archive was checked against its signed hash before it gets here, so
//! only the developer could have made it. It is still unpacked as if anyone
//! had: a slip in a release script, or a stolen key, should not be able to
//! write outside the folder it is unpacked into. So every entry is checked:
//!
//! - its path is relative, with no `..`, no drive or root, and no `:`,
//!   and on Windows no device name (`CON`, `NUL`) or trailing dot or space;
//! - it is a file, a folder or a symbolic link, never a device, a pipe or a
//!   hard link;
//! - a link points down from where it is (no `..`, not absolute), and no
//!   entry is written through a link, so nothing reaches outside;
//! - there are at most [`Limits::entries`] of them, and at most
//!   [`Limits::bytes`] in all, counted as they are written, whatever the
//!   archive's headers claim. [`crate::install::stage`] also holds the
//!   bytes to [`MAX_UNPACK_RATIO`] times the archive's size.
//!
//! A `__MACOSX` folder and `._` files (macOS resource forks and extended
//! attributes) are skipped.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::manifest::{archive_kind, ArchiveKind};

/// How much an archive may unpack to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub entries: u64,
    pub bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            entries: 200_000,
            bytes: 32 * 1024 * 1024 * 1024,
        }
    }
}

/// How many times its own size an archive may unpack to. Game files
/// (sound, code, data) rarely shrink more than four times; a far smaller
/// archive that unpacks huge is a bomb, and would fill the temporary folder.
pub const MAX_UNPACK_RATIO: u64 = 20;

/// What any archive may unpack to, however small it is.
pub const MIN_UNPACK_BYTES: u64 = 32 * 1024 * 1024;

impl Limits {
    /// The default limits, with the bytes held to [`MAX_UNPACK_RATIO`]
    /// times an archive of `size` bytes (never under [`MIN_UNPACK_BYTES`]).
    pub fn for_archive(size: u64) -> Limits {
        Limits::default().at_most(Limits {
            entries: u64::MAX,
            bytes: size.saturating_mul(MAX_UNPACK_RATIO).max(MIN_UNPACK_BYTES),
        })
    }

    /// The tighter of `self` and `other`, limit by limit.
    pub fn at_most(self, other: Limits) -> Limits {
        Limits {
            entries: self.entries.min(other.entries),
            bytes: self.bytes.min(other.bytes),
        }
    }
}

fn refuse(why: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.into())
}

fn cancelled_error() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "the update was canceled")
}

/// An entry's name as a path inside the destination, or `None` for a name
/// that could reach outside it. Backslashes count as separators, since a
/// Windows tool may write them.
pub fn safe_path(name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains('\0') {
        return None;
    }
    let name = name.replace('\\', "/");
    if name.starts_with('/') {
        return None;
    }
    let mut out = PathBuf::new();
    for part in name.split('/') {
        match part {
            "" | "." => continue,
            ".." => return None,
            // A drive (`C:`) or an alternate data stream (`file:stream`).
            part if part.contains(':') => return None,
            part if cfg!(windows) && !windows_plain_name(part) => return None,
            part => out.push(part),
        }
    }
    // Belt and braces: what Path makes of it must be plain names too.
    if out.as_os_str().is_empty() || !out.components().all(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(out)
}

/// Whether Windows would open `part` as the file it names. `CON`, `NUL`,
/// `COM1` and the like (with any extension) open a device instead, and a
/// trailing dot or space is dropped, so `a.` would land on `a`. Only
/// Windows refuses these: a game elsewhere may well ship `aux.ogg`.
pub fn windows_plain_name(part: &str) -> bool {
    if part.ends_with(['.', ' ']) {
        return false;
    }
    let stem = part.split('.').next().unwrap_or(part).trim_end();
    let stem = stem.to_ascii_uppercase();
    let numbered = (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.chars().count() == 4
        && stem
            .chars()
            .nth(3)
            .is_some_and(|c| c.is_ascii_digit() || matches!(c, '¹' | '²' | '³'));
    let named = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    );
    !(named || numbered)
}

/// Whether a link's target stays below the link: relative, no `..`.
fn safe_link_target(target: &str) -> bool {
    !target.is_empty()
        && !target.contains('\0')
        && !target.starts_with(['/', '\\'])
        && target
            .split(['/', '\\'])
            .all(|part| part != ".." && !part.contains(':'))
}

/// Whether to skip an entry: Finder's resource-fork folder, or an
/// AppleDouble file (`._name`) that `ditto` writes for a file's extended
/// attributes. Inside an app those would break its code signature.
fn skipped(path: &Path) -> bool {
    path.components()
        .next()
        .is_some_and(|c| c.as_os_str() == "__MACOSX")
        || path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("._"))
}

/// What one entry is.
enum Kind {
    Dir,
    File { mode: Option<u32> },
    Link(String),
}

/// Writes entries under `dest`, keeping count.
struct Writer<'a> {
    dest: &'a Path,
    limits: Limits,
    entries: u64,
    bytes: u64,
    cancelled: Option<&'a AtomicBool>,
}

impl Writer<'_> {
    /// Refuses `rel` when a folder on the way to it, or the entry itself,
    /// is a link: writing through a link could land anywhere.
    fn no_links_on_the_way(&self, rel: &Path) -> io::Result<()> {
        let mut at = self.dest.to_path_buf();
        for part in rel.components() {
            at.push(part);
            match fs::symlink_metadata(&at) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(refuse(format!(
                        "{} would be written through a link",
                        rel.display()
                    )))
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn entry(&mut self, name: &str, kind: Kind, body: &mut dyn Read) -> io::Result<()> {
        if self.cancelled.is_some_and(|c| c.load(Ordering::SeqCst)) {
            return Err(cancelled_error());
        }
        self.entries += 1;
        if self.entries > self.limits.entries {
            return Err(refuse("the archive has too many entries"));
        }
        let rel = safe_path(name).ok_or_else(|| refuse(format!("unsafe entry name {name:?}")))?;
        if skipped(&rel) {
            return Ok(());
        }
        self.no_links_on_the_way(&rel)?;
        let path = self.dest.join(&rel);
        match kind {
            Kind::Dir => fs::create_dir_all(&path),
            Kind::File { mode } => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                if path.is_dir() {
                    return Err(refuse(format!("{name} is a folder and a file")));
                }
                let mut file = fs::File::create(&path)?;
                let room = self.limits.bytes - self.bytes;
                let written = io::copy(&mut body.take(room.saturating_add(1)), &mut file)?;
                if written > room {
                    return Err(refuse("the archive unpacks to more than allowed"));
                }
                self.bytes += written;
                file.flush()?;
                set_mode(&path, mode)
            }
            Kind::Link(target) => {
                if !safe_link_target(&target) {
                    return Err(refuse(format!("{name} links outside the update")));
                }
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                if fs::symlink_metadata(&path).is_ok() {
                    return Err(refuse(format!("{name} is in the archive twice")));
                }
                make_link(&target, &path)
            }
        }
    }
}

/// A file's permissions from the archive, without set-id bits or write
/// access for others. Without a mode, the file keeps the default.
fn set_mode(path: &Path, mode: Option<u32>) -> io::Result<()> {
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        let mode = (mode & 0o777 & !0o022) | 0o600;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    let _ = (path, mode);
    Ok(())
}

#[cfg(unix)]
fn make_link(target: &str, path: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, path)
}

#[cfg(not(unix))]
fn make_link(_target: &str, path: &Path) -> io::Result<()> {
    Err(refuse(format!(
        "{} is a link, which this system's updates do not hold",
        path.display()
    )))
}

/// Unpacks `archive` (a `.zip` or `.tar.gz`, by its name) into `dest`.
pub fn extract(
    archive: &Path,
    dest: &Path,
    limits: Limits,
    cancelled: Option<&AtomicBool>,
) -> io::Result<()> {
    fs::create_dir_all(dest)?;
    let name = archive
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut writer = Writer {
        dest,
        limits,
        entries: 0,
        bytes: 0,
        cancelled,
    };
    match archive_kind(&name) {
        Some(ArchiveKind::Zip) => extract_zip(archive, &mut writer),
        Some(ArchiveKind::TarGz) => extract_tar_gz(archive, &mut writer),
        _ => Err(refuse(format!(
            "{name} is not an archive this updater unpacks"
        ))),
    }
}

const S_IFMT: u32 = 0o170000;
const S_IFREG: u32 = 0o100000;
const S_IFDIR: u32 = 0o040000;
const S_IFLNK: u32 = 0o120000;

/// The most a link's target may be.
const MAX_LINK_BYTES: u64 = 4096;

fn extract_zip(archive: &Path, writer: &mut Writer) -> io::Result<()> {
    let file = fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(io::BufReader::new(file)).map_err(io::Error::other)?;
    if zip.len() as u64 > writer.limits.entries {
        return Err(refuse("the archive has too many entries"));
    }
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(io::Error::other)?;
        let name = entry.name().to_string();
        let mode = entry.unix_mode();
        let kind = match mode.map(|m| m & S_IFMT) {
            _ if entry.is_dir() => Kind::Dir,
            Some(S_IFDIR) => Kind::Dir,
            Some(S_IFLNK) => {
                let mut target = String::new();
                (&mut entry)
                    .take(MAX_LINK_BYTES)
                    .read_to_string(&mut target)?;
                Kind::Link(target)
            }
            None | Some(0) | Some(S_IFREG) => Kind::File { mode },
            Some(_) => return Err(refuse(format!("{name} is not a file, folder or link"))),
        };
        writer.entry(&name, kind, &mut entry)?;
    }
    Ok(())
}

fn extract_tar_gz(archive: &Path, writer: &mut Writer) -> io::Result<()> {
    let file = fs::File::open(archive)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(io::BufReader::new(file)));
    for entry in tar.entries()? {
        let mut entry = entry?;
        let header = entry.header();
        let name = String::from_utf8(entry.path_bytes().into_owned())
            .map_err(|_| refuse("an entry's name is not UTF-8"))?;
        let kind = match header.entry_type() {
            tar::EntryType::Regular | tar::EntryType::Continuous => Kind::File {
                mode: header.mode().ok(),
            },
            tar::EntryType::Directory => Kind::Dir,
            tar::EntryType::Symlink => {
                let target = entry
                    .link_name_bytes()
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .unwrap_or_default();
                Kind::Link(target)
            }
            // Extended headers, read by the tar crate for the next entry.
            tar::EntryType::XGlobalHeader | tar::EntryType::XHeader => continue,
            other => {
                return Err(refuse(format!(
                    "{name} is not a file, folder or link ({other:?})"
                )))
            }
        };
        writer.entry(&name, kind, &mut entry)?;
    }
    Ok(())
}

/// The folder an unpacked archive's files are in: its one top folder, when
/// it has one and nothing else, else the destination itself.
pub fn unpacked_root(dest: &Path) -> io::Result<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dest)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name() != "__MACOSX")
        .map(|e| e.path())
        .collect();
    match entries.as_mut_slice() {
        [only] if fs::symlink_metadata(&*only)?.is_dir() => Ok(std::mem::take(only)),
        _ => Ok(dest.to_path_buf()),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn tar_gz(
        path: &Path,
        build: impl FnOnce(&mut tar::Builder<flate2::write::GzEncoder<fs::File>>),
    ) {
        let file = fs::File::create(path).unwrap();
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            file,
            flate2::Compression::fast(),
        ));
        build(&mut builder);
        builder.into_inner().unwrap().finish().unwrap();
    }

    pub fn file_header(size: u64, mode: u32) -> tar::Header {
        let mut header = tar::Header::new_gnu();
        header.set_size(size);
        header.set_mode(mode);
        header.set_entry_type(tar::EntryType::Regular);
        header
    }

    /// A raw tar header whose name the tar crate's builder would refuse to
    /// write, as an attacker's tool would not.
    fn raw_entry(
        builder: &mut tar::Builder<flate2::write::GzEncoder<fs::File>>,
        name: &str,
        kind: tar::EntryType,
        link: &str,
        body: &[u8],
    ) {
        let mut header = tar::Header::new_old();
        header.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
        header.as_old_mut().linkname[..link.len()].copy_from_slice(link.as_bytes());
        header.set_size(body.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(kind);
        header.set_cksum();
        builder.append(&header, body).unwrap();
    }

    pub fn zip_file(path: &Path, entries: &[(&str, &[u8])]) {
        let file = fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
        for (name, body) in entries {
            if name.ends_with('/') {
                zip.add_directory(*name, options).unwrap();
            } else {
                zip.start_file(*name, options).unwrap();
                zip.write_all(body).unwrap();
            }
        }
        zip.finish().unwrap();
    }

    fn unpack(archive: &Path, dest: &Path) -> io::Result<()> {
        extract(archive, dest, Limits::default(), None)
    }

    #[test]
    fn names_that_reach_outside_are_refused() {
        for name in [
            "../evil",
            "a/../../evil",
            "/etc/passwd",
            "\\windows\\evil",
            "C:/evil",
            "C:evil",
            "a\\..\\..\\evil",
            "file:stream",
            "",
            ".",
            "a/\0b",
        ] {
            assert_eq!(safe_path(name), None, "{name:?}");
        }
        assert_eq!(
            safe_path("game/./bin/game"),
            Some(PathBuf::from("game/bin/game"))
        );
        assert_eq!(
            safe_path("game\\sounds\\a.ogg"),
            Some(PathBuf::from("game/sounds/a.ogg"))
        );
        assert_eq!(safe_path("dir/"), Some(PathBuf::from("dir")));
    }

    #[test]
    fn windows_device_names_are_not_plain_there() {
        for name in [
            "CON", "nul", "aux.ogg", "Com1", "lpt9.txt", "COM¹", "conin$", "a.", "a ",
        ] {
            assert!(!windows_plain_name(name), "{name:?}");
        }
        for name in [
            "console.ogg",
            "com10",
            "lpt",
            "auxiliary",
            "a.b",
            "nul_ref.txt",
        ] {
            assert!(windows_plain_name(name), "{name:?}");
        }
        // Only Windows refuses them: elsewhere they are ordinary files.
        assert_eq!(safe_path("game/aux.ogg").is_none(), cfg!(windows));
    }

    #[test]
    fn a_plain_release_unpacks_with_its_modes() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("game.tar.gz");
        tar_gz(&archive, |b| {
            b.append_data(
                &mut file_header(4, 0o4755),
                "game-1.0.0/game",
                &b"\x7fELF"[..],
            )
            .unwrap();
            b.append_data(
                &mut file_header(2, 0o666),
                "game-1.0.0/sounds/a.txt",
                &b"hi"[..],
            )
            .unwrap();
        });
        let dest = dir.path().join("out");
        unpack(&archive, &dest).unwrap();
        let root = unpacked_root(&dest).unwrap();
        assert_eq!(root, dest.join("game-1.0.0"));
        assert_eq!(fs::read(root.join("sounds/a.txt")).unwrap(), b"hi");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o7777;
            // No set-id bit, no write for others.
            assert_eq!(mode(&root.join("game")), 0o755);
            assert_eq!(mode(&root.join("sounds/a.txt")), 0o644);
        }
    }

    #[test]
    fn a_zip_unpacks_and_its_root_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("game.zip");
        zip_file(
            &archive,
            &[
                ("game-1.0.0/", b""),
                ("game-1.0.0/game.exe", b"MZ"),
                ("__MACOSX/._game.exe", b"junk"),
                ("game-1.0.0/._game.exe", b"junk"),
            ],
        );
        let dest = dir.path().join("out");
        unpack(&archive, &dest).unwrap();
        assert!(!dest.join("__MACOSX").exists());
        assert!(!dest.join("game-1.0.0/._game.exe").exists());
        assert_eq!(unpacked_root(&dest).unwrap(), dest.join("game-1.0.0"));
        // No wrapper folder: the files are the root.
        let flat = dir.path().join("flat.zip");
        zip_file(&flat, &[("game.exe", b"MZ"), ("data/a", b"1")]);
        let out = dir.path().join("flat");
        unpack(&flat, &out).unwrap();
        assert_eq!(unpacked_root(&out).unwrap(), out);
    }

    #[test]
    fn a_zip_entry_reaching_outside_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("evil.zip");
        zip_file(&archive, &[("ok.txt", b"1"), ("../evil.txt", b"2")]);
        let dest = dir.path().join("out");
        assert!(unpack(&archive, &dest).is_err());
        assert!(!dir.path().join("evil.txt").exists());
    }

    #[test]
    fn tar_traversal_devices_and_hard_links_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let cases: Vec<(&str, tar::EntryType, &str)> = vec![
            ("../evil", tar::EntryType::Regular, ""),
            ("/tmp/evil", tar::EntryType::Regular, ""),
            ("dev", tar::EntryType::Char, ""),
            ("blk", tar::EntryType::Block, ""),
            ("fifo", tar::EntryType::Fifo, ""),
            ("hard", tar::EntryType::Link, "/etc/passwd"),
            ("up", tar::EntryType::Symlink, "../../etc"),
            ("abs", tar::EntryType::Symlink, "/etc"),
        ];
        for (i, (name, kind, link)) in cases.into_iter().enumerate() {
            let archive = dir.path().join(format!("evil{i}.tar.gz"));
            tar_gz(&archive, |b| raw_entry(b, name, kind, link, b""));
            let dest = dir.path().join(format!("out{i}"));
            assert!(unpack(&archive, &dest).is_err(), "{name} {kind:?}");
        }
        assert!(!dir.path().join("evil").exists());
    }

    #[cfg(unix)]
    #[test]
    fn nothing_is_written_through_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("link.tar.gz");
        // A link pointing down is fine on its own; writing through it is not.
        tar_gz(&archive, |b| {
            raw_entry(b, "game/here", tar::EntryType::Symlink, ".", b"");
            raw_entry(b, "game/here/x", tar::EntryType::Regular, "", b"x");
        });
        let dest = dir.path().join("out");
        let err = unpack(&archive, &dest).unwrap_err();
        assert!(err.to_string().contains("through a link"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn links_inside_the_update_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("app.tar.gz");
        tar_gz(&archive, |b| {
            raw_entry(
                b,
                "My.app/Versions/A/lib",
                tar::EntryType::Regular,
                "",
                b"lib",
            );
            raw_entry(
                b,
                "My.app/Versions/Current",
                tar::EntryType::Symlink,
                "A",
                b"",
            );
        });
        let dest = dir.path().join("out");
        unpack(&archive, &dest).unwrap();
        let link = dest.join("My.app/Versions/Current");
        assert_eq!(fs::read_link(&link).unwrap(), PathBuf::from("A"));
        assert_eq!(fs::read(link.join("lib")).unwrap(), b"lib");
    }

    #[test]
    fn size_and_entry_limits_hold_whatever_the_headers_say() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("big.zip");
        let big = vec![0u8; 100_000];
        zip_file(&archive, &[("a", &big), ("b", &big)]);
        let tight = Limits {
            entries: 10,
            bytes: 150_000,
        };
        let err = extract(&archive, &dir.path().join("o1"), tight, None).unwrap_err();
        assert!(err.to_string().contains("more than allowed"), "{err}");
        let few = Limits {
            entries: 1,
            bytes: u64::MAX / 2,
        };
        let err = extract(&archive, &dir.path().join("o2"), few, None).unwrap_err();
        assert!(err.to_string().contains("too many entries"), "{err}");
        let tar = dir.path().join("many.tar.gz");
        tar_gz(&tar, |b| {
            for i in 0..5 {
                b.append_data(&mut file_header(1, 0o644), format!("f{i}"), &b"x"[..])
                    .unwrap();
            }
        });
        let few = Limits {
            entries: 3,
            bytes: 1000,
        };
        assert!(extract(&tar, &dir.path().join("o3"), few, None).is_err());
    }

    #[test]
    fn the_unpack_limit_follows_the_archive_size() {
        assert_eq!(Limits::for_archive(0).bytes, MIN_UNPACK_BYTES);
        assert_eq!(Limits::for_archive(1000).bytes, MIN_UNPACK_BYTES);
        assert_eq!(
            Limits::for_archive(100 * 1024 * 1024).bytes,
            2000 * 1024 * 1024
        );
        // Never above the default, however big the archive.
        assert_eq!(Limits::for_archive(u64::MAX).bytes, Limits::default().bytes);
        assert_eq!(Limits::for_archive(1).entries, Limits::default().entries);
    }

    #[test]
    fn a_cancel_stops_unpacking() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        zip_file(&archive, &[("a", b"1")]);
        let cancelled = AtomicBool::new(true);
        let err = extract(
            &archive,
            &dir.path().join("o"),
            Limits::default(),
            Some(&cancelled),
        )
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn an_unknown_archive_is_not_unpacked() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("a.rar");
        fs::write(&archive, b"x").unwrap();
        assert!(unpack(&archive, &dir.path().join("o")).is_err());
    }
}
