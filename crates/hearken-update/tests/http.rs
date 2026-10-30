//! A whole update over real HTTP, against a server on this computer: the
//! manifest is found on a plain host and on a GitHub-style API, checked,
//! the archive downloaded and staged.
#![cfg(feature = "http")]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use hearken_update::manifest::{self, Manifest, FORMAT};
use hearken_update::{
    check, download, stage, GitHubReleases, HttpFetch, Install, Limits, ManifestUrl, Running,
    SecretKey, Staged, UpdateSource,
};

type Files = Arc<Mutex<HashMap<String, Vec<u8>>>>;
/// Paths answered with a redirect to the address given.
type Redirects = Arc<Mutex<HashMap<String, String>>>;
/// Each request's path, and the range it asked for when it was answered as
/// one.
type Requests = Arc<Mutex<Vec<(String, Option<(u64, u64)>)>>>;

/// `bytes=first-last`.
fn parse_range(value: &str) -> Option<(u64, u64)> {
    let (first, last) = value.strip_prefix("bytes=")?.split_once('-')?;
    Some((first.parse().ok()?, last.parse().ok()?))
}

static REQUESTS: std::sync::OnceLock<Mutex<HashMap<String, Requests>>> = std::sync::OnceLock::new();

/// The requests the server at `base` was sent.
fn requests(base: &str) -> Requests {
    REQUESTS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap()
        .entry(base.to_string())
        .or_default()
        .clone()
}

/// Serves `files` by path until the test ends; anything else is a 404.
fn serve() -> (String, Files) {
    let (base, files, _) = serve_with_redirects();
    (base, files)
}

fn serve_with_redirects() -> (String, Files, Redirects) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let files: Files = Arc::default();
    let redirects: Redirects = Arc::default();
    let served = files.clone();
    let moved = redirects.clone();
    let log = requests(&base);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let files = served.clone();
            let redirects = moved.clone();
            let requests = log.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    return;
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                let mut range = None;
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = header.split_once(':') {
                        if name.eq_ignore_ascii_case("range") {
                            range = parse_range(value.trim());
                        }
                    }
                }
                if let Some(to) = redirects.lock().unwrap().get(&path).cloned() {
                    let head = format!(
                        "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    let _ = stream.write_all(head.as_bytes());
                    return;
                }
                let body = files.lock().unwrap().get(&path).cloned();
                // Paths starting /ranged/ answer ranges; the rest ignore
                // them, as many servers do.
                let ranged = path.starts_with("/ranged/");
                requests
                    .lock()
                    .unwrap()
                    .push((path.clone(), range.filter(|_| ranged)));
                let (status, body, extra) = match (body, range) {
                    (Some(body), Some((first, last))) if ranged => {
                        let total = body.len();
                        let part = body[first as usize..=last as usize].to_vec();
                        let extra = format!("Content-Range: bytes {first}-{last}/{total}\r\n");
                        ("206 Partial Content", part, extra)
                    }
                    (Some(body), _) => ("200 OK", body, String::new()),
                    (None, _) => ("404 Not Found", b"not found".to_vec(), String::new()),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            });
        }
    });
    (base, files, redirects)
}

fn tar_gz_with_program(path: &Path) -> Vec<u8> {
    let file = std::fs::File::create(path).unwrap();
    let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
        file,
        flate2::Compression::fast(),
    ));
    let mut header = tar::Header::new_gnu();
    header.set_size(9);
    header.set_mode(0o755);
    tar.append_data(
        &mut header,
        "my-game-1.1.0-linux-x64/my-game",
        &b"#!/bin/sh"[..],
    )
    .unwrap();
    tar.into_inner().unwrap().finish().unwrap();
    std::fs::read(path).unwrap()
}

fn manifest_for(archive: &[u8], name: &str) -> Manifest {
    Manifest {
        format: FORMAT,
        game: "my-game".into(),
        version: "1.1.0".into(),
        channel: "stable".into(),
        platform: "linux-x64".into(),
        archive: name.into(),
        url: None,
        size: archive.len() as u64,
        sha256: sha256_hex(archive),
        notes: "- Faster menus.".into(),
        published: None,
        index: None,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x");
    std::fs::write(&path, bytes).unwrap();
    hearken_update::download::sha256_file(&path).unwrap()
}

fn running(secret: &SecretKey) -> Running {
    Running {
        key: secret.public(),
        game: "my-game".into(),
        version: "1.0.0".into(),
        channel: "stable".into(),
        platform: "linux-x64".into(),
    }
}

fn update_through(source: &dyn UpdateSource, secret: &SecretKey, dir: &Path) -> Staged {
    let fetch = HttpFetch::default();
    let update = check(source, &fetch, &running(secret), "stable")
        .unwrap()
        .expect("an update");
    assert_eq!(update.manifest.note_lines(), ["Faster menus."]);
    let downloads = dir.join("downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    let archive = download(
        &update,
        &fetch,
        &downloads,
        &mut |_, _| {},
        &AtomicBool::new(false),
    )
    .unwrap();
    let install = Install::Folder {
        dir: dir.join("game"),
        exe: "my-game".into(),
    };
    stage(
        &archive,
        &dir.join("staging"),
        &install,
        Limits::default(),
        None,
    )
    .unwrap()
}

#[test]
fn an_update_from_any_https_host() {
    let (base, files) = serve();
    let dir = tempfile::tempdir().unwrap();
    let secret = SecretKey::generate().unwrap();
    let name = "my-game-1.1.0-linux-x64.tar.gz";
    let archive = tar_gz_with_program(&dir.path().join(name));
    let signed = manifest::sign(&manifest_for(&archive, name), &secret).unwrap();
    {
        let mut files = files.lock().unwrap();
        files.insert(
            "/games/my-game/update-stable-linux-x64.json".into(),
            signed.into_bytes(),
        );
        files.insert(format!("/games/my-game/{name}"), archive);
    }
    let source = ManifestUrl::new(&format!(
        "{base}/games/{{game}}/update-{{channel}}-{{platform}}.json"
    ))
    .unwrap();
    let staged = update_through(&source, &secret, dir.path());
    assert_eq!(
        std::fs::read(staged.path().join("my-game")).unwrap(),
        b"#!/bin/sh"
    );
    // Nothing published on another channel: up to date, not an error.
    let fetch = HttpFetch::default();
    assert_eq!(check(&source, &fetch, &running(&secret), "beta"), Ok(None));
}

#[test]
fn an_update_from_github_releases() {
    let (base, files) = serve();
    let dir = tempfile::tempdir().unwrap();
    let secret = SecretKey::generate().unwrap();
    let name = "my-game-1.1.0-linux-x64.tar.gz";
    let archive = tar_gz_with_program(&dir.path().join(name));
    let signed = manifest::sign(&manifest_for(&archive, name), &secret).unwrap();
    let download = |file: &str| format!("{base}/o/r/releases/download/v1.1.0/{file}");
    let releases = serde_json::json!([{
        "tag_name": "v1.1.0",
        "draft": false,
        "assets": [
            { "name": "update-stable-linux-x64.json",
              "browser_download_url": download("update-stable-linux-x64.json") },
            { "name": name, "browser_download_url": download(name) },
        ],
    }]);
    {
        let mut files = files.lock().unwrap();
        files.insert(
            "/api/repos/o/r/releases?per_page=100".into(),
            releases.to_string().into_bytes(),
        );
        files.insert(
            "/o/r/releases/download/v1.1.0/update-stable-linux-x64.json".into(),
            signed.into_bytes(),
        );
        files.insert(format!("/o/r/releases/download/v1.1.0/{name}"), archive);
    }
    let source = GitHubReleases::new("o", "r")
        .unwrap()
        .api(&format!("{base}/api"));
    let staged = update_through(&source, &secret, dir.path());
    assert!(staged.path().join("my-game").is_file());
}

#[test]
fn a_tampered_archive_is_never_unpacked() {
    let (base, files) = serve();
    let dir = tempfile::tempdir().unwrap();
    let secret = SecretKey::generate().unwrap();
    let name = "my-game-1.1.0-linux-x64.tar.gz";
    let archive = tar_gz_with_program(&dir.path().join(name));
    let signed = manifest::sign(&manifest_for(&archive, name), &secret).unwrap();
    let mut swapped = archive.clone();
    let last = swapped.len() - 1;
    swapped[last] ^= 1;
    {
        let mut files = files.lock().unwrap();
        files.insert(
            "/u/update-stable-linux-x64.json".into(),
            signed.into_bytes(),
        );
        files.insert(format!("/u/{name}"), swapped);
    }
    let source =
        ManifestUrl::new(&format!("{base}/u/update-{{channel}}-{{platform}}.json")).unwrap();
    let fetch = HttpFetch::default();
    let update = check(&source, &fetch, &running(&secret), "stable")
        .unwrap()
        .unwrap();
    let err = download(
        &update,
        &fetch,
        dir.path(),
        &mut |_, _| {},
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(matches!(err, hearken_update::DownloadError::Corrupt));
    assert!(!dir.path().join(name).exists());
}

#[test]
fn plain_http_elsewhere_is_never_fetched() {
    use hearken_update::{Fetch, FetchError};
    let fetch = HttpFetch::default();
    assert!(matches!(
        fetch.get("http://example.com/update.json", &[], 1000),
        Err(FetchError::NotAllowed(_))
    ));
    assert!(matches!(
        fetch.open("http://example.com/a.zip"),
        Err(FetchError::NotAllowed(_))
    ));
}

/// A redirect between allowed addresses is followed, a relative one too.
#[test]
fn redirects_to_allowed_addresses_are_followed() {
    use hearken_update::Fetch;
    let (base, files, redirects) = serve_with_redirects();
    files
        .lock()
        .unwrap()
        .insert("/real/a.json".into(), b"{}".to_vec());
    {
        let mut redirects = redirects.lock().unwrap();
        redirects.insert("/moved/a.json".into(), format!("{base}/hop/a.json"));
        redirects.insert("/hop/a.json".into(), "/real/a.json".into());
    }
    let fetch = HttpFetch::default();
    assert_eq!(
        fetch.get(&format!("{base}/moved/a.json"), &[], 100),
        Ok(b"{}".to_vec())
    );
    let mut body = fetch.open(&format!("{base}/moved/a.json")).unwrap();
    let mut read = Vec::new();
    std::io::Read::read_to_end(&mut body.reader, &mut read).unwrap();
    assert_eq!(read, b"{}");
    // A loop ends.
    redirects
        .lock()
        .unwrap()
        .insert("/loop".into(), format!("{base}/loop"));
    assert!(fetch.get(&format!("{base}/loop"), &[], 100).is_err());
}

/// An update host that redirects the manifest or the archive to plain HTTP
/// on another computer must not get the game to send anything there: the
/// address is refused before the request, not after. 127.0.0.2 stands in
/// for a machine on the player's network; Linux answers on all of 127/8.
#[cfg(target_os = "linux")]
#[test]
fn a_redirect_to_plain_http_sends_nothing_there() {
    use hearken_update::{Fetch, FetchError};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let elsewhere = TcpListener::bind("127.0.0.2:0").unwrap();
    let leak = format!("http://{}/leak", elsewhere.local_addr().unwrap());
    let reached = Arc::new(AtomicUsize::new(0));
    {
        let reached = reached.clone();
        std::thread::spawn(move || {
            for stream in elsewhere.incoming() {
                if stream.is_ok() {
                    reached.fetch_add(1, Ordering::SeqCst);
                }
            }
        });
    }
    let (base, files, redirects) = serve_with_redirects();
    let dir = tempfile::tempdir().unwrap();
    let secret = SecretKey::generate().unwrap();
    let name = "my-game-1.1.0-linux-x64.tar.gz";
    let archive = tar_gz_with_program(&dir.path().join(name));
    let signed = manifest::sign(&manifest_for(&archive, name), &secret).unwrap();
    files.lock().unwrap().insert(
        "/u/update-stable-linux-x64.json".into(),
        signed.into_bytes(),
    );
    {
        let mut redirects = redirects.lock().unwrap();
        redirects.insert(format!("/u/{name}"), format!("{leak}?archive"));
        redirects.insert(
            "/m/update-stable-linux-x64.json".into(),
            format!("{leak}?manifest"),
        );
    }
    let fetch = HttpFetch::default();
    let source =
        ManifestUrl::new(&format!("{base}/u/update-{{channel}}-{{platform}}.json")).unwrap();
    let update = check(&source, &fetch, &running(&secret), "stable")
        .unwrap()
        .unwrap();
    let err = download(
        &update,
        &fetch,
        dir.path(),
        &mut |_, _| {},
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            hearken_update::DownloadError::Fetch(FetchError::NotAllowed(_))
        ),
        "{err:?}"
    );
    assert!(matches!(
        fetch.get(&format!("{base}/m/update-stable-linux-x64.json"), &[], 1000),
        Err(FetchError::NotAllowed(_))
    ));
    // The listener only ever counts; give a stray connection time to land.
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert_eq!(
        reached.load(Ordering::SeqCst),
        0,
        "a request reached {leak}"
    );
}

/// A zip release with a file index, over real HTTP: a server that answers
/// ranges sends only what the player lacks; one that ignores them sends
/// the whole archive; both leave the same update staged.
#[test]
fn only_the_changed_files_are_downloaded_over_http() {
    use hearken_update::{download_and_stage, plan, Phase};
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let secret = SecretKey::generate().unwrap();
    let name = "my-game-1.1.0-windows-x64.zip";
    let path = dir.path().join(name);
    // Two megabytes that do not compress, the same as the player's.
    let mut x: u32 = 1;
    let big: Vec<u8> = (0..2_000_000)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 24) as u8
        })
        .collect();
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("my-game-1.1.0-windows-x64/my-game.exe", options)
        .unwrap();
    zip.write_all(b"MZ new").unwrap();
    zip.start_file("my-game-1.1.0-windows-x64/sounds.pak", options)
        .unwrap();
    zip.write_all(&big).unwrap();
    zip.finish().unwrap();
    let archive = std::fs::read(&path).unwrap();
    let index = hearken_update::index::build(&path).unwrap().to_json();
    let index_name = hearken_update::index::index_name(name).unwrap();
    let mut manifest = manifest_for(&archive, name);
    manifest.platform = "windows-x64".into();
    manifest.index = Some(hearken_update::IndexRef {
        file: index_name.clone(),
        size: index.len() as u64,
        sha256: sha256_hex(index.as_bytes()),
    });
    let signed = manifest::sign(&manifest, &secret).unwrap();
    let game = dir.path().join("game");
    std::fs::create_dir_all(&game).unwrap();
    std::fs::write(game.join("my-game.exe"), b"MZ old").unwrap();
    std::fs::write(game.join("sounds.pak"), &big).unwrap();
    let install = Install::Folder {
        dir: game.clone(),
        exe: "my-game.exe".into(),
    };
    let mut running = running(&secret);
    running.platform = "windows-x64".into();
    for folder in ["ranged", "whole"] {
        let (base, files) = serve();
        {
            let mut files = files.lock().unwrap();
            files.insert(
                format!("/{folder}/update-stable-windows-x64.json"),
                signed.clone().into_bytes(),
            );
            files.insert(format!("/{folder}/{name}"), archive.clone());
            files.insert(
                format!("/{folder}/{index_name}"),
                index.clone().into_bytes(),
            );
        }
        let source = ManifestUrl::new(&format!(
            "{base}/{folder}/update-{{channel}}-{{platform}}.json"
        ))
        .unwrap();
        let fetch = HttpFetch::default();
        let mut update = check(&source, &fetch, &running, "stable").unwrap().unwrap();
        update.plan = plan(&update, &fetch, &install);
        assert!(update.plan.is_some(), "{folder}");
        assert!(
            update.download_size() < 100_000,
            "{}",
            update.download_size()
        );
        let staging = dir.path().join(format!("staging-{folder}"));
        std::fs::create_dir_all(&staging).unwrap();
        let mut phases = Vec::new();
        let staged = download_and_stage(
            &update,
            &fetch,
            &staging,
            &install,
            Limits::default(),
            &mut |_, _| {},
            &mut |p| phases.push(p),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            std::fs::read(staged.path().join("my-game.exe")).unwrap(),
            b"MZ new"
        );
        assert_eq!(
            std::fs::read(staged.path().join("sounds.pak")).unwrap(),
            big
        );
        let log = requests(&base).lock().unwrap().clone();
        let archive_path = format!("/{folder}/{name}");
        let asked: Vec<_> = log.iter().filter(|(p, _)| *p == archive_path).collect();
        if folder == "ranged" {
            assert!(
                !asked.is_empty() && asked.iter().all(|(_, r)| r.is_some()),
                "{log:?}"
            );
            assert_eq!(phases, [Phase::Downloading, Phase::Unpacking]);
        } else {
            // The range was ignored: the whole archive, once.
            assert_eq!(
                asked.iter().filter(|(_, r)| r.is_none()).count(),
                2,
                "{log:?}"
            );
            assert_eq!(
                phases,
                [Phase::Downloading, Phase::Downloading, Phase::Unpacking]
            );
        }
    }
}
