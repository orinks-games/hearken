mod common;

use common::{Probe, TestGame};
use hearken_app::seal::{Key as SealKey, Vault};
use hearken_app::soundtrack::{load_from_vault, save_to_vault, FILE};
use hearken_app::{Bus, Headless, Key, SoundtrackScreen, Stems, Track, Variation, Variety};

fn tracks() -> Vec<Track> {
    vec![
        Track::clip("title", "Title theme", "music/title")
            .hint("Open the game.")
            .about("Plays on the title screen."),
        Track::stems(
            "night",
            "Night song",
            Stems::new(4.0)
                .bed(["song/base"])
                .voices(["song/a", "song/b", "song/c"]),
        )
        .hint("Finish a night."),
        Track::clip("secret", "Hidden waltz", "music/waltz").secret(),
        Track::clip("free", "Welcome", "music/welcome").open(),
    ]
}

fn game() -> Headless<TestGame> {
    let mut h = Headless::new(Default::default(), Probe::new("base"));
    h.app.ctx.soundtrack.set_tracks(tracks());
    h.frame(0.0);
    h
}

fn saved(h: &Headless<TestGame>) -> String {
    std::fs::read_to_string(h.app.ctx.vault.dir().join(FILE)).unwrap_or_default()
}

#[test]
fn hearing_a_track_unlocks_and_saves_it() {
    let mut h = game();
    assert!(!h.app.ctx.track_unlocked("title"));
    h.app.ctx.audio.play("music/title", Bus::MUSIC);
    h.frame(0.016);
    assert!(h.app.ctx.track_unlocked("title"));
    assert!(saved(&h).contains("\"title\""));
    assert_eq!(h.app.ctx.soundtrack.take_new(), vec!["Title theme"]);
    assert!(h.app.ctx.soundtrack.take_new().is_empty());
}

#[test]
fn the_game_can_unlock_a_track_itself() {
    let mut h = game();
    assert!(h.app.ctx.unlock_track("night"));
    assert!(!h.app.ctx.unlock_track("night"));
    assert!(!h.app.ctx.unlock_track("no such track"));
    assert!(saved(&h).contains("\"night\""));
}

#[test]
fn a_sound_test_can_play_music_without_unlocking_it() {
    let mut h = game();
    h.app.ctx.soundtrack.listen(false);
    h.app.ctx.audio.play("music/title", Bus::MUSIC);
    h.frame(0.016);
    assert!(!h.app.ctx.track_unlocked("title"));
    // Heard while not listening does not count later either.
    h.app.ctx.soundtrack.listen(true);
    h.frame(0.016);
    assert!(!h.app.ctx.track_unlocked("title"));
}

#[test]
fn open_tracks_are_unlocked_from_the_start() {
    let mut h = game();
    assert!(h.app.ctx.track_unlocked("free"));
}

#[test]
fn the_screen_lists_locked_tracks_but_not_secret_ones() {
    let mut h = game();
    h.app.ctx.push(SoundtrackScreen::new());
    h.frame(0.0);
    let lines = h.app.top_lines().join("\n");
    assert!(lines.contains("Welcome"), "{lines}");
    assert!(lines.contains("Locked track"), "{lines}");
    assert!(!lines.contains("Hidden waltz"), "{lines}");
    assert!(
        h.spoken()
            .iter()
            .any(|l| l.contains("1 of 3 tracks unlocked")),
        "{:?}",
        h.spoken()
    );
    // The secret one is listed once it is heard.
    h.app.ctx.unlock_track("secret");
    h.press(Key::Down);
    h.app.ctx.pop();
    h.frame(0.0);
    h.app.ctx.push(SoundtrackScreen::new());
    h.frame(0.0);
    assert!(h.app.top_lines().join("\n").contains("Hidden waltz"));
}

#[test]
fn a_locked_track_reads_its_hint() {
    let mut h = game();
    h.app.ctx.push(SoundtrackScreen::new());
    h.frame(0.0);
    // Rows: Title theme (locked), Night song (locked), Welcome, Back.
    h.clear();
    h.press(Key::Return);
    assert_eq!(h.spoken(), vec!["Locked. Open the game."]);
    assert!(h.played().is_empty());
}

#[test]
fn enter_plays_and_stops_and_the_menu_music_comes_back() {
    let mut h = game();
    h.app.ctx.audio.play_music(Some("music/menu"), true);
    h.app.ctx.unlock_track("title");
    h.app.ctx.push(SoundtrackScreen::new());
    h.frame(0.0);
    assert_eq!(h.app.ctx.audio.music(), None);
    // Rows: Play all, Title theme, Night song (locked), Welcome, Back.
    h.press(Key::Down);
    h.clear();
    h.press(Key::Return);
    assert_eq!(h.played(), vec!["music/title"]);
    assert_eq!(h.spoken(), vec!["Playing Title theme."]);
    assert!(h
        .app
        .top_lines()
        .join("\n")
        .contains("Title theme, playing"));
    h.clear();
    h.press(Key::Return);
    assert_eq!(h.spoken(), vec!["Stopped."]);
    h.press(Key::Escape);
    h.frame(0.0);
    assert_eq!(h.app.ctx.audio.music(), Some("music/menu"));
}

#[test]
fn stems_start_together_and_never_drift() {
    let mut h = game();
    h.app.ctx.unlock_track("night");
    let track = h.app.ctx.soundtrack.track("night").unwrap().clone();
    let mut p = track.perform(Variety::new(7));
    h.clear();
    p.start(&mut h.app.ctx.audio);
    let mut keys = h.played();
    keys.sort();
    assert_eq!(keys, vec!["song/a", "song/b", "song/base", "song/c"]);
    // It plays a while, starting nothing more, then ends.
    h.clear();
    let mut t = 0.0;
    while p.update(&mut h.app.ctx.audio, 0.1) {
        t += 0.1;
        assert!(t < 600.0, "never ended");
    }
    assert!(h.played().is_empty());
    assert!(t > 60.0, "ended after {t}s");
}

#[test]
fn every_play_is_arranged_differently() {
    let stems = Stems::new(4.0).voices(["a", "b", "c", "d", "e", "f"]);
    let plans: Vec<_> = (0..8).map(|s| stems.plan(&mut Variety::new(s))).collect();
    for plan in &plans {
        // Every voice is heard, no loop is silent, and it thins out to one.
        for v in 0..6 {
            assert!(plan.iter().any(|pass| pass[v]), "voice {v} never plays");
        }
        assert!(plan.iter().all(|pass| pass.iter().any(|&o| o)));
        assert_eq!(plan.last().unwrap().iter().filter(|&&o| o).count(), 1);
    }
    let distinct: std::collections::BTreeSet<_> = plans.iter().collect();
    assert!(
        distinct.len() >= 6,
        "only {} distinct plans",
        distinct.len()
    );
    // The same seed plays the same arrangement.
    assert_eq!(stems.plan(&mut Variety::new(3)), plans[3]);
}

#[test]
fn variation_stays_within_its_range() {
    let mut v = Variety::new(1);
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..200 {
        let p = Variation::GENTLE.pitch(&mut v);
        assert!((0.94..=1.06).contains(&p), "{p}");
        seen.insert((p * 1000.0) as i32);
    }
    assert_eq!(seen.len(), 3, "down, as recorded, up");
    assert_eq!(Variation::NONE.pitch(&mut v), 1.0);
}

#[test]
fn a_tampered_save_starts_the_soundtrack_over() {
    let dir = tempfile::tempdir().unwrap();
    let key = SealKey::from_bytes([7; 32]);
    let mut vault = Vault::with_key(dir.path(), Some(key.clone()));
    let unlocked: std::collections::BTreeSet<String> = ["title".to_string()].into();
    save_to_vault(&mut vault, &unlocked);
    assert_eq!(load_from_vault(&mut vault, false), unlocked);

    // Flip a byte: the file no longer matches its seal.
    let path = dir.path().join(FILE);
    let mut bytes = std::fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&path, bytes).unwrap();
    assert!(load_from_vault(&mut vault, false).is_empty());
    assert!(!path.exists(), "the tampered file is set aside");

    // A hand-written plain file is not taken either.
    std::fs::write(&path, br#"{"unlocked":["title","night"]}"#).unwrap();
    assert!(load_from_vault(&mut vault, false).is_empty());
}
