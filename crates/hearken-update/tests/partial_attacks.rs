//! Attacks on the partial download by a host, mirror or anything between
//! them that has no signing key: it can change any byte of a range answer,
//! and nothing else. Each test here is a bug: it fails until it is fixed.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use hearken_update::fetch::{Body, Fetch, FetchError, RangeBody};
use hearken_update::index::{self, Index};
use hearken_update::manifest::{self, Manifest, FORMAT};
use hearken_update::partial::{download_and_stage_with, plan};
use hearken_update::{check, IndexRef, Install, Limits, ManifestUrl, Running, SecretKey};

const BASE: &str = "https://example.com/g/";
const NAME: &str = "g-1.1.0-windows-x64.zip";

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

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The host: the signed files as published, and range answers that go
/// through `tamper` first. A whole download is never tampered with.
struct Host {
    files: BTreeMap<String, Vec<u8>>,
    tampered_archive: Vec<u8>,
    whole_downloads: AtomicU64,
}

impl Fetch for Host {
    fn get(&self, url: &str, _: &[(&str, &str)], limit: u64) -> Result<Vec<u8>, FetchError> {
        let name = url.strip_prefix(BASE).ok_or(FetchError::Status(404))?;
        let body = self
            .files
            .get(name)
            .cloned()
            .ok_or(FetchError::Status(404))?;
        if body.len() as u64 > limit {
            return Err(FetchError::TooLarge);
        }
        Ok(body)
    }

    fn open(&self, url: &str) -> Result<Body, FetchError> {
        let body = self.get(url, &[], u64::MAX)?;
        self.whole_downloads.fetch_add(1, Ordering::SeqCst);
        Ok(Body {
            length: Some(body.len() as u64),
            reader: Box::new(io::Cursor::new(body)),
        })
    }

    fn supports_ranges(&self) -> bool {
        true
    }

    fn open_range(&self, url: &str, range: Range<u64>) -> Result<Option<RangeBody>, FetchError> {
        assert!(url.starts_with(BASE));
        let total = self.tampered_archive.len();
        let body = self.tampered_archive[range.start as usize..range.end as usize].to_vec();
        Ok(Some(RangeBody {
            status: 206,
            content_range: Some(format!("bytes {}-{}/{total}", range.start, range.end - 1)),
            length: Some(body.len() as u64),
            reader: Box::new(io::Cursor::new(body)),
        }))
    }
}

struct Release {
    dir: tempfile::TempDir,
    archive: Vec<u8>,
    index: Index,
    files: BTreeMap<String, Vec<u8>>,
    secret: SecretKey,
    install: Install,
}

/// A Windows-style zip release, as `hearken release` signs it, and the
/// player's copy of the one before: the same 2 MB sound pack, a changed
/// 300 KB file and a changed program.
fn release() -> Release {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(NAME);
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    let o = zip::write::SimpleFileOptions::default();
    zip.add_directory("g-1.1.0-windows-x64/", o).unwrap();
    for (name, body) in [
        ("g-1.1.0-windows-x64/g.exe", b"MZ new".to_vec()),
        ("g-1.1.0-windows-x64/sounds.pak", noise(1, 2_000_000)),
        ("g-1.1.0-windows-x64/data/changed.bin", noise(3, 300_000)),
    ] {
        zip.start_file(name, o).unwrap();
        zip.write_all(&body).unwrap();
    }
    zip.finish().unwrap();
    let archive = fs::read(&path).unwrap();
    // What `hearken release` does for a zip (release.rs, write_index).
    let built = index::build(&path).unwrap();
    let index_text = built.to_json();
    let index_file = index::index_name(NAME).unwrap();
    let manifest = Manifest {
        format: FORMAT,
        game: "g".into(),
        version: "1.1.0".into(),
        channel: "stable".into(),
        platform: "windows-x64".into(),
        archive: NAME.into(),
        url: None,
        size: archive.len() as u64,
        sha256: sha256_hex(&archive),
        notes: String::new(),
        published: None,
        index: Some(IndexRef {
            file: index_file.clone(),
            size: index_text.len() as u64,
            sha256: sha256_hex(index_text.as_bytes()),
        }),
    };
    let secret = SecretKey::generate().unwrap();
    let signed = manifest::sign(&manifest, &secret).unwrap();
    let mut files = BTreeMap::new();
    files.insert("update-stable-windows-x64.json".into(), signed.into_bytes());
    files.insert(NAME.to_string(), archive.clone());
    files.insert(index_file, index_text.into_bytes());

    let game = dir.path().join("game");
    fs::create_dir_all(game.join("data")).unwrap();
    fs::write(game.join("g.exe"), b"MZ old").unwrap();
    fs::write(game.join("sounds.pak"), noise(1, 2_000_000)).unwrap();
    fs::write(game.join("data/changed.bin"), noise(2, 300_000)).unwrap();
    Release {
        dir,
        archive,
        index: built,
        files,
        secret,
        install: Install::Folder {
            dir: game,
            exe: "g.exe".into(),
        },
    }
}

/// The largest any file under `dir` got, polled until `stop`.
fn watch_largest(dir: PathBuf, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<u64> {
    std::thread::spawn(move || {
        let mut largest = 0;
        while !stop.load(Ordering::SeqCst) {
            let mut stack = vec![dir.clone()];
            while let Some(at) = stack.pop() {
                for entry in fs::read_dir(&at).into_iter().flatten().flatten() {
                    let Ok(meta) = entry.metadata() else { continue };
                    if meta.is_dir() {
                        stack.push(entry.path());
                    } else if entry.path().extension().is_some_and(|e| e == "bin") {
                        largest = largest.max(meta.len());
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        largest
    })
}

fn running(secret: &SecretKey) -> Running {
    Running {
        key: secret.public(),
        game: "g".into(),
        version: "1.0.0".into(),
        channel: "stable".into(),
        platform: "windows-x64".into(),
    }
}

/// A downloaded entry's compressed bytes are not covered by any signature
/// until they are unpacked and hashed. A man in the middle swaps the
/// 300 KB changed file's deflate stream for one of zeros that inflates
/// without end. The unpack writes until the archive-wide limit (20 times
/// the archive, at least 32 MB; 20 GB for a 1 GB game) before anything
/// compares it with the signed 300 KB, so the player's disk fills with
/// what a whole download, checked against the signed SHA-256 first, would
/// never unpack. Each entry should be cut off at its signed size.
/// The changed file's entry, and a host whose range answers carry, in
/// place of its compressed bytes, a deflate stream of 100 MB of zeros.
fn bomb_host(release: &Release) -> (index::IndexEntry, Host) {
    let entry = release
        .index
        .entries
        .iter()
        .find(|e| e.path.ends_with("changed.bin"))
        .unwrap()
        .clone();
    let span = (entry.end - entry.data) as usize;
    let mut bomb = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    let zeros = vec![0u8; 1 << 20];
    for _ in 0..100 {
        bomb.write_all(&zeros).unwrap();
    }
    let mut bomb = bomb.finish().unwrap();
    assert!(bomb.len() < span, "{} > {span}", bomb.len());
    bomb.resize(span, 0);
    let mut tampered = release.archive.clone();
    tampered[entry.data as usize..entry.end as usize].copy_from_slice(&bomb);
    let host = Host {
        files: release.files.clone(),
        tampered_archive: tampered,
        whole_downloads: AtomicU64::new(0),
    };
    (entry, host)
}

/// The update the check finds, with its plan: the player has the pack.
fn planned(release: &Release, host: &Host) -> hearken_update::Update {
    let source = ManifestUrl::new(&format!("{BASE}update-{{channel}}-{{platform}}.json")).unwrap();
    let mut update = check(&source, host, &running(&release.secret), "stable")
        .unwrap()
        .unwrap();
    update.plan = plan(&update, host, &release.install);
    assert!(update.plan.is_some(), "the player has the sound pack");
    update
}

#[test]
fn a_tampered_range_inflates_far_past_its_signed_size() {
    let release = release();
    let (entry, host) = bomb_host(&release);
    let update = planned(&release, &host);

    let staging = release.dir.path().join("staging");
    fs::create_dir_all(&staging).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let watcher = watch_largest(staging.clone(), stop.clone());
    let staged = download_and_stage_with(
        &update,
        &host,
        &staging,
        &release.install,
        Limits::default(),
        &mut |_, _| {},
        &mut |_| {},
        &AtomicBool::new(false),
        Duration::from_secs(5),
    );
    stop.store(true, Ordering::SeqCst);
    let largest = watcher.join().unwrap();
    // It recovers: the whole archive is downloaded and checked.
    let staged = staged.unwrap();
    assert_eq!(host.whole_downloads.load(Ordering::SeqCst), 1);
    assert_eq!(
        fs::read(staged.path().join("data/changed.bin")).unwrap(),
        noise(3, 300_000)
    );
    // But before that, the unsigned bytes were unpacked far past the
    // signed size.
    assert!(
        largest <= entry.size,
        "{largest} bytes written for an entry signed at {} bytes",
        entry.size
    );
}

/// The same tampered entry, and the player presses Escape once 1 MB of it
/// is on disk. The unpack hears a cancel only between entries, so it goes
/// on writing the attacker's zeros to the archive-wide limit first (20 GB
/// for a 1 GB game) while the screen says it is cancelling.
#[test]
fn a_cancel_is_heard_inside_a_tampered_entry() {
    let release = release();
    let (_, host) = bomb_host(&release);
    let update = planned(&release, &host);
    let staging = release.dir.path().join("staging");
    fs::create_dir_all(&staging).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let watcher = {
        let (cancel, stop, staging) = (cancel.clone(), stop.clone(), staging.clone());
        std::thread::spawn(move || {
            let file = staging.join("unpacked/g-1.1.0-windows-x64/data/changed.bin");
            let mut largest = 0;
            while !stop.load(Ordering::SeqCst) {
                if let Ok(meta) = fs::metadata(&file) {
                    largest = largest.max(meta.len());
                    if meta.len() > 1_000_000 {
                        cancel.store(true, Ordering::SeqCst);
                    }
                }
            }
            largest
        })
    };
    let staged = download_and_stage_with(
        &update,
        &host,
        &staging,
        &release.install,
        Limits::default(),
        &mut |_, _| {},
        &mut |_| {},
        &cancel,
        Duration::from_secs(5),
    );
    stop.store(true, Ordering::SeqCst);
    let largest = watcher.join().unwrap();
    assert!(matches!(
        staged,
        Err(hearken_update::DownloadError::Cancelled)
    ));
    // Within a few chunks of the cancel, not at the limit.
    assert!(
        largest < 8_000_000,
        "{largest} bytes written after a cancel at 1 MB"
    );
}
