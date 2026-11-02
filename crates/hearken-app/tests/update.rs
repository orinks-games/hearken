//! Checking for, offering and installing an update, against a host that
//! answers from a table and an apply step that only records the call.
#![cfg(feature = "update")]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hearken_app::screens::{SettingsScreen, UpdateCheckScreen};
use hearken_app::update::manifest::{self, Manifest, FORMAT};
use hearken_app::update::{Body, Fetch, FetchError, Host, Os, Running, SecretKey, Staged};
use hearken_app::{Game, Headless, Key, NoAction, NoCategory, UpdateConfig};

struct Patched;

struct Home;
impl hearken_app::State<Patched> for Home {
    fn name(&self) -> &'static str {
        "home"
    }
    fn allows_update_prompt(&self) -> bool {
        true
    }
    fn help(&self, _: &hearken_app::Ctx<Patched>) -> String {
        "Home".into()
    }
}

struct Playing;
impl hearken_app::State<Patched> for Playing {
    fn name(&self) -> &'static str {
        "playing"
    }
    fn help(&self, _: &hearken_app::Ctx<Patched>) -> String {
        "Playing".into()
    }
}

impl Game for Patched {
    const TITLE: &'static str = "Patched";
    const DATA_DIR: &'static str = "HearkenPatchedTest";
    const ENV_PREFIX: &'static str = "HEARKEN_PATCHED_TEST";
    type Event = ();
    type Data = ();
    type Settings = ();
    type Action = NoAction;
    type Category = NoCategory;

    fn updates() -> Option<UpdateConfig> {
        Some(
            UpdateConfig::manifest_url(
                "Patched",
                "https://example.test/patched/update-{channel}-{platform}.json",
            )?
            .channels(&["stable", "beta"])
            .manual("example.test slash patched"),
        )
    }
}

/// Answers GETs from a table, and keeps the URLs asked for. Says it asks
/// for ranges only when `ranges` is set.
#[derive(Default)]
struct Table(
    Mutex<HashMap<String, Vec<u8>>>,
    Mutex<Vec<String>>,
    AtomicBool,
);

impl Fetch for Table {
    fn get(&self, url: &str, _: &[(&str, &str)], limit: u64) -> Result<Vec<u8>, FetchError> {
        self.1.lock().unwrap().push(url.to_string());
        let body = self
            .0
            .lock()
            .unwrap()
            .get(url)
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
            reader: Box::new(std::io::Cursor::new(body)),
        })
    }

    fn supports_ranges(&self) -> bool {
        self.2.load(Ordering::SeqCst)
    }
}

const PLATFORM: &str = "linux-x64";
const EXE: &str = "patched";

struct World {
    h: Headless<Patched>,
    dir: tempfile::TempDir,
    secret: SecretKey,
    table: Arc<Table>,
    applied: Arc<Mutex<Vec<(Staged, PathBuf)>>>,
}

fn archive(dir: &Path, with_program: bool) -> (String, Vec<u8>) {
    let name = format!("patched-1.1.0-{PLATFORM}.tar.gz");
    let path = dir.join(&name);
    let file = std::fs::File::create(&path).unwrap();
    let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
        file,
        flate2::Compression::fast(),
    ));
    let mut header = tar::Header::new_gnu();
    header.set_size(2);
    header.set_mode(0o755);
    let inside = if with_program { EXE } else { "readme.txt" };
    tar.append_data(
        &mut header,
        format!("patched-1.1.0-{PLATFORM}/{inside}"),
        &b"hi"[..],
    )
    .unwrap();
    tar.into_inner().unwrap().finish().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    (name, bytes)
}

fn publish(world: &World, signer: &SecretKey, with_program: bool, version: &str) {
    let (name, bytes) = archive(world.dir.path(), with_program);
    let sums = world.dir.path().join("sum");
    std::fs::write(&sums, &bytes).unwrap();
    let manifest = Manifest {
        format: FORMAT,
        built: None,
        game: "patched".into(),
        version: version.into(),
        channel: "stable".into(),
        platform: PLATFORM.into(),
        archive: name.clone(),
        url: None,
        size: bytes.len() as u64,
        sha256: hearken_app::update::download::sha256_file(&sums).unwrap(),
        notes: "## Fixed\n- The map reads the right street.\n- Faster menus.".into(),
        published: None,
        index: None,
    };
    let signed = manifest::sign(&manifest, signer).unwrap();
    let mut table = world.table.0.lock().unwrap();
    table.insert(
        format!("https://example.test/patched/update-stable-{PLATFORM}.json"),
        signed.into_bytes(),
    );
    table.insert(format!("https://example.test/patched/{name}"), bytes);
}

/// A released copy of version 1.0.0, installed in a folder of its own, on
/// its Settings screen. (Its first screen enters before the test can make
/// it a released copy, so the check screen is pushed after.)
fn world() -> World {
    world_with_config(Patched::updates().unwrap())
}

fn world_with_config(config: UpdateConfig) -> World {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path().join("Patched");
    std::fs::create_dir_all(&install).unwrap();
    std::fs::write(install.join(EXE), b"old").unwrap();
    let secret = SecretKey::generate().unwrap();
    let table = Arc::new(Table::default());
    let applied: Arc<Mutex<Vec<(Staged, PathBuf)>>> = Arc::default();
    let mut h = Headless::<Patched>::new((), SettingsScreen::<Patched>::new());
    if config.prompt_at_start {
        h.app.ctx.reset_to(Home);
        h.frame(0.016);
    }
    h.app.ctx.updates = hearken_app::updates::Updates::new(Some(config));
    let updates = &mut h.app.ctx.updates;
    updates.set_fetch(table.clone());
    updates.set_running(Some(Running {
        key: secret.public(),
        game: "patched".into(),
        version: "1.0.0".into(),
        channel: "stable".into(),
        platform: PLATFORM.into(),
    }));
    updates.set_host(Host::fake(Os::Linux, &install.join(EXE)));
    updates.set_agnow(false);
    let record = applied.clone();
    updates.set_apply(Arc::new(move |staged, _install, staging, _host| {
        record
            .lock()
            .unwrap()
            .push((staged.clone(), staging.to_path_buf()));
        Ok(())
    }));
    World {
        h,
        dir,
        secret,
        table,
        applied,
    }
}

impl World {
    fn check(&mut self) {
        self.h.app.ctx.push(UpdateCheckScreen::new());
        self.h.frame(0.016);
    }
}

fn frames_until(h: &mut Headless<Patched>, line: &str) {
    for _ in 0..500 {
        h.frame(0.016);
        if h.spoken().iter().any(|l| l.contains(line)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("never heard {line:?}: {:?}", h.spoken());
}

#[test]
fn an_update_is_found_offered_downloaded_and_handed_to_the_swap() {
    let mut w = world();
    let secret = w.secret.clone();
    publish(&w, &secret, true, "1.1.0");
    w.check();
    frames_until(&mut w.h, "Update available");
    let offer = w.h.spoken().join(" ");
    assert!(
        offer.contains("Update available: Patched version 1.1.0, less than 1 megabyte."),
        "{offer}"
    );
    assert!(offer.contains("You have version 1.0.0."), "{offer}");
    assert!(offer.contains("Download and restart."), "{offer}");

    // What's new reads the notes as plain lines.
    w.h.press(Key::Down);
    w.h.press(Key::Return);
    w.h.clear();
    w.h.press(Key::Down);
    w.h.press(Key::Down);
    assert_eq!(w.h.spoken(), ["Fixed", "The map reads the right street."]);
    w.h.press(Key::Escape);
    w.h.press(Key::Home);
    w.h.clear();

    w.h.press(Key::Return);
    assert!(w.h.spoken()[0].starts_with("Downloading Patched version 1.1.0"));
    for _ in 0..500 {
        w.h.frame(0.016);
        if !w.applied.lock().unwrap().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let applied = w.applied.lock().unwrap().clone();
    let [(Staged::Folder(root), staging)] = applied.as_slice() else {
        panic!("not applied: {applied:?} {:?}", w.h.spoken());
    };
    assert_eq!(std::fs::read(root.join(EXE)).unwrap(), b"hi");
    assert!(root.starts_with(staging));
    assert!(w
        .h
        .spoken()
        .iter()
        .any(|l| l.contains("The game closes and starts again by itself")));
    assert!(!w.h.app.ctx.running, "the game quits so the swap can run");
    let _ = std::fs::remove_dir_all(staging);
}

#[test]
fn up_to_date_says_so_with_the_version() {
    let mut w = world();
    let secret = w.secret.clone();
    publish(&w, &secret, true, "1.0.0");
    w.check();
    frames_until(&mut w.h, "Up to date");
    assert!(w
        .h
        .spoken()
        .contains(&"Up to date. You have Patched version 1.0.0.".to_string()));
}

#[test]
fn an_update_signed_with_another_key_is_refused() {
    let mut w = world();
    let other = SecretKey::generate().unwrap();
    publish(&w, &other, true, "9.0.0");
    w.check();
    frames_until(&mut w.h, "did not sign");
    assert!(w.applied.lock().unwrap().is_empty());
}

#[test]
fn an_own_build_channel_ignores_a_package_built_for_another_channel() {
    // `publish` signs a manifest that does not say which channel its
    // package was built for, like one signed again for another channel.
    let mut w = world_with_config(Patched::updates().unwrap().own_build(&["stable"]));
    let secret = w.secret.clone();
    publish(&w, &secret, true, "9.0.0");
    w.check();
    frames_until(&mut w.h, "Up to date");
    assert!(w.applied.lock().unwrap().is_empty());
}

#[test]
fn a_developer_build_never_checks() {
    let mut w = world();
    let secret = w.secret.clone();
    publish(&w, &secret, true, "1.1.0");
    w.h.app.ctx.updates.set_running(None);
    w.check();
    for _ in 0..20 {
        w.h.frame(0.016);
    }
    assert_eq!(
        w.h.spoken().last().map(String::as_str),
        Some("This is a developer build, which never updates itself. Released copies do.")
    );
    assert!(!w.h.spoken().iter().any(|l| l.contains("Update available")));
}

#[test]
fn an_update_without_the_program_leaves_the_game_unchanged() {
    let mut w = world();
    let secret = w.secret.clone();
    publish(&w, &secret, false, "1.1.0");
    w.check();
    frames_until(&mut w.h, "Update available");
    w.h.press(Key::Return);
    frames_until(&mut w.h, "Your game is unchanged");
    assert!(w
        .h
        .spoken()
        .iter()
        .any(|l| l.contains("missing a file this game needs")
            && l.contains("download it from example.test slash patched")));
    assert!(w.applied.lock().unwrap().is_empty());
    assert!(w.h.app.ctx.running);
}

#[test]
fn skipping_a_version_stops_the_check_at_start_offering_it() {
    let mut w = world();
    let secret = w.secret.clone();
    publish(&w, &secret, true, "1.1.0");
    w.check();
    frames_until(&mut w.h, "Update available");
    w.h.press(Key::End);
    w.h.press(Key::Return);
    assert_eq!(
        w.h.app.ctx.settings.updates.skipped.as_deref(),
        Some("1.1.0")
    );
    // The check at start now stays quiet about 1.1.0.
    w.h.clear();
    let settings = w.h.app.ctx.settings.updates.clone();
    w.h.app.ctx.updates.check_at_start(&settings);
    for _ in 0..50 {
        w.h.frame(0.016);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(w.h.app.ctx.updates.available().is_none());
    assert!(!w.h.spoken().iter().any(|l| l.contains("is available")));
}

#[test]
fn the_check_at_start_announces_and_settings_offers_it() {
    let mut w = world();
    let secret = w.secret.clone();
    publish(&w, &secret, true, "1.1.0");
    let settings = w.h.app.ctx.settings.updates.clone();
    w.h.app.ctx.updates.check_at_start(&settings);
    frames_until(&mut w.h, "Patched version 1.1.0 is available.");
    assert!(w.h.app.ctx.updates.available().is_some());
    // The row says what it will install; the channel row comes after it.
    w.h.app.ctx.reset_to(SettingsScreen::<Patched>::new());
    w.h.frame(0.016);
    let lines = w.h.app.top_lines();
    assert!(
        lines.iter().any(|l| l.contains("Install version 1.1.0")),
        "{lines:?}"
    );
    assert!(lines.iter().any(|l| l.contains("Update channel: stable")));
    assert!(lines
        .iter()
        .any(|l| l.contains("Check for updates at start: on")));
}

#[test]
fn startup_can_offer_the_update_directly_and_later_dismisses_it_once() {
    let mut w = world_with_config(Patched::updates().unwrap().prompt_at_start());
    let secret = w.secret.clone();
    publish(&w, &secret, true, "1.1.0");
    let settings = w.h.app.ctx.settings.updates.clone();
    w.h.app.ctx.updates.check_at_start(&settings);
    frames_until(&mut w.h, "Update available");
    let lines = w.h.app.top_lines().join(" ");
    for choice in [
        "Download and restart",
        "What's new",
        "Remind me later",
        "Skip this version",
    ] {
        assert!(lines.contains(choice), "{lines}");
    }
    assert!(!w
        .h
        .spoken()
        .iter()
        .any(|s| s.contains("choose Check for updates in Settings")));
    assert!(w.h.spoken().last().unwrap().contains("Remind me later"));
    w.h.press(Key::Return);
    w.h.clear();
    for _ in 0..20 {
        w.h.frame(0.016);
    }
    assert!(!w.h.app.top_lines().join(" ").contains("Update available"));
    assert!(w.applied.lock().unwrap().is_empty());
}

#[test]
fn startup_prompt_respects_quiet_disabled_and_skipped_settings() {
    for mode in ["quiet", "disabled", "skipped"] {
        let mut config = Patched::updates().unwrap().prompt_at_start();
        if mode == "quiet" {
            config = config.quiet();
        }
        let mut w = world_with_config(config);
        let secret = w.secret.clone();
        publish(&w, &secret, true, "1.1.0");
        if mode == "disabled" {
            w.h.app.ctx.settings.updates.check_at_start = false;
        }
        if mode == "skipped" {
            w.h.app.ctx.settings.updates.skipped = Some("1.1.0".into());
        }
        let settings = w.h.app.ctx.settings.updates.clone();
        w.h.app.ctx.updates.check_at_start(&settings);
        w.h.clear();
        for _ in 0..50 {
            w.h.frame(0.016);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !w.h.spoken().iter().any(|s| s.contains("Update available")),
            "{mode}"
        );
    }
}

#[test]
fn startup_prompt_plans_the_download_like_a_manual_check() {
    let mut w = world_with_config(Patched::updates().unwrap().prompt_at_start());
    let index_url = publish_indexed(&w);
    let settings = w.h.app.ctx.settings.updates.clone();
    w.h.app.ctx.updates.check_at_start(&settings);
    frames_until(&mut w.h, "Update available");
    assert!(w.table.1.lock().unwrap().contains(&index_url));
    assert!(w.applied.lock().unwrap().is_empty());
}

#[test]
fn a_manual_check_takes_over_without_a_second_startup_prompt() {
    let mut w = world_with_config(Patched::updates().unwrap().prompt_at_start());
    let secret = w.secret.clone();
    publish(&w, &secret, true, "1.1.0");
    let settings = w.h.app.ctx.settings.updates.clone();
    w.h.app.ctx.updates.check_at_start(&settings);
    w.check();
    frames_until(&mut w.h, "Update available");
    w.h.press(Key::Escape);
    for _ in 0..50 {
        w.h.frame(0.016);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!w.h.app.top_lines().join(" ").contains("Update available"));
    let offers =
        w.h.spoken()
            .iter()
            .filter(|s| s.contains("Update available:"))
            .count();
    assert_eq!(offers, 1);
}

#[test]
fn a_late_startup_offer_waits_for_home_and_ignores_held_enter() {
    let mut w = world_with_config(Patched::updates().unwrap().prompt_at_start());
    let secret = w.secret.clone();
    publish(&w, &secret, true, "1.1.0");
    w.h.app.ctx.push(Playing);
    let settings = w.h.app.ctx.settings.updates.clone();
    w.h.app.ctx.updates.check_at_start(&settings);
    for _ in 0..50 {
        w.h.frame(0.016);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(w.h.app.ctx.updates.available().is_some());
    assert!(!w.h.spoken().iter().any(|s| s.contains("Update available")));
    w.h.app.ctx.pop();
    frames_until(&mut w.h, "Checking for updates.");
    let mut held = hearken_app::InputEvent::key(Key::Return);
    if let hearken_app::InputEvent::KeyDown { repeat, .. } = &mut held {
        *repeat = true;
    }
    w.h.app.step(vec![held], 0.016);
    frames_until(&mut w.h, "Update available");
    let mut held = hearken_app::InputEvent::key(Key::Return);
    if let hearken_app::InputEvent::KeyDown { repeat, .. } = &mut held {
        *repeat = true;
    }
    w.h.app.step(vec![held], 0.016);
    assert!(w.h.app.top_lines().join(" ").contains("Update available"));
    assert!(w.applied.lock().unwrap().is_empty());
    w.h.press(Key::Return);
    assert!(!w.h.app.top_lines().join(" ").contains("Update available"));
}

/// A zip release with a file index, as `hearken release` publishes for
/// Windows and Mac: only its manifest and index, which is all a check reads.
fn publish_indexed(world: &World) -> String {
    let index = b"{}".to_vec();
    let manifest = Manifest {
        format: FORMAT,
        built: None,
        game: "patched".into(),
        version: "1.1.0".into(),
        channel: "stable".into(),
        platform: PLATFORM.into(),
        archive: format!("patched-1.1.0-{PLATFORM}.zip"),
        url: None,
        size: 1000,
        sha256: "0".repeat(64),
        notes: String::new(),
        published: None,
        index: Some(hearken_app::update::IndexRef {
            file: format!("patched-1.1.0-{PLATFORM}.index.json"),
            size: index.len() as u64,
            sha256: "0".repeat(64),
        }),
    };
    let signed = manifest::sign(&manifest, &world.secret).unwrap();
    let index_url = format!("https://example.test/patched/patched-1.1.0-{PLATFORM}.index.json");
    let mut table = world.table.0.lock().unwrap();
    table.insert(
        format!("https://example.test/patched/update-stable-{PLATFORM}.json"),
        signed.into_bytes(),
    );
    table.insert(index_url.clone(), index);
    world.table.2.store(true, Ordering::SeqCst);
    index_url
}

/// The check at start only says whether there is an update: it never
/// fetches the file index or hashes the install, so a version the player
/// skipped, or a quiet game's, costs nothing more. Check for updates does.
#[test]
fn only_check_for_updates_looks_for_the_files_the_player_has() {
    let mut w = world();
    let index_url = publish_indexed(&w);
    let settings = w.h.app.ctx.settings.updates.clone();
    w.h.app.ctx.updates.check_at_start(&settings);
    frames_until(&mut w.h, "Patched version 1.1.0 is available.");
    assert!(!w.table.1.lock().unwrap().contains(&index_url));

    w.check();
    frames_until(&mut w.h, "Update available");
    assert!(w.table.1.lock().unwrap().contains(&index_url));
}

/// An update the check at start found, with a file index, is checked
/// again from Settings' "Install version" row, so its offer and download
/// take only the files that changed.
#[test]
fn installing_from_settings_looks_for_the_files_the_player_has() {
    let mut w = world();
    let index_url = publish_indexed(&w);
    let settings = w.h.app.ctx.settings.updates.clone();
    w.h.app.ctx.updates.check_at_start(&settings);
    frames_until(&mut w.h, "Patched version 1.1.0 is available.");
    assert!(!w.table.1.lock().unwrap().contains(&index_url));
    w.h.app.ctx.reset_to(SettingsScreen::<Patched>::new());
    w.h.frame(0.016);
    for _ in 0..40 {
        if w.h
            .spoken()
            .last()
            .is_some_and(|l| l.contains("Install version"))
        {
            break;
        }
        w.h.press(Key::Down);
        w.h.frame(0.016);
    }
    assert!(
        w.h.spoken()
            .last()
            .is_some_and(|l| l.contains("Install version")),
        "{:?}",
        w.h.spoken()
    );
    w.h.press(Key::Return);
    frames_until(&mut w.h, "Update available");
    assert!(w.table.1.lock().unwrap().contains(&index_url));
}

#[test]
fn escape_cancels_a_download() {
    let mut w = world();
    let secret = w.secret.clone();
    publish(&w, &secret, true, "1.1.0");
    w.check();
    frames_until(&mut w.h, "Update available");
    w.h.press(Key::Return);
    w.h.press(Key::Escape);
    assert!(w.h.spoken().iter().any(|l| l.contains("Update canceled")));
    for _ in 0..30 {
        w.h.frame(0.016);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(w.applied.lock().unwrap().is_empty());
    assert!(w.h.app.ctx.running);
}

/// An update the game cannot install by itself goes to the player's home
/// folder, and nothing of it is left in the temporary folder.
#[test]
fn an_update_installed_by_hand_leaves_no_staging_behind() {
    let mut w = world();
    let secret = w.secret.clone();
    publish(&w, &secret, true, "1.1.0");
    let home = w.dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut host = Host::fake(Os::Linux, &w.dir.path().join("Patched").join(EXE));
    host.home = Some(home.clone());
    w.h.app.ctx.updates.set_host(host);
    let record = w.applied.clone();
    w.h.app
        .ctx
        .updates
        .set_apply(Arc::new(move |staged, _install, staging, _host| {
            record
                .lock()
                .unwrap()
                .push((staged.clone(), staging.to_path_buf()));
            Err(std::io::Error::other("no shell"))
        }));
    w.check();
    frames_until(&mut w.h, "Update available");
    w.h.clear();
    w.h.press(Key::Return);
    frames_until(&mut w.h, "could not install itself");
    let applied = w.applied.lock().unwrap().clone();
    let [(_, staging)] = applied.as_slice() else {
        panic!("not applied: {applied:?}");
    };
    let saved = home.join(format!("patched-1.1.0-{PLATFORM}"));
    assert_eq!(std::fs::read(saved.join(EXE)).unwrap(), b"hi");
    assert!(!staging.exists(), "{} was left behind", staging.display());
    assert!(w.h.app.ctx.running, "the game stays open");
}

#[test]
fn agnow_hides_update_controls_and_preserves_preferences() {
    let mut w = world();
    w.h.app.ctx.settings.updates.channel = Some("beta".into());
    w.h.app.ctx.settings.updates.skipped = Some("1.1.0".into());
    let before = serde_json::to_value(&w.h.app.ctx.settings.updates).unwrap();
    for managed in [true, false] {
        w.h.app.ctx.updates.set_agnow(managed);
        assert_eq!(w.h.app.ctx.updates.is_offered(), !managed);
        w.h.app.ctx.reset_to(SettingsScreen::<Patched>::new());
        w.h.frame(0.016);
        let lines = w.h.app.top_lines();
        for label in [
            "Check for updates",
            "Update channel: beta",
            "Check for updates at start: on",
        ] {
            assert_eq!(
                lines.iter().any(|line| line.contains(label)),
                !managed,
                "{lines:?}"
            );
        }
        assert_eq!(
            serde_json::to_value(&w.h.app.ctx.settings.updates).unwrap(),
            before
        );
    }
}

#[test]
fn agnow_blocks_startup_prompts_and_manual_requests() {
    let mut w = world_with_config(Patched::updates().unwrap().prompt_at_start());
    w.h.app.ctx.updates.set_agnow(true);
    let settings = w.h.app.ctx.settings.updates.clone();
    w.h.clear();
    w.h.app.ctx.updates.check_at_start(&settings);
    for _ in 0..10 {
        w.h.frame(0.016);
    }
    assert!(w.h.app.ctx.updates.available().is_none());
    assert!(!w
        .h
        .spoken()
        .iter()
        .any(|line| line.contains("is available")));
    w.check();
    assert!(w
        .h
        .spoken()
        .iter()
        .any(|line| line.contains("AG Now keeps this copy up to date")));
    assert!(w.table.1.lock().unwrap().is_empty());
    assert!(w.applied.lock().unwrap().is_empty());
}
