//! Round 2: how the partial download plans its requests, on a release
//! shaped like a real game with many loose sound files.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::ops::Range;

use hearken_update::fetch::{Body, Fetch, FetchError, RangeBody};
use hearken_update::index;
use hearken_update::manifest::{self, Manifest, FORMAT};
use hearken_update::{check, plan, IndexRef, Install, ManifestUrl, Running, SecretKey};

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

/// An honest host that serves the signed files and answers ranges exactly.
struct Host {
    files: BTreeMap<String, Vec<u8>>,
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
        Ok(Body {
            length: Some(body.len() as u64),
            reader: Box::new(io::Cursor::new(body)),
        })
    }

    fn supports_ranges(&self) -> bool {
        true
    }

    fn open_range(&self, url: &str, range: Range<u64>) -> Result<Option<RangeBody>, FetchError> {
        let body = self.get(url, &[], u64::MAX)?;
        let total = body.len();
        let part = body[range.start as usize..range.end as usize].to_vec();
        Ok(Some(RangeBody {
            status: 206,
            content_range: Some(format!("bytes {}-{}/{total}", range.start, range.end - 1)),
            length: Some(part.len() as u64),
            reader: Box::new(io::Cursor::new(part)),
        }))
    }
}

/// A game with 1,100 loose sounds of 100 KB each (about 110 MB) that the
/// player already has, byte for byte, and one changed 100 KB file. Every
/// kept file still costs a request of its own, for its ~100-byte local
/// header, because the unpack reads the header to find the data, so the
/// plan needs more than MAX_RANGES (1,000) requests and is dropped: the
/// player downloads all 110 MB for a 100 KB change.
///
/// Ignored because it fails on this branch (it would block the build).
#[test]
#[ignore = "fails: a game with over 1000 unchanged large files always downloads whole"]
fn many_unchanged_files_still_get_a_partial_download() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(NAME);
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    let stored =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let game = dir.path().join("game");
    fs::create_dir_all(game.join("sounds")).unwrap();
    fs::write(game.join("g.exe"), b"MZ old").unwrap();
    zip.start_file("g/g.exe", stored).unwrap();
    zip.write_all(b"MZ new").unwrap();
    for i in 0..1100u64 {
        let body = noise(i + 10, 100_000);
        zip.start_file(format!("g/sounds/s{i:04}.ogg"), stored)
            .unwrap();
        zip.write_all(&body).unwrap();
        fs::write(game.join(format!("sounds/s{i:04}.ogg")), &body).unwrap();
    }
    zip.start_file("g/sounds/changed.ogg", stored).unwrap();
    zip.write_all(&noise(1, 100_000)).unwrap();
    fs::write(game.join("sounds/changed.ogg"), noise(2, 100_000)).unwrap();
    zip.finish().unwrap();

    let archive = fs::read(&path).unwrap();
    let index_text = index::build(&path).unwrap().to_json();
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
    let mut files = BTreeMap::new();
    files.insert(
        "update-stable-windows-x64.json".to_string(),
        manifest::sign(&manifest, &secret).unwrap().into_bytes(),
    );
    files.insert(NAME.to_string(), archive.clone());
    files.insert(index_file, index_text.into_bytes());
    let host = Host { files };

    let source = ManifestUrl::new(&format!("{BASE}update-{{channel}}-{{platform}}.json")).unwrap();
    let running = Running {
        key: secret.public(),
        game: "g".into(),
        version: "1.0.0".into(),
        channel: "stable".into(),
        platform: "windows-x64".into(),
    };
    let update = check(&source, &host, &running, "stable").unwrap().unwrap();
    let install = Install::Folder {
        dir: game,
        exe: "g.exe".into(),
    };
    let planned = plan(&update, &host, &install);
    assert!(
        planned.is_some(),
        "1,100 of 1,101 files are already here, yet all {} bytes download",
        archive.len()
    );
}
