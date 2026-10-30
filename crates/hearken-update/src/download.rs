//! Fetching the archive: bounded by the signed size, checked against the
//! signed SHA-256 before anything unpacks it, and always possible to leave.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::check::Update;
use crate::fetch::{Fetch, FetchError};
use crate::key::to_hex;

/// How long a download may go without a byte before it fails.
///
/// An idle bound, never a total one: a big archive on a slow line cannot
/// fit any total deadline, and failing it would offer the same update
/// forever. Sixty seconds of nothing is a dead transfer on any line that
/// could finish one.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// How often the copy loop wakes to look for a cancel.
const CANCEL_POLL: Duration = Duration::from_millis(100);

/// Why a download stopped.
#[derive(Debug)]
pub enum DownloadError {
    /// The player backed out.
    Cancelled,
    /// Nothing arrived for this long.
    Stalled(Duration),
    /// The file is not the one the manifest signed: another size or hash.
    Corrupt,
    Fetch(FetchError),
    Io(io::Error),
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadError::Cancelled => f.write_str("the update was canceled"),
            DownloadError::Stalled(idle) => {
                write!(f, "no data arrived for {} seconds", idle.as_secs())
            }
            DownloadError::Corrupt => f.write_str("the download does not match the release"),
            DownloadError::Fetch(e) => e.fmt(f),
            DownloadError::Io(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for DownloadError {}

impl From<io::Error> for DownloadError {
    fn from(e: io::Error) -> Self {
        DownloadError::Io(e)
    }
}

impl From<FetchError> for DownloadError {
    fn from(e: FetchError) -> Self {
        DownloadError::Fetch(e)
    }
}

/// Downloads `update`'s archive into `dir`, under its signed name.
/// `progress(done, total)` hears each chunk. A file that is not exactly the
/// signed size and hash is deleted and reported as [`DownloadError::Corrupt`].
pub fn download(
    update: &Update,
    fetch: &dyn Fetch,
    dir: &Path,
    progress: &mut dyn FnMut(u64, u64),
    cancelled: &AtomicBool,
) -> Result<PathBuf, DownloadError> {
    download_with(update, fetch, dir, progress, cancelled, IDLE_TIMEOUT)
}

/// [`download`] with another idle bound, for tests.
pub fn download_with(
    update: &Update,
    fetch: &dyn Fetch,
    dir: &Path,
    progress: &mut dyn FnMut(u64, u64),
    cancelled: &AtomicBool,
    idle: Duration,
) -> Result<PathBuf, DownloadError> {
    let manifest = &update.manifest;
    let body = fetch.open(&update.archive_url)?;
    if body.length.is_some_and(|n| n != manifest.size) {
        log::warn!(
            "{} is {} bytes, the manifest signed {}",
            update.archive_url,
            body.length.unwrap_or(0),
            manifest.size
        );
        return Err(DownloadError::Corrupt);
    }
    // The name was checked when the manifest was opened: a plain file name.
    let dest = dir.join(&manifest.archive);
    let file = fs::File::create(&dest)?;
    let result = stream_to_file(
        body.reader,
        file,
        manifest.size,
        progress,
        Some(cancelled),
        idle,
    );
    let verified = match result {
        Ok((done, digest)) if done == manifest.size && digest == manifest.sha256 => Ok(dest),
        Ok((done, digest)) => {
            log::warn!(
                "{} arrived as {done} bytes with SHA-256 {digest}; the manifest signed {} bytes, {}",
                manifest.archive,
                manifest.size,
                manifest.sha256
            );
            Err(DownloadError::Corrupt)
        }
        Err(e) => Err(e),
    };
    if verified.is_err() {
        let _ = fs::remove_file(dir.join(&manifest.archive));
    }
    verified
}

/// Copies `reader` into `file`, at most `limit` bytes, bounded by an idle
/// timeout and a cancel flag. Returns the bytes written and their SHA-256.
/// More than `limit` bytes is [`DownloadError::Corrupt`].
///
/// The blocking reads run on their own thread and hand chunks over a small
/// channel, so this loop can always give up: a socket read cannot be
/// interrupted, but it no longer has to be waited for. A reader left blocked
/// on a dead connection ends when that read returns (its next send finds
/// nobody listening) or with the process.
pub fn stream_to_file<R: Read + Send + 'static>(
    mut reader: R,
    mut file: fs::File,
    limit: u64,
    progress: &mut dyn FnMut(u64, u64),
    cancelled: Option<&AtomicBool>,
    idle: Duration,
) -> Result<(u64, String), DownloadError> {
    use std::sync::mpsc::{sync_channel, RecvTimeoutError};

    let (tx, rx) = sync_channel::<io::Result<Vec<u8>>>(8);
    std::thread::Builder::new()
        .name("update-download-read".into())
        .spawn(move || loop {
            let mut buf = vec![0u8; 64 * 1024];
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    buf.truncate(n);
                    if tx.send(Ok(buf)).is_err() {
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    break;
                }
            }
        })?;

    let mut hasher = Sha256::new();
    let mut done: u64 = 0;
    let mut last_data = Instant::now();
    loop {
        if cancelled.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
            return Err(DownloadError::Cancelled);
        }
        match rx.recv_timeout(CANCEL_POLL) {
            Ok(Ok(chunk)) => {
                done += chunk.len() as u64;
                if done > limit {
                    log::warn!("the download ran past the {limit} bytes signed for it");
                    return Err(DownloadError::Corrupt);
                }
                file.write_all(&chunk)?;
                hasher.update(&chunk);
                last_data = Instant::now();
                progress(done, limit);
            }
            // The socket's own read timeout: the same stall, seen first.
            Ok(Err(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                return Err(DownloadError::Stalled(idle))
            }
            Ok(Err(e)) => return Err(e.into()),
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                if last_data.elapsed() >= idle {
                    log::warn!(
                        "update download stalled: nothing for {} s after {done} bytes",
                        idle.as_secs()
                    );
                    return Err(DownloadError::Stalled(idle));
                }
            }
        }
    }
    file.flush()?;
    file.sync_all()?;
    Ok((done, to_hex(&hasher.finalize())))
}

/// SHA-256 of a file, as 64 lowercase hex digits.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(to_hex(&hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::Body;
    use crate::manifest::tests::sample;
    use crate::source::tests::Table;

    fn update_for(bytes: &[u8]) -> Update {
        let mut manifest = sample();
        manifest.size = bytes.len() as u64;
        manifest.sha256 = to_hex(&Sha256::digest(bytes));
        Update {
            manifest,
            archive_url: "https://example.com/g/a.tar.gz".into(),
            index_url: None,
            plan: None,
        }
    }

    #[test]
    fn a_download_matching_its_manifest_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = vec![7u8; 200_000];
        let update = update_for(&bytes);
        let fetch = Table::default().with(&update.archive_url, bytes.clone());
        let mut seen = Vec::new();
        let path = download(
            &update,
            &fetch,
            dir.path(),
            &mut |done, total| seen.push((done, total)),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(seen.last(), Some(&(200_000, 200_000)));
        assert_eq!(sha256_file(&path).unwrap(), update.manifest.sha256);
    }

    #[test]
    fn a_changed_download_is_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let update = update_for(b"the real archive");
        let fetch = Table::default().with(&update.archive_url, b"the fake archive".to_vec());
        let err = download(
            &update,
            &fetch,
            dir.path(),
            &mut |_, _| {},
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(matches!(err, DownloadError::Corrupt));
        assert!(!dir.path().join(&update.manifest.archive).exists());
    }

    /// A server that declares no length cannot send more than was signed.
    struct Endless;

    impl Fetch for Endless {
        fn get(&self, _: &str, _: &[(&str, &str)], _: u64) -> Result<Vec<u8>, FetchError> {
            Err(FetchError::Status(404))
        }

        fn open(&self, _: &str) -> Result<Body, FetchError> {
            Ok(Body {
                length: None,
                reader: Box::new(io::repeat(b'x')),
            })
        }
    }

    #[test]
    fn a_download_stops_at_the_signed_size() {
        let dir = tempfile::tempdir().unwrap();
        let update = update_for(&[b'x'; 1000]);
        let mut wrong = update.clone();
        wrong.manifest.sha256 = "00".repeat(32);
        let err = download(
            &wrong,
            &Endless,
            dir.path(),
            &mut |_, _| {},
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(matches!(err, DownloadError::Corrupt));
        assert!(!dir.path().join(&update.manifest.archive).exists());
    }

    #[test]
    fn a_declared_length_other_than_the_signed_size_is_refused_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let update = update_for(b"short");
        let fetch = Table::default().with(&update.archive_url, b"longer than signed".to_vec());
        assert!(matches!(
            download(
                &update,
                &fetch,
                dir.path(),
                &mut |_, _| {},
                &AtomicBool::new(false)
            ),
            Err(DownloadError::Corrupt)
        ));
    }

    /// Never answers.
    struct Silent;

    impl Read for Silent {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            std::thread::sleep(Duration::from_secs(30));
            Ok(0)
        }
    }

    #[test]
    fn a_quiet_transfer_stalls_and_a_cancel_returns_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let file = fs::File::create(dir.path().join("a")).unwrap();
        let started = Instant::now();
        let err = stream_to_file(
            Silent,
            file,
            10,
            &mut |_, _| {},
            None,
            Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(matches!(err, DownloadError::Stalled(_)));
        assert!(started.elapsed() < Duration::from_secs(5));

        let file = fs::File::create(dir.path().join("b")).unwrap();
        let cancelled = AtomicBool::new(true);
        let started = Instant::now();
        let err = stream_to_file(
            Silent,
            file,
            10,
            &mut |_, _| {},
            Some(&cancelled),
            Duration::from_secs(60),
        )
        .unwrap_err();
        assert!(matches!(err, DownloadError::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
