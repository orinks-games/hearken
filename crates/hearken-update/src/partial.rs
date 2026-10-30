//! Downloading only the files that changed.
//!
//! A zip release can carry a signed file index ([`crate::index`]). With it,
//! a copy finds the files it already has, byte for byte (the same size and
//! the signed SHA-256, hashed here, never taken from the zip's CRC-32), and
//! downloads the rest of the archive with HTTP range requests: every other
//! entry, the local headers, and the central directory. The archive is put
//! together in the staging folder at its full size, the parts it skipped
//! left empty, and unpacked by the same code as a whole download, with the
//! player's own files copied in for the parts it skipped. Every file and
//! link it writes is checked against the signed index once it is on disk.
//!
//! Anything off falls back to downloading the whole archive, with all of
//! that path's checks: a server that ignores ranges or answers another
//! range, too few or too many bytes, a central directory or a file that is
//! not the signed one, a file of the player's that changed since the check,
//! or any other error. Only a cancel stops it.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::check::Update;
use crate::download::{self, sha256_file, stream_to_file, DownloadError, IDLE_TIMEOUT};
use crate::extract::{self, safe_path, Assembly, Limits};
use crate::fetch::{Fetch, RangeBody};
use crate::index::{hash_reader, Index};
use crate::install::{self, Install, Staged, UNPACKED};
use crate::manifest::{archive_kind, ArchiveKind};

/// The smallest file worth keeping rather than downloading: anything
/// smaller costs about as much to ask for around as to fetch.
pub const MIN_REUSE_BYTES: u64 = 64 * 1024;

/// The least a partial download must save over the whole archive to be
/// worth its extra requests.
pub const MIN_SAVING_BYTES: u64 = 1024 * 1024;

/// Parts of the archive this close together are fetched in one request,
/// gap and all.
pub const MERGE_GAP: u64 = 64 * 1024;

/// The most requests a partial download makes. More, and the whole archive
/// is simpler.
pub const MAX_RANGES: usize = 1000;

/// When a partial download is worth making. The defaults are the
/// constants above; tests use smaller ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thresholds {
    pub min_reuse: u64,
    pub min_saving: u64,
    pub merge_gap: u64,
    pub max_ranges: usize,
}

impl Default for Thresholds {
    fn default() -> Self {
        Thresholds {
            min_reuse: MIN_REUSE_BYTES,
            min_saving: MIN_SAVING_BYTES,
            merge_gap: MERGE_GAP,
            max_ranges: MAX_RANGES,
        }
    }
}

/// A partial download: the files to keep from the install, and the parts of
/// the archive to fetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The signed index, checked against the manifest.
    pub index: Index,
    /// Entries (by position in the index) the install already has, and
    /// where.
    pub reuse: BTreeMap<usize, PathBuf>,
    /// The parts of the archive to fetch, in order, none overlapping.
    pub ranges: Vec<Range<u64>>,
}

impl Plan {
    /// The bytes it downloads.
    pub fn download_size(&self) -> u64 {
        self.ranges.iter().map(|r| r.end - r.start).sum()
    }
}

/// What a download is doing, for the screen that shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Downloading,
    Unpacking,
}

/// The partial download for `update` over `install`, when there is one
/// worth making: the release is a zip with a signed index, `fetch` asks
/// for ranges, and the install already has enough of it. Hashes the
/// install's larger files, so it runs off the game loop. `None` for
/// anything else, and the whole archive is downloaded.
pub fn plan(update: &Update, fetch: &dyn Fetch, install: &Install) -> Option<Plan> {
    plan_inner(update, fetch, install, Thresholds::default(), None)
}

/// [`plan`] that stops, with `None`, once `cancelled` is set: between the
/// install's files and within each one's hashing.
pub fn plan_until(
    update: &Update,
    fetch: &dyn Fetch,
    install: &Install,
    cancelled: &AtomicBool,
) -> Option<Plan> {
    plan_inner(
        update,
        fetch,
        install,
        Thresholds::default(),
        Some(cancelled),
    )
}

/// [`plan`] with other thresholds.
pub fn plan_with(
    update: &Update,
    fetch: &dyn Fetch,
    install: &Install,
    thresholds: Thresholds,
) -> Option<Plan> {
    plan_inner(update, fetch, install, thresholds, None)
}

fn plan_inner(
    update: &Update,
    fetch: &dyn Fetch,
    install: &Install,
    thresholds: Thresholds,
    cancelled: Option<&AtomicBool>,
) -> Option<Plan> {
    let manifest = &update.manifest;
    let wanted = manifest.index.as_ref()?;
    if archive_kind(&manifest.archive) != Some(ArchiveKind::Zip) || !fetch.supports_ranges() {
        return None;
    }
    let url = update.index_url.as_deref()?;
    let bytes = match fetch.get(url, &[], wanted.size) {
        Ok(bytes) => bytes,
        Err(e) => {
            log::info!("no file index for the update ({e}); it downloads whole");
            return None;
        }
    };
    let (size, sha256) = hash_reader(&bytes[..]).ok()?;
    if size != wanted.size || sha256 != wanted.sha256 {
        log::warn!("the update's file index is not the one its manifest signed");
        return None;
    }
    let index = match Index::parse(&bytes) {
        Ok(index) => index,
        Err(why) => {
            log::warn!("the update's file index is malformed: {why}");
            return None;
        }
    };
    if index.archive_size != manifest.size || index.archive_sha256 != manifest.sha256 {
        log::warn!("the update's file index is for another archive");
        return None;
    }
    let plan = plan_from_index_inner(index, install, thresholds, cancelled);
    if let Some(plan) = &plan {
        log::info!(
            "the update downloads {} of its {} bytes: {} files are already here",
            plan.download_size(),
            manifest.size,
            plan.reuse.len()
        );
    }
    plan
}

/// The plan for a checked `index` over `install`.
pub fn plan_from_index(index: Index, install: &Install, thresholds: Thresholds) -> Option<Plan> {
    plan_from_index_inner(index, install, thresholds, None)
}

/// The SHA-256 of the file at `path`, or `None` once `cancelled` is set.
fn sha256_file_until(path: &Path, cancelled: Option<&AtomicBool>) -> Option<String> {
    let Some(cancelled) = cancelled else {
        return sha256_file(path).ok();
    };
    let file = fs::File::open(path).ok()?;
    let heeding = Heeding {
        inner: file,
        cancelled,
    };
    hash_reader(heeding).ok().map(|(_, sum)| sum)
}

/// A reader that fails, for good, once `cancelled` is set.
struct Heeding<'a, R> {
    inner: R,
    cancelled: &'a AtomicBool,
}

impl<R: Read> Read for Heeding<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancelled.load(Ordering::SeqCst) {
            return Err(io::Error::other("the check was canceled"));
        }
        self.inner.read(buf)
    }
}

fn plan_from_index_inner(
    index: Index,
    install: &Install,
    thresholds: Thresholds,
    cancelled: Option<&AtomicBool>,
) -> Option<Plan> {
    let installed = install_paths(&index, install);
    let mut reuse = BTreeMap::new();
    for (i, (entry, path)) in index.entries.iter().zip(installed).enumerate() {
        if cancelled.is_some_and(|c| c.load(Ordering::SeqCst)) {
            return None;
        }
        let Some(path) = path else { continue };
        if entry.is_dir() || entry.size < thresholds.min_reuse {
            continue;
        }
        let same_size = fs::symlink_metadata(&path)
            .is_ok_and(|meta| meta.is_file() && meta.len() == entry.size);
        if same_size && sha256_file_until(&path, cancelled).is_some_and(|sum| sum == entry.sha256) {
            reuse.insert(i, path);
        }
    }
    if reuse.is_empty() {
        return None;
    }
    let mut parts: Vec<Range<u64>> = index
        .entries
        .iter()
        .enumerate()
        .map(|(i, entry)| match reuse.contains_key(&i) {
            // The local header only: the unpack reads where the data starts.
            true => entry.header..entry.data,
            false => entry.header..entry.end,
        })
        .chain(std::iter::once(index.central_start..index.archive_size))
        .filter(|r| !r.is_empty())
        .collect();
    parts.sort_by_key(|r| r.start);
    let mut ranges: Vec<Range<u64>> = Vec::new();
    for part in parts {
        match ranges.last_mut() {
            Some(last) if part.start <= last.end.saturating_add(thresholds.merge_gap) => {
                last.end = last.end.max(part.end);
            }
            _ => ranges.push(part),
        }
    }
    let plan = Plan {
        index,
        reuse,
        ranges,
    };
    let worth = plan.download_size().saturating_add(thresholds.min_saving)
        <= plan.index.archive_size
        && plan.ranges.len() <= thresholds.max_ranges;
    worth.then_some(plan)
}

/// For each entry of `index`, where the install has the file it unpacks
/// to, laid out as staging lays out the update: below the archive's one top
/// folder, and for an app, below the app. `None` for an entry outside what
/// replaces the install, or one the unpack skips.
pub fn install_paths(index: &Index, install: &Install) -> Vec<Option<PathBuf>> {
    let rels: Vec<Option<PathBuf>> = index
        .entries
        .iter()
        .map(|e| safe_path(&e.path).filter(|p| !extract::skipped(p)))
        .collect();
    let written = || rels.iter().flatten();
    // `extract::unpacked_root`: the one top folder, when there is nothing
    // else.
    let tops: BTreeSet<OsString> = written()
        .filter_map(|p| p.components().next())
        .map(|c| c.as_os_str().to_os_string())
        .collect();
    let mut root = PathBuf::new();
    if let [top] = tops.into_iter().collect::<Vec<_>>().as_slice() {
        let is_dir = index.entries.iter().zip(&rels).any(|(e, rel)| {
            rel.as_ref().is_some_and(|r| {
                r.components().count() > 1 || (e.is_dir() && r.as_os_str() == top.as_os_str())
            })
        });
        if is_dir {
            root = PathBuf::from(top);
        }
    }
    let (base, target) = match install {
        Install::Folder { dir, .. } => (root, dir.clone()),
        Install::Bundle { app, .. } => {
            let name = app.file_name().unwrap_or_default();
            match bundle_in(&root, name, written()) {
                Some(bundle) => (bundle, app.clone()),
                None => return vec![None; rels.len()],
            }
        }
        Install::AppImage { .. } => return vec![None; rels.len()],
    };
    rels.iter()
        .map(|rel| {
            let inside = rel.as_ref()?.strip_prefix(&base).ok()?;
            (!inside.as_os_str().is_empty()).then(|| target.join(inside))
        })
        .collect()
}

/// `install::find_bundle`, over the paths an update unpacks to: `root`
/// itself when it is an app, else the app in it named `name`, else its
/// only app.
fn bundle_in<'a>(
    root: &Path,
    name: &std::ffi::OsStr,
    written: impl Iterator<Item = &'a PathBuf>,
) -> Option<PathBuf> {
    if root.extension().is_some_and(|e| e == "app") {
        return Some(root.to_path_buf());
    }
    // Folders directly in the root: those with something inside them.
    let mut folders = BTreeSet::new();
    for path in written {
        let Ok(inside) = path.strip_prefix(root) else {
            continue;
        };
        let mut parts = inside.components();
        if let (Some(Component::Normal(first)), Some(_)) = (parts.next(), parts.next()) {
            folders.insert(first.to_os_string());
        }
    }
    if folders.contains(name) {
        return Some(root.join(name));
    }
    let apps: Vec<&OsString> = folders
        .iter()
        .filter(|f| Path::new(f).extension().is_some_and(|e| e == "app"))
        .collect();
    match apps.as_slice() {
        [one] => Some(root.join(one)),
        _ => None,
    }
}

/// Downloads `update` and unpacks it into `staging`, ready to replace
/// `install`: only the changed files when the update has a [`Plan`], else,
/// or when that does not work, the whole archive. `progress(done, total)`
/// hears each chunk, `phase` each change between downloading and
/// unpacking. The result is the same as [`download::download`] and then
/// [`install::stage`].
#[allow(clippy::too_many_arguments)]
pub fn download_and_stage(
    update: &Update,
    fetch: &dyn Fetch,
    staging: &Path,
    install: &Install,
    limits: Limits,
    progress: &mut dyn FnMut(u64, u64),
    phase: &mut dyn FnMut(Phase),
    cancelled: &AtomicBool,
) -> Result<Staged, DownloadError> {
    download_and_stage_with(
        update,
        fetch,
        staging,
        install,
        limits,
        progress,
        phase,
        cancelled,
        IDLE_TIMEOUT,
    )
}

/// [`download_and_stage`] with another idle bound, for tests.
#[allow(clippy::too_many_arguments)]
pub fn download_and_stage_with(
    update: &Update,
    fetch: &dyn Fetch,
    staging: &Path,
    install: &Install,
    limits: Limits,
    progress: &mut dyn FnMut(u64, u64),
    phase: &mut dyn FnMut(Phase),
    cancelled: &AtomicBool,
    idle: Duration,
) -> Result<Staged, DownloadError> {
    if let Some(plan) = &update.plan {
        phase(Phase::Downloading);
        let partial = partial(
            update, plan, fetch, staging, install, limits, progress, phase, cancelled, idle,
        );
        match partial {
            Ok(staged) => return Ok(staged),
            Err(_) if cancelled.load(Ordering::SeqCst) => return Err(DownloadError::Cancelled),
            Err(DownloadError::Cancelled) => return Err(DownloadError::Cancelled),
            Err(e) => {
                log::warn!(
                    "downloading only the changed files did not work ({e}); downloading the \
                     whole update"
                );
                let _ = fs::remove_dir_all(staging.join(UNPACKED));
                let _ = fs::remove_file(staging.join(&update.manifest.archive));
            }
        }
    }
    phase(Phase::Downloading);
    let archive = download::download_with(update, fetch, staging, progress, cancelled, idle)?;
    phase(Phase::Unpacking);
    Ok(install::stage(
        &archive,
        staging,
        install,
        limits,
        Some(cancelled),
    )?)
}

/// Why a partial download is given up, logged; the caller downloads the
/// whole archive.
fn off(why: impl std::fmt::Display) -> DownloadError {
    log::warn!("partial update download: {why}");
    DownloadError::Corrupt
}

/// Whether `body` is exactly `range` of a file of `size` bytes.
pub fn range_answer_ok(body: &RangeBody, range: &Range<u64>, size: u64) -> Result<(), String> {
    if body.status != 206 {
        return Err(format!("the server answered {} to a range", body.status));
    }
    let header = body
        .content_range
        .as_deref()
        .ok_or("no Content-Range")?
        .trim();
    let parsed = header
        .strip_prefix("bytes ")
        .and_then(|rest| rest.trim().split_once('/'))
        .and_then(|(span, total)| {
            let (first, last) = span.split_once('-')?;
            Some((
                first.trim().parse::<u64>().ok()?,
                last.trim().parse::<u64>().ok()?,
                total.trim().parse::<u64>().ok()?,
            ))
        });
    if parsed != Some((range.start, range.end - 1, size)) {
        return Err(format!(
            "asked for bytes {}-{} of {size}, got {header:?}",
            range.start,
            range.end - 1
        ));
    }
    if body.length.is_some_and(|n| n != range.end - range.start) {
        return Err(format!("a range declared {:?} bytes", body.length));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn partial(
    update: &Update,
    plan: &Plan,
    fetch: &dyn Fetch,
    staging: &Path,
    install: &Install,
    limits: Limits,
    progress: &mut dyn FnMut(u64, u64),
    phase: &mut dyn FnMut(Phase),
    cancelled: &AtomicBool,
    idle: Duration,
) -> Result<Staged, DownloadError> {
    let manifest = &update.manifest;
    if plan.index.archive_size != manifest.size || plan.index.archive_sha256 != manifest.sha256 {
        return Err(off("the plan is for another archive"));
    }
    // The archive at its full size: the parts not fetched stay empty, and
    // nothing reads them.
    let path = staging.join(&manifest.archive);
    let file = fs::File::create(&path)?;
    file.set_len(manifest.size)?;
    let total = plan.download_size();
    let mut done = 0u64;
    for range in &plan.ranges {
        if range.is_empty() || range.end > manifest.size {
            return Err(off("a range outside the archive"));
        }
        let len = range.end - range.start;
        let body = fetch
            .open_range(&update.archive_url, range.clone())?
            .ok_or_else(|| off("this connection does not ask for ranges"))?;
        range_answer_ok(&body, range, manifest.size).map_err(off)?;
        let mut part = file.try_clone()?;
        part.seek(SeekFrom::Start(range.start))?;
        let before = done;
        let (got, _) = stream_to_file(
            body.reader,
            part,
            len,
            &mut |n, _| progress(before + n, total),
            Some(cancelled),
            idle,
        )?;
        if got != len {
            return Err(off(format!("a range sent {got} of {len} bytes")));
        }
        done += len;
    }
    drop(file);
    // The central directory is what the unpack trusts for every entry's
    // name, kind and place: it must be the signed one.
    let mut central = fs::File::open(&path)?;
    central.seek(SeekFrom::Start(plan.index.central_start))?;
    let (_, sha256) = hash_reader(central.take(manifest.size - plan.index.central_start))?;
    if sha256 != plan.index.central_sha256 {
        return Err(off("the central directory is not the signed one"));
    }
    phase(Phase::Unpacking);
    let limits = limits.at_most(Limits::for_archive(manifest.size));
    let assembly = Assembly {
        entries: &plan.index.entries,
        reuse: &plan.reuse,
    };
    extract::extract_assembled(
        &path,
        &staging.join(UNPACKED),
        limits,
        Some(cancelled),
        &assembly,
    )
    .map_err(|e| {
        if e.kind() == io::ErrorKind::Interrupted {
            DownloadError::Cancelled
        } else {
            off(e)
        }
    })?;
    fs::remove_file(&path)?;
    Ok(install::staged_from(staging, install)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::{Body, FetchError};
    use crate::index::index_name;
    use crate::key::to_hex;
    use crate::manifest::tests::sample;
    use crate::manifest::IndexRef;
    use sha2::{Digest, Sha256};
    use std::io::Write;
    use std::sync::Mutex;

    const ARCHIVE_URL: &str = "https://example.com/g/g-1.1.0-windows-x64.zip";
    const INDEX_URL: &str = "https://example.com/g/g-1.1.0-windows-x64.index.json";
    const BIG: usize = 200_000;

    /// Bytes that do not compress, so each big file is big in the zip.
    fn noise(seed: u64, len: usize) -> Vec<u8> {
        let mut x = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect()
    }

    /// How a fake server answers ranges.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Mode {
        Honest,
        /// Answers 200 with the whole file.
        IgnoresRanges,
        /// Answers 206 for another range.
        WrongRange,
        /// Sends one byte too few.
        Short,
        /// Sends one byte too many.
        Long,
        /// Declares another length.
        WrongLength,
        /// Changes a byte of a downloaded file's compressed bytes.
        Tampers,
        /// Refuses ranges with an error, as a broken redirect would.
        Fails,
    }

    struct Server {
        archive: Vec<u8>,
        index: Vec<u8>,
        mode: Mode,
        ranges: bool,
        tamper_at: u64,
        range_requests: Mutex<Vec<Range<u64>>>,
        whole_downloads: Mutex<u32>,
    }

    impl Server {
        fn new(archive: Vec<u8>, index: Vec<u8>, mode: Mode) -> Server {
            Server {
                archive,
                index,
                mode,
                ranges: true,
                tamper_at: 0,
                range_requests: Mutex::default(),
                whole_downloads: Mutex::default(),
            }
        }
    }

    impl Fetch for Server {
        fn get(&self, url: &str, _: &[(&str, &str)], limit: u64) -> Result<Vec<u8>, FetchError> {
            let body = match url {
                INDEX_URL => self.index.clone(),
                _ => return Err(FetchError::Status(404)),
            };
            if body.len() as u64 > limit {
                return Err(FetchError::TooLarge);
            }
            Ok(body)
        }

        fn open(&self, url: &str) -> Result<Body, FetchError> {
            assert_eq!(url, ARCHIVE_URL);
            *self.whole_downloads.lock().unwrap() += 1;
            Ok(Body {
                length: Some(self.archive.len() as u64),
                reader: Box::new(io::Cursor::new(self.archive.clone())),
            })
        }

        fn supports_ranges(&self) -> bool {
            self.ranges
        }

        fn open_range(
            &self,
            url: &str,
            range: Range<u64>,
        ) -> Result<Option<RangeBody>, FetchError> {
            assert_eq!(url, ARCHIVE_URL);
            self.range_requests.lock().unwrap().push(range.clone());
            let total = self.archive.len();
            let (s, e) = (range.start as usize, range.end as usize);
            let mut body = self.archive[s..e].to_vec();
            let mut status = 206;
            let mut content_range = format!("bytes {s}-{}/{total}", e - 1);
            let mut length = Some(body.len() as u64);
            match self.mode {
                Mode::Honest => {}
                Mode::IgnoresRanges => {
                    status = 200;
                    body = self.archive.clone();
                    length = Some(total as u64);
                }
                Mode::WrongRange => content_range = format!("bytes 0-{}/{total}", e - s - 1),
                Mode::Short => {
                    body.pop();
                    length = None;
                }
                Mode::Long => {
                    body.push(0);
                    length = None;
                }
                Mode::WrongLength => length = Some(body.len() as u64 + 1),
                // Inside a downloaded file's compressed bytes, not the
                // central directory, which has its own hash: the entry's
                // own checks must catch it.
                Mode::Tampers => {
                    if (s..e).contains(&(self.tamper_at as usize)) {
                        body[self.tamper_at as usize - s] ^= 0xff;
                    }
                }
                Mode::Fails => return Err(FetchError::Status(416)),
            }
            Ok(Some(RangeBody {
                status,
                content_range: Some(content_range),
                length,
                reader: Box::new(io::Cursor::new(body)),
            }))
        }
    }

    /// A release, the copy a player has of the one before, and the update
    /// for it.
    struct Fixture {
        dir: tempfile::TempDir,
        archive: PathBuf,
        install: Install,
        update: Update,
        index: Vec<u8>,
    }

    fn thresholds() -> Thresholds {
        Thresholds {
            min_reuse: 64 * 1024,
            min_saving: 1000,
            merge_gap: 1024,
            max_ranges: 100,
        }
    }

    /// The new release has a changed program, one big file the player
    /// already has, one big file that changed, a small file, a folder and
    /// macOS junk.
    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let name = "g-1.1.0-windows-x64.zip";
        let archive = dir.path().join(name);
        let file = fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let exe = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
        let data = zip::write::SimpleFileOptions::default().unix_permissions(0o644);
        let stored = data.compression_method(zip::CompressionMethod::Stored);
        let add = |zip: &mut zip::ZipWriter<fs::File>, name: &str, o, body: &[u8]| {
            zip.start_file(name, o).unwrap();
            zip.write_all(body).unwrap();
        };
        zip.add_directory("g/", data).unwrap();
        add(&mut zip, "g/g.exe", exe, b"MZ new program");
        add(&mut zip, "g/data/same.bin", stored, &noise(1, BIG));
        add(&mut zip, "g/data/changed.bin", data, &noise(3, BIG));
        add(&mut zip, "g/data/notes.txt", data, b"small");
        zip.add_directory("g/empty/", data).unwrap();
        add(&mut zip, "g/data/._same.bin", data, b"junk");
        zip.finish().unwrap();

        let install_dir = dir.path().join("game");
        fs::create_dir_all(install_dir.join("data")).unwrap();
        fs::write(install_dir.join("g.exe"), b"MZ old program").unwrap();
        fs::write(install_dir.join("data/same.bin"), noise(1, BIG)).unwrap();
        fs::write(install_dir.join("data/changed.bin"), noise(2, BIG)).unwrap();
        let install = Install::Folder {
            dir: install_dir,
            exe: "g.exe".into(),
        };

        let built = crate::index::build(&archive).unwrap();
        let index = built.to_json().into_bytes();
        let bytes = fs::read(&archive).unwrap();
        let mut manifest = sample();
        manifest.platform = "windows-x64".into();
        manifest.archive = name.into();
        manifest.size = bytes.len() as u64;
        manifest.sha256 = to_hex(&Sha256::digest(&bytes));
        manifest.index = Some(IndexRef {
            file: index_name(name).unwrap(),
            size: index.len() as u64,
            sha256: to_hex(&Sha256::digest(&index)),
        });
        let update = Update {
            manifest,
            archive_url: ARCHIVE_URL.into(),
            index_url: Some(INDEX_URL.into()),
            plan: None,
        };
        Fixture {
            dir,
            archive,
            install,
            update,
            index,
        }
    }

    impl Fixture {
        fn server(&self, mode: Mode) -> Server {
            let mut server =
                Server::new(fs::read(&self.archive).unwrap(), self.index.clone(), mode);
            let index = Index::parse(&self.index).unwrap();
            let changed = index
                .entries
                .iter()
                .find(|e| e.path == "g/data/changed.bin")
                .unwrap();
            server.tamper_at = (changed.data + changed.end) / 2;
            server
        }

        fn planned(&self, server: &Server) -> Update {
            let mut update = self.update.clone();
            update.plan = plan_with(&update, server, &self.install, thresholds());
            assert!(update.plan.is_some());
            update
        }

        /// What a whole download of the release unpacks to.
        fn full(&self) -> BTreeMap<PathBuf, (String, Vec<u8>, u32)> {
            let staging = self.dir.path().join("full");
            fs::create_dir_all(&staging).unwrap();
            let copy = staging.join(self.archive.file_name().unwrap());
            fs::copy(&self.archive, &copy).unwrap();
            let staged =
                install::stage(&copy, &staging, &self.install, Limits::default(), None).unwrap();
            tree(staged.path())
        }

        fn stage_with(&self, update: &Update, server: &Server) -> Result<Staged, DownloadError> {
            let staging = install::make_staging_dir_in(self.dir.path(), "g").unwrap();
            download_and_stage_with(
                update,
                server,
                &staging,
                &self.install,
                Limits::default(),
                &mut |_, _| {},
                &mut |_| {},
                &AtomicBool::new(false),
                Duration::from_secs(5),
            )
        }
    }

    /// Every file, folder and link under `root`: its kind, contents (a
    /// link's target) and mode.
    fn tree(root: &Path) -> BTreeMap<PathBuf, (String, Vec<u8>, u32)> {
        let mut out = BTreeMap::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                let meta = fs::symlink_metadata(&path).unwrap();
                #[cfg(unix)]
                let mode = std::os::unix::fs::PermissionsExt::mode(&meta.permissions());
                #[cfg(not(unix))]
                let mode = 0;
                let rel = path.strip_prefix(root).unwrap().to_path_buf();
                let (kind, body) = if meta.file_type().is_symlink() {
                    let target = fs::read_link(&path).unwrap();
                    ("link", target.to_string_lossy().as_bytes().to_vec())
                } else if meta.is_dir() {
                    stack.push(path.clone());
                    ("dir", Vec::new())
                } else {
                    ("file", fs::read(&path).unwrap())
                };
                out.insert(rel, (kind.to_string(), body, mode));
            }
        }
        out
    }

    #[test]
    fn only_big_files_the_player_has_byte_for_byte_are_kept() {
        let f = fixture();
        let server = f.server(Mode::Honest);
        let plan = plan_with(&f.update, &server, &f.install, thresholds()).unwrap();
        let kept: Vec<&str> = plan
            .reuse
            .keys()
            .map(|&i| plan.index.entries[i].path.as_str())
            .collect();
        assert_eq!(kept, ["g/data/same.bin"]);
        let Install::Folder { dir, .. } = &f.install else {
            unreachable!()
        };
        assert_eq!(
            plan.reuse.values().next().unwrap(),
            &dir.join("data/same.bin")
        );
        // It downloads less than the archive, by about the kept file.
        let size = f.update.manifest.size;
        assert!(plan.download_size() < size - BIG as u64 + 1000, "{plan:?}");
        // The parts do not overlap, are in order, and include the central
        // directory to the end of the archive.
        for pair in plan.ranges.windows(2) {
            assert!(pair[0].end < pair[1].start);
        }
        assert_eq!(plan.ranges.last().unwrap().end, size);
    }

    #[test]
    fn a_file_of_the_same_size_but_other_bytes_is_downloaded() {
        let f = fixture();
        let Install::Folder { dir, .. } = &f.install else {
            unreachable!()
        };
        // Same size, one byte different: the CRC-32 is never asked.
        let mut almost = noise(1, BIG);
        almost[100] ^= 1;
        fs::write(dir.join("data/same.bin"), almost).unwrap();
        let server = f.server(Mode::Honest);
        assert_eq!(
            plan_with(&f.update, &server, &f.install, thresholds()),
            None
        );
    }

    #[test]
    fn small_files_and_small_savings_are_not_worth_a_plan() {
        let f = fixture();
        let server = f.server(Mode::Honest);
        let picky = Thresholds {
            min_reuse: BIG as u64 + 1,
            ..thresholds()
        };
        assert_eq!(plan_with(&f.update, &server, &f.install, picky), None);
        let greedy = Thresholds {
            min_saving: BIG as u64 * 2,
            ..thresholds()
        };
        assert_eq!(plan_with(&f.update, &server, &f.install, greedy), None);
        let few = Thresholds {
            max_ranges: 1,
            ..thresholds()
        };
        assert_eq!(plan_with(&f.update, &server, &f.install, few), None);
    }

    #[test]
    fn no_plan_without_a_good_signed_index() {
        let f = fixture();
        // No index in the manifest, or nowhere to fetch it.
        let server = f.server(Mode::Honest);
        let mut bare = f.update.clone();
        bare.manifest.index = None;
        assert_eq!(plan_with(&bare, &server, &f.install, thresholds()), None);
        let mut nowhere = f.update.clone();
        nowhere.index_url = None;
        assert_eq!(plan_with(&nowhere, &server, &f.install, thresholds()), None);
        // An index other than the signed one.
        let mut changed = f.server(Mode::Honest);
        changed.index = String::from_utf8(f.index.clone())
            .unwrap()
            .replacen("\"size\":200000", "\"size\":200001", 1)
            .into_bytes();
        changed.index.truncate(f.index.len());
        assert_eq!(
            plan_with(&f.update, &changed, &f.install, thresholds()),
            None
        );
        // A signed index for another archive.
        let mut other = f.update.clone();
        other.manifest.sha256 = "ab".repeat(32);
        assert_eq!(plan_with(&other, &server, &f.install, thresholds()), None);
        // A Fetch that does not ask for ranges.
        let mut plain = f.server(Mode::Honest);
        plain.ranges = false;
        assert_eq!(plan_with(&f.update, &plain, &f.install, thresholds()), None);
        // A tar.gz is never read in parts.
        let mut tar = f.update.clone();
        tar.manifest.archive = "g-1.1.0-linux-x64.tar.gz".into();
        assert_eq!(plan_with(&tar, &server, &f.install, thresholds()), None);
        // An AppImage install has no files to keep.
        let image = Install::AppImage {
            file: f.dir.path().join("g.AppImage"),
        };
        assert_eq!(plan_with(&f.update, &server, &image, thresholds()), None);
    }

    #[test]
    fn a_partial_download_unpacks_exactly_what_a_whole_one_does() {
        let f = fixture();
        let server = f.server(Mode::Honest);
        let update = f.planned(&server);
        let plan = update.plan.clone().unwrap();
        let mut heard = Vec::new();
        let mut phases = Vec::new();
        let staging = install::make_staging_dir_in(f.dir.path(), "g").unwrap();
        let staged = download_and_stage_with(
            &update,
            &server,
            &staging,
            &f.install,
            Limits::default(),
            &mut |done, total| heard.push((done, total)),
            &mut |p| phases.push(p),
            &AtomicBool::new(false),
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(*server.whole_downloads.lock().unwrap(), 0);
        assert_eq!(*server.range_requests.lock().unwrap(), plan.ranges);
        assert_eq!(tree(staged.path()), f.full());
        assert_eq!(
            heard.last(),
            Some(&(plan.download_size(), plan.download_size()))
        );
        assert_eq!(phases, [Phase::Downloading, Phase::Unpacking]);
        assert!(!staging.join(&update.manifest.archive).exists());
        assert_eq!(staged, Staged::Folder(staging.join("unpacked/g")));
    }

    #[test]
    fn anything_off_downloads_the_whole_archive() {
        let f = fixture();
        let full = f.full();
        for mode in [
            Mode::IgnoresRanges,
            Mode::WrongRange,
            Mode::Short,
            Mode::Long,
            Mode::WrongLength,
            Mode::Tampers,
            Mode::Fails,
        ] {
            let server = f.server(mode);
            let update = f.planned(&server);
            let staged = f.stage_with(&update, &server).unwrap();
            assert_eq!(*server.whole_downloads.lock().unwrap(), 1, "{mode:?}");
            assert_eq!(tree(staged.path()), full, "{mode:?}");
        }
    }

    #[test]
    fn a_cancel_stops_the_plan() {
        let f = fixture();
        let server = f.server(Mode::Honest);
        let go = AtomicBool::new(false);
        assert!(plan_inner(&f.update, &server, &f.install, thresholds(), Some(&go)).is_some());
        let stop = AtomicBool::new(true);
        assert_eq!(
            plan_inner(&f.update, &server, &f.install, thresholds(), Some(&stop)),
            None
        );
    }

    #[test]
    fn a_kept_file_that_changed_or_went_is_downloaded_after_all() {
        let f = fixture();
        let full = f.full();
        let Install::Folder { dir, .. } = &f.install else {
            unreachable!()
        };
        let server = f.server(Mode::Honest);
        let update = f.planned(&server);
        // Changed after the check, same size.
        fs::write(dir.join("data/same.bin"), noise(9, BIG)).unwrap();
        let staged = f.stage_with(&update, &server).unwrap();
        assert_eq!(*server.whole_downloads.lock().unwrap(), 1);
        assert_eq!(tree(staged.path()), full);
        // Gone.
        let server = f.server(Mode::Honest);
        fs::remove_file(dir.join("data/same.bin")).unwrap();
        let staged = f.stage_with(&update, &server).unwrap();
        assert_eq!(*server.whole_downloads.lock().unwrap(), 1);
        assert_eq!(tree(staged.path()), full);
    }

    #[cfg(unix)]
    #[test]
    fn a_kept_file_turned_into_a_link_is_not_followed() {
        let f = fixture();
        let Install::Folder { dir, .. } = &f.install else {
            unreachable!()
        };
        let server = f.server(Mode::Honest);
        let update = f.planned(&server);
        let elsewhere = f.dir.path().join("elsewhere.bin");
        fs::rename(dir.join("data/same.bin"), &elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, dir.join("data/same.bin")).unwrap();
        let staged = f.stage_with(&update, &server).unwrap();
        assert_eq!(*server.whole_downloads.lock().unwrap(), 1);
        assert_eq!(tree(staged.path()), f.full());
    }

    /// A plan that names the wrong file to keep (another entry's bytes)
    /// is caught by the signed hash, never shipped.
    #[test]
    fn a_kept_file_must_be_the_signed_one_once_copied() {
        let f = fixture();
        let server = f.server(Mode::Honest);
        let mut update = f.planned(&server);
        let Install::Folder { dir, .. } = &f.install else {
            unreachable!()
        };
        let plan = update.plan.as_mut().unwrap();
        let kept = *plan.reuse.keys().next().unwrap();
        plan.reuse.insert(kept, dir.join("data/changed.bin"));
        let staged = f.stage_with(&update, &server).unwrap();
        assert_eq!(*server.whole_downloads.lock().unwrap(), 1);
        assert_eq!(tree(staged.path()), f.full());
    }

    #[test]
    fn a_cancel_stops_without_a_whole_download() {
        let f = fixture();
        let server = f.server(Mode::Honest);
        let update = f.planned(&server);
        let staging = install::make_staging_dir_in(f.dir.path(), "g").unwrap();
        let err = download_and_stage_with(
            &update,
            &server,
            &staging,
            &f.install,
            Limits::default(),
            &mut |_, _| {},
            &mut |_| {},
            &AtomicBool::new(true),
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(matches!(err, DownloadError::Cancelled), "{err:?}");
        assert_eq!(*server.whole_downloads.lock().unwrap(), 0);
    }

    #[test]
    fn without_a_plan_it_is_a_whole_download() {
        let f = fixture();
        let server = f.server(Mode::Honest);
        let staged = f.stage_with(&f.update, &server).unwrap();
        assert!(server.range_requests.lock().unwrap().is_empty());
        assert_eq!(*server.whole_downloads.lock().unwrap(), 1);
        assert_eq!(tree(staged.path()), f.full());
    }

    #[test]
    fn range_answers_are_checked_to_the_byte() {
        let body = |status, range: Option<&str>, length| RangeBody {
            status,
            content_range: range.map(str::to_string),
            length,
            reader: Box::new(io::empty()),
        };
        let r = 100..200;
        assert!(
            range_answer_ok(&body(206, Some("bytes 100-199/5000"), Some(100)), &r, 5000).is_ok()
        );
        assert!(range_answer_ok(&body(206, Some("bytes 100-199/5000"), None), &r, 5000).is_ok());
        for bad in [
            body(200, Some("bytes 100-199/5000"), Some(100)),
            body(206, None, Some(100)),
            body(206, Some("bytes 100-199/*"), Some(100)),
            body(206, Some("bytes 100-199/4999"), Some(100)),
            body(206, Some("bytes 100-200/5000"), Some(101)),
            body(206, Some("bytes 0-99/5000"), Some(100)),
            body(206, Some("bytes 100-199/5000"), Some(99)),
            body(206, Some("items 100-199/5000"), Some(100)),
        ] {
            assert!(
                range_answer_ok(&bad, &r, 5000).is_err(),
                "{:?}",
                bad.content_range
            );
        }
    }

    fn index_of(paths: &[&str]) -> Index {
        Index {
            format: crate::index::INDEX_FORMAT,
            archive_size: 1000,
            archive_sha256: "ab".repeat(32),
            central_start: 900,
            central_sha256: "cd".repeat(32),
            entries: paths
                .iter()
                .map(|p| crate::index::IndexEntry {
                    path: p.to_string(),
                    size: 1,
                    sha256: "ef".repeat(32),
                    header: 0,
                    data: 0,
                    end: 0,
                })
                .collect(),
        }
    }

    #[test]
    fn files_are_found_where_staging_lays_them_out() {
        let folder = Install::Folder {
            dir: PathBuf::from("/games/g"),
            exe: "g".into(),
        };
        // One top folder: below it.
        let wrapped = index_of(&[
            "g-1.1.0/",
            "g-1.1.0/g",
            "g-1.1.0/data/a.ogg",
            "__MACOSX/._g",
        ]);
        assert_eq!(
            install_paths(&wrapped, &folder),
            [
                None,
                Some(PathBuf::from("/games/g/g")),
                Some(PathBuf::from("/games/g/data/a.ogg")),
                None
            ]
        );
        // No single top folder: the archive is the install.
        let flat = index_of(&["g", "data/a.ogg"]);
        assert_eq!(
            install_paths(&flat, &folder),
            [
                Some(PathBuf::from("/games/g/g")),
                Some(PathBuf::from("/games/g/data/a.ogg"))
            ]
        );
        // A Mac app: below the app, wherever it is in the archive.
        let app = Install::Bundle {
            app: PathBuf::from("/Applications/My Game.app"),
            exe: PathBuf::from("Contents/MacOS/g"),
        };
        let mac = index_of(&[
            "g-1.1.0-macos-arm64/My Game.app/Contents/MacOS/g",
            "g-1.1.0-macos-arm64/My Game.app/Contents/Resources/sounds.pak",
            "g-1.1.0-macos-arm64/README.md",
        ]);
        assert_eq!(
            install_paths(&mac, &app),
            [
                Some(PathBuf::from("/Applications/My Game.app/Contents/MacOS/g")),
                Some(PathBuf::from(
                    "/Applications/My Game.app/Contents/Resources/sounds.pak"
                )),
                None
            ]
        );
        // An app the player renamed is still the only app.
        let renamed = Install::Bundle {
            app: PathBuf::from("/Applications/Mine.app"),
            exe: PathBuf::from("Contents/MacOS/g"),
        };
        assert_eq!(
            install_paths(&mac, &renamed)[0],
            Some(PathBuf::from("/Applications/Mine.app/Contents/MacOS/g"))
        );
        // Names that reach outside are never looked for.
        let evil = index_of(&["g/../../etc/passwd", "g/ok"]);
        assert_eq!(
            install_paths(&evil, &folder),
            [None, Some(PathBuf::from("/games/g/ok"))]
        );
    }
}
