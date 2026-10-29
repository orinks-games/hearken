mod common;

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{Probe, TestAction, TestCategory, TestGame};
use hearken_app::audio::{
    Audio, Backend, BackendError, Channel, MemoryClips, Mix, RecordingBackend, Source,
};
use hearken_app::screens::PauseScreen;
use hearken_app::seal::{Key as SealKey, Vault};
use hearken_app::soundtrack::{load_from_vault, save_to_vault, FILE};
use hearken_app::{
    Bus, Call, Ctx, Game, Headless, InputEvent, Key, SoundtrackScreen, State, Stems, Track,
    Variation, Variety,
};

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
    let menu = h.app.ctx.audio.music_sound().unwrap();
    assert!(h.app.ctx.audio.is_paused(menu), "the menu music is held");
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
    h.clear();
    h.press(Key::Escape);
    h.frame(0.0);
    assert_eq!(h.app.ctx.audio.music(), Some("music/menu"));
    assert!(!h.app.ctx.audio.is_paused(menu), "and goes on where it was");
    assert!(!h.played().contains(&"music/menu".to_string()));
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
        // Every voice is heard, and with no bed no loop holds fewer than
        // two voices; it thins out to two.
        for v in 0..6 {
            assert!(plan.iter().any(|pass| pass[v]), "voice {v} never plays");
        }
        assert!(plan
            .iter()
            .all(|pass| pass.iter().filter(|&&o| o).count() >= 2));
        assert_eq!(plan.last().unwrap().iter().filter(|&&o| o).count(), 2);
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
    assert_eq!(seen.len(), 2, "down or up, never as recorded");
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

// --- Unlocking: regressions ------------------------------------------------

/// A game whose soundtrack comes from `Game::soundtrack`.
struct StGame;

impl Game for StGame {
    const TITLE: &'static str = "Soundtrack Game";
    const DATA_DIR: &'static str = "HearkenSoundtrackGame";
    const ENV_PREFIX: &'static str = "HEARKEN_SOUNDTRACK_ST";
    type Event = u32;
    type Data = ();
    type Settings = ();
    type Action = TestAction;
    type Category = TestCategory;

    fn verbosity() -> hearken_app::Verbosity<TestCategory> {
        TestGame::verbosity()
    }

    fn soundtrack() -> Vec<Track> {
        vec![Track::clip("title", "Title theme", "music/title")]
    }
}

/// A first screen that starts its theme on arrival, as title screens do.
struct Title;

impl State<StGame> for Title {
    fn name(&self) -> &'static str {
        "title"
    }
    fn help(&self, _ctx: &Ctx<StGame>) -> String {
        String::new()
    }
    fn enter(&mut self, ctx: &mut Ctx<StGame>) {
        ctx.audio.play_music(Some("music/title"), true);
    }
}

struct TestTitle;

impl State<TestGame> for TestTitle {
    fn name(&self) -> &'static str {
        "title"
    }
    fn help(&self, _ctx: &Ctx<TestGame>) -> String {
        String::new()
    }
    fn enter(&mut self, ctx: &mut Ctx<TestGame>) {
        ctx.audio.play_music(Some("music/title"), true);
    }
}

#[test]
fn music_started_by_the_first_screen_unlocks_its_track() {
    // The first screen's enter runs before the watch list exists, and a
    // loop starts only once.
    let mut h: Headless<StGame> = Headless::new((), Title);
    h.frame(0.1);
    assert!(h.app.ctx.track_unlocked("title"));

    // The same with the list set after the game starts.
    let mut h = Headless::new(Default::default(), TestTitle);
    h.app.ctx.soundtrack.set_tracks(tracks());
    for _ in 0..3 {
        h.frame(0.016);
    }
    assert_eq!(h.app.ctx.audio.music(), Some("music/title"));
    assert!(h.app.ctx.track_unlocked("title"));
}

#[test]
fn music_started_the_frame_its_track_is_added_unlocks_it() {
    let mut h = Headless::new(Default::default(), Probe::new("base"));
    h.app
        .ctx
        .soundtrack
        .set_tracks(vec![Track::clip("one", "Level one", "music/one")]);
    h.frame(0.0);
    // Level two: new tracks, and its music starts in the same step.
    h.app.ctx.soundtrack.set_tracks(vec![
        Track::clip("one", "Level one", "music/one"),
        Track::clip("two", "Level two", "music/two"),
    ]);
    h.app.ctx.audio.play_music(Some("music/two"), true);
    h.frame(0.1);
    assert!(h.app.ctx.track_unlocked("two"));
}

#[test]
fn the_soundtrack_screen_does_not_unlock_tracks_by_playing_them() {
    let mut h = Headless::new(Default::default(), Probe::new("base"));
    h.app.ctx.soundtrack.set_tracks(vec![
        Track::stems(
            "medley",
            "Medley",
            Stems::new(4.0).bed(["music/waltz"]).voices(["medley/a"]),
        )
        .open(),
        Track::clip("waltz", "Hidden waltz", "music/waltz").secret(),
    ]);
    h.frame(0.0);
    h.app.ctx.push(SoundtrackScreen::new());
    h.frame(0.0);
    // Rows: Medley, Back.
    h.press(Key::Return);
    assert!(h.played().contains(&"music/waltz".to_string()));
    h.frame(0.016);
    h.frame(0.016);
    assert!(!h.app.ctx.track_unlocked("waltz"));
    // Nor once the screen closes on it.
    h.press(Key::Escape);
    h.frame(0.016);
    h.frame(0.016);
    assert!(!h.app.ctx.track_unlocked("waltz"));
    assert!(h.app.ctx.soundtrack.is_listening());
}

#[test]
fn a_failed_read_never_overwrites_the_saved_unlocks() {
    let mut h = Headless::new(Default::default(), Probe::new("base"));
    let dir = h.app.ctx.vault.dir().to_path_buf();
    h.app.ctx.vault = Vault::with_key(&dir, Some(SealKey::from_bytes([7; 32])));
    h.app.ctx.soundtrack.set_tracks(vec![
        Track::clip("a", "A", "music/a"),
        Track::clip("b", "B", "music/b"),
        Track::clip("c", "C", "music/c"),
        Track::clip("d", "D", "music/d"),
    ]);
    let earned: BTreeSet<String> = ["a".to_string(), "b".to_string()].into();
    save_to_vault(&mut h.app.ctx.vault, &earned);
    let path = dir.join(FILE);
    let sealed = std::fs::read(&path).unwrap();

    // The read fails on the first frame (here a folder in the file's place).
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    h.frame(0.0);
    assert!(!h.app.ctx.track_unlocked("a"));
    // An unlock while it still fails writes nothing over it.
    h.app.ctx.unlock_track("d");
    assert!(path.is_dir());
    // The file is back a moment later.
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, &sealed).unwrap();

    h.app.ctx.unlock_track("c");
    let now = load_from_vault(
        &mut Vault::with_key(&dir, Some(SealKey::from_bytes([7; 32]))),
        false,
    );
    let want: BTreeSet<String> = ["a", "b", "c", "d"].map(String::from).into();
    assert_eq!(now, want);
    // And the session has them back too.
    assert!(h.app.ctx.track_unlocked("a"));
}

// --- The screen: regressions ------------------------------------------------

fn live_music(h: &Headless<TestGame>) -> Vec<String> {
    h.app
        .ctx
        .audio
        .live()
        .into_iter()
        .filter(|l| l.bus == Bus::MUSIC)
        .map(|l| l.key)
        .collect()
}

#[test]
fn a_covered_soundtrack_screen_stops_playing() {
    let mut h = game();
    h.app.ctx.unlock_track("night");
    h.app.ctx.push(SoundtrackScreen::new());
    h.frame(0.0);
    // Rows: Play all, Title theme (locked), Night song, Welcome, Back.
    h.press(Key::Down);
    h.press(Key::Down);
    h.press(Key::Return);
    assert!(live_music(&h).contains(&"song/a".to_string()));
    h.app.ctx.push(Probe::new("cover"));
    h.frame(0.0);
    assert!(
        live_music(&h).iter().all(|k| !k.starts_with("song/")),
        "still playing under the cover: {:?}",
        live_music(&h)
    );
}

#[test]
fn opening_the_soundtrack_from_pause_leaves_the_game_music_paused() {
    let mut h = game();
    h.app.ctx.audio.play_music(Some("music/level"), true);
    h.app.ctx.push(PauseScreen::<TestGame>::new("", |_| {}));
    h.frame(0.0);
    h.app.ctx.push(SoundtrackScreen::new());
    h.frame(0.0);
    h.clear();
    h.press(Key::Escape);
    h.frame(0.0);
    assert_eq!(h.app.top_name(), "pause");
    assert!(!h.played().contains(&"music/level".to_string()));
    let level = h.app.ctx.audio.music_sound().unwrap();
    assert!(h.app.ctx.audio.is_paused(level));
    // Resuming the game resumes it where it was.
    h.press(Key::Escape);
    h.frame(0.0);
    assert!(!h.app.ctx.audio.is_paused(level));
}

#[test]
fn play_all_says_what_comes_next() {
    let mut h = game();
    h.app.ctx.unlock_track("title");
    h.app.ctx.push(SoundtrackScreen::new());
    h.frame(0.0);
    h.press(Key::Return); // Play all, shuffled.
    let first = h
        .played()
        .into_iter()
        .find(|k| k.starts_with("music/"))
        .expect("a track started");
    let ch = h.sounds.channel_of(&first).unwrap();
    h.clear();
    h.sounds.finish(ch);
    h.frame(0.016);
    h.frame(0.016);
    assert!(h.played().iter().any(|k| k.starts_with("music/")));
    assert!(
        h.spoken().iter().any(|l| l.starts_with("Playing ")),
        "{:?}",
        h.spoken()
    );
}

#[test]
fn a_title_ending_in_a_question_mark_is_read_cleanly() {
    let mut h = Headless::new(Default::default(), Probe::new("base"));
    h.app
        .ctx
        .soundtrack
        .set_tracks(vec![Track::clip("q", "Who goes there?", "music/q").open()]);
    h.frame(0.0);
    h.app.ctx.push(SoundtrackScreen::new());
    h.frame(0.0);
    let intro = h.spoken();
    h.clear();
    h.press(Key::Return);
    assert!(
        intro.iter().any(|l| l.contains("1 of 1 track unlocked.")),
        "{intro:?}"
    );
    assert_eq!(h.spoken(), vec!["Playing Who goes there?"]);
}

// --- Every play differs: regressions -----------------------------------------

#[test]
fn how_long_a_play_runs_varies() {
    let stems = Stems::new(4.0).voices(["a", "b", "c", "d", "e", "f"]);
    let lengths: BTreeSet<usize> = (0..64)
        .map(|s| stems.plan(&mut Variety::new(s)).len())
        .collect();
    assert!(lengths.len() > 1, "every play has {lengths:?} loops");
    // About 150 s of 4 s loops, a fifth either way.
    assert!(
        lengths.iter().all(|&n| (30..=45).contains(&n)),
        "{lengths:?}"
    );
}

#[test]
fn a_one_voice_piece_is_still_arranged_anew() {
    let stems = Stems::new(4.0).bed(["pad"]).voices(["lead"]);
    let plans: BTreeSet<_> = (0..64).map(|s| stems.plan(&mut Variety::new(s))).collect();
    assert!(plans.len() > 1, "64 plays, one arrangement");
}

#[test]
fn no_play_lays_a_stem_bare() {
    // With no bed, every loop holds at least two voices.
    let stems = Stems::new(4.0).voices(["a", "b", "c", "d"]);
    for s in 0..64 {
        for pass in stems.plan(&mut Variety::new(s)) {
            assert!(
                pass.iter().filter(|&&o| o).count() >= 2,
                "seed {s}: {pass:?}"
            );
        }
    }
}

#[test]
fn a_clip_track_is_never_the_recording_as_is() {
    let mut v = Variety::new(5);
    assert!((0..300).all(|_| Variation::GENTLE.pitch(&mut v) != 1.0));
}

#[test]
fn a_repeated_clip_plays_its_repeats_and_no_more() {
    let mut h = game();
    h.sounds.set_length("music/c", 10.0);
    let track = Track::clip("c", "C", "music/c")
        .repeats(3)
        .variation(Variation::NONE);
    let mut p = track.perform(Variety::new(1));
    p.start(&mut h.app.ctx.audio);
    let mut t = 0.0;
    let mut released = None;
    while t < 40.0 {
        p.update(&mut h.app.ctx.audio, 0.1);
        t += 0.1;
        if released.is_none()
            && h.sounds
                .calls()
                .iter()
                .any(|c| matches!(c, Call::Release { .. }))
        {
            released = Some(t);
        }
    }
    // Let go during the third time through, so it plays out its end there.
    let at = released.expect("never released");
    assert!((20.0..29.9).contains(&at), "released at {at:.2}s");
}

#[test]
fn a_wide_variation_stays_in_the_mixers_pitch_range() {
    let variation = Variation {
        semitones: 36,
        level_db: 0.0,
        pan: 0.0,
    };
    let mut v = Variety::new(3);
    for _ in 0..1000 {
        let p = variation.pitch(&mut v);
        assert!((0.5..=2.0).contains(&p), "{p}");
    }
}

#[test]
fn an_endless_length_does_not_crash_the_game() {
    let endless = Stems::new(4.0).voices(["a"]).length_s(f64::INFINITY);
    let n = endless.plan(&mut Variety::new(1)).len();
    assert!((30..=45).contains(&n), "{n}: the default length");
    let huge = Stems::new(4.0).voices(["a"]).length_s(1e300);
    assert!(huge.plan(&mut Variety::new(1)).len() <= 1100);
}

#[test]
fn stems_that_will_not_play_do_not_hold_silence() {
    let mut h = game();
    h.app.ctx.audio.set_default_fallback(false);
    let track = Track::stems(
        "m",
        "M",
        Stems::new(4.0).voices(["gone/a", "gone/b", "gone/c"]),
    );
    let mut p = track.perform(Variety::new(1));
    p.start(&mut h.app.ctx.audio);
    assert!(live_music(&h).is_empty());
    assert!(!p.update(&mut h.app.ctx.audio, 0.1));
}

// --- Round 2: unlocks ---------------------------------------------------------

/// A game's sound test, written the way the README says: `listen(false)`
/// while it is open. Enter previews the title theme, Escape leaves.
struct SoundTest;

impl State<TestGame> for SoundTest {
    fn name(&self) -> &'static str {
        "sound test"
    }
    fn help(&self, _ctx: &Ctx<TestGame>) -> String {
        String::new()
    }
    fn enter(&mut self, ctx: &mut Ctx<TestGame>) {
        ctx.soundtrack.listen(false);
    }
    fn exit(&mut self, ctx: &mut Ctx<TestGame>) {
        ctx.soundtrack.listen(true);
    }
    fn handle_input(&mut self, ctx: &mut Ctx<TestGame>, event: &InputEvent) {
        match event.key_down().map(|(k, _, _)| k) {
            Some(Key::Return) => {
                ctx.audio.play("music/title", Bus::MUSIC);
            }
            Some(Key::Escape) => ctx.pop(),
            _ => {}
        }
    }
}

#[test]
fn a_sound_test_preview_never_unlocks_even_when_left_the_same_frame() {
    let mut h = game();
    h.app.ctx.push(SoundTest);
    h.frame(0.0);
    assert!(!h.app.ctx.soundtrack.is_listening());
    // Enter and Escape in one batch, as a key macro sends them.
    h.app.step(
        vec![InputEvent::key(Key::Return), InputEvent::key(Key::Escape)],
        0.016,
    );
    h.frame(0.016);
    assert!(h.app.ctx.soundtrack.is_listening());
    assert!(!h.app.ctx.track_unlocked("title"));
}

#[test]
fn heard_while_deaf_does_not_count_after_listening_resumes_in_the_same_frame() {
    let mut h = game();
    h.app.ctx.soundtrack.listen(false);
    h.app.ctx.audio.play("music/title", Bus::MUSIC);
    h.app.ctx.soundtrack.listen(true);
    h.frame(0.016);
    assert!(!h.app.ctx.track_unlocked("title"));
    // Listening for real still works.
    h.app.ctx.audio.play("music/title", Bus::MUSIC);
    h.frame(0.016);
    assert!(h.app.ctx.track_unlocked("title"));
}

#[test]
fn keys_heard_while_the_list_was_empty_and_deaf_do_not_unlock_later() {
    let mut h = game();
    h.app.ctx.soundtrack.set_tracks(Vec::new());
    h.frame(0.016);
    h.app.ctx.soundtrack.listen(false);
    h.app.ctx.audio.play("music/title", Bus::MUSIC);
    h.frame(0.016);
    h.app.ctx.soundtrack.listen(true);
    h.frame(0.016);
    h.frame(5.0);
    h.app.ctx.soundtrack.set_tracks(tracks());
    for l in h.app.ctx.audio.live() {
        h.app.ctx.audio.stop(l.sound, 0);
    }
    h.frame(0.016);
    assert!(!h.app.ctx.track_unlocked("title"));
}

#[test]
fn a_track_earned_while_the_file_is_held_is_saved_once_it_is_free() {
    let key = SealKey::from_bytes([7; 32]);
    let mut h = Headless::new(Default::default(), Probe::new("base"));
    let dir = h.app.ctx.vault.dir().to_path_buf();
    h.app.ctx.vault = Vault::with_key(&dir, Some(key.clone()));
    h.app.ctx.soundtrack.set_tracks(tracks());
    h.frame(0.0);
    let path = dir.join(FILE);
    // Held: the read fails (a folder in the file's place).
    std::fs::create_dir(&path).unwrap();
    assert!(h.app.ctx.unlock_track("night"));
    std::fs::remove_dir(&path).unwrap();
    for _ in 0..600 {
        h.frame(0.016);
    }
    let on_disk = load_from_vault(&mut Vault::with_key(&dir, Some(key)), false);
    assert!(on_disk.contains("night"), "{on_disk:?}");
}

// --- Round 2: playback --------------------------------------------------------

/// A backend whose `play` takes what Kira's does to decode a real stem
/// (about 100 ms), and notes when each sound can first be heard: at once,
/// or when a hold is let go.
struct SlowStart {
    starts: Arc<Mutex<Vec<(String, Instant)>>>,
    held: Option<Vec<String>>,
    next: Channel,
}

impl Backend for SlowStart {
    fn name(&self) -> String {
        "slow".into()
    }
    fn play(
        &mut self,
        key: &str,
        _source: Source,
        _looping: bool,
        _mix: Mix,
    ) -> Result<Channel, BackendError> {
        std::thread::sleep(Duration::from_millis(100));
        match &mut self.held {
            Some(held) => held.push(key.to_string()),
            None => self
                .starts
                .lock()
                .unwrap()
                .push((key.to_string(), Instant::now())),
        }
        self.next += 1;
        Ok(self.next)
    }
    fn hold(&mut self, held: bool) {
        if held {
            self.held.get_or_insert_with(Vec::new);
        } else {
            let now = Instant::now();
            let mut starts = self.starts.lock().unwrap();
            for key in self.held.take().unwrap_or_default() {
                starts.push((key, now));
            }
        }
    }
    fn set_mix(&mut self, _c: Channel, _m: Mix, _r: u32) {}
    fn stop(&mut self, _c: Channel, _f: u32) {}
    fn pause(&mut self, _c: Channel, _p: bool) {}
    fn is_playing(&mut self, _c: Channel) -> bool {
        true
    }
}

#[test]
fn stems_start_together_even_when_each_start_takes_a_decode() {
    let starts = Arc::new(Mutex::new(Vec::new()));
    let mut audio = Audio::new(
        Box::new(SlowStart {
            starts: starts.clone(),
            held: None,
            next: 0,
        }),
        Box::new(MemoryClips::default()),
    );
    let track = Track::stems(
        "s",
        "S",
        Stems::new(4.0)
            .bed(["base", "pulse"])
            .voices(["melody", "lift", "hurry", "danger"]),
    );
    let mut p = track.perform(Variety::new(1));
    p.start(&mut audio);
    let starts = starts.lock().unwrap();
    assert_eq!(starts.len(), 6);
    let first = starts.iter().map(|s| s.1).min().unwrap();
    let last = starts.iter().map(|s| s.1).max().unwrap();
    assert!(
        last - first < Duration::from_millis(15),
        "{:?}",
        last - first
    );
}

#[test]
fn a_piece_whose_stems_do_not_all_load_does_not_play() {
    let mut h = game();
    let (backend, rec) = RecordingBackend::new();
    // The bed's pack entry is damaged; the voices are fine.
    let clips = MemoryClips::default()
        .with("a", vec![1u8; 8])
        .with("b", vec![2u8; 8])
        .with("c", vec![3u8; 8]);
    h.app
        .ctx
        .install_audio(Audio::new(Box::new(backend), Box::new(clips)));
    let track = Track::stems(
        "s",
        "S",
        Stems::new(4.0).bed(["pad"]).voices(["a", "b", "c"]),
    )
    .variation(Variation::NONE)
    .open();
    for seed in 0..8 {
        let mut p = track.perform(Variety::new(seed));
        p.start(&mut h.app.ctx.audio);
        assert!(!p.update(&mut h.app.ctx.audio, 0.0), "seed {seed} plays");
        for k in ["pad", "a", "b", "c"] {
            if let Some(c) = rec.channel_of(k) {
                assert!(!rec.is_playing(c), "seed {seed}: {k} still plays");
            }
        }
        rec.clear();
    }
    // And the screen says so.
    h.app.ctx.soundtrack.set_tracks(vec![track]);
    h.frame(0.0);
    h.app.ctx.push(SoundtrackScreen::new());
    h.frame(0.0);
    h.clear();
    h.press(Key::Return);
    assert_eq!(h.spoken(), vec!["Can't play that track."]);
}

fn ended(h: &Headless<TestGame>) -> bool {
    h.sounds.calls().iter().any(|c| {
        matches!(c, Call::SetMixRamped { ramps, mix, .. } if ramps.gain_ms == 2500 && mix.gain == 0.0)
    })
}

#[test]
fn a_frame_hitch_does_not_put_the_arrangement_behind_the_music() {
    let mut h = game();
    // 6 loops of 0.5 s: the plan is done 3 s after the start.
    let track = Track::stems(
        "s",
        "S",
        Stems::new(0.5).bed(["p"]).voices(["a", "b"]).length_s(2.0),
    )
    .variation(Variation::NONE);
    let mut p = track.perform(Variety::new(4));
    p.start(&mut h.app.ctx.audio);
    h.clear();
    let began = Instant::now();
    // Frames that each took 0.3 s, reported as 0.05 s.
    while began.elapsed() < Duration::from_millis(3600) {
        std::thread::sleep(Duration::from_millis(300));
        p.update(&mut h.app.ctx.audio, 0.05);
    }
    assert!(
        ended(&h),
        "3.6 s of music played and the 3 s piece has not ended"
    );
}

#[test]
fn an_endless_loop_length_still_ends() {
    let mut h = game();
    for loop_s in [f64::INFINITY, f64::NAN, 1e12] {
        let track = Track::stems("s", "S", Stems::new(loop_s).bed(["p"]).voices(["a", "b"]));
        let mut p = track.perform(Variety::new(1));
        p.start(&mut h.app.ctx.audio);
        let mut t = 0.0;
        while p.update(&mut h.app.ctx.audio, 1.0) {
            t += 1.0;
            assert!(
                t < 4.0 * 3600.0,
                "loop_s {loop_s}: still playing after four hours"
            );
        }
    }
}

#[test]
fn too_few_layers_to_arrange_are_named() {
    // Two voices with no bed, or one over a bed: every loop is the full
    // mix, and the developer is told so.
    assert!(!Stems::new(4.0).voices(["a", "b"]).arranges());
    assert!(!Stems::new(4.0).bed(["p"]).voices(["a"]).arranges());
    assert!(Stems::new(4.0).voices(["a", "b", "c"]).arranges());
    assert!(Stems::new(4.0).bed(["p"]).voices(["a", "b"]).arranges());
    // And three voices are arranged differently from play to play.
    let stems = Stems::new(4.0).voices(["a", "b", "c"]);
    let shapes: BTreeSet<Vec<Vec<bool>>> = (0..64)
        .map(|s| {
            let mut plan = stems.plan(&mut Variety::new(s));
            plan.dedup();
            plan
        })
        .collect();
    assert!(shapes.len() > 1, "{shapes:?}");
}

#[test]
fn a_track_with_nothing_to_play_is_not_announced_as_playing() {
    let mut h = Headless::new(Default::default(), Probe::new("base"));
    h.app
        .ctx
        .soundtrack
        .set_tracks(vec![Track::stems("e", "Empty", Stems::new(4.0)).open()]);
    h.frame(0.0);
    h.app.ctx.push(SoundtrackScreen::new());
    h.frame(0.0);
    h.clear();
    h.press(Key::Return);
    h.frame(0.016);
    assert_eq!(h.spoken(), vec!["Can't play that track."]);
    assert!(h.played().is_empty());
}

#[test]
fn escape_during_the_closing_fade_cuts_it_short() {
    let mut h = game();
    h.app.ctx.audio.play_music(Some("music/menu"), true);
    h.app.ctx.soundtrack.set_tracks(vec![Track::stems(
        "s",
        "S",
        Stems::new(0.5).bed(["p"]).voices(["a", "b"]).length_s(1.0),
    )
    .open()]);
    h.frame(0.0);
    h.app.ctx.push(SoundtrackScreen::new());
    h.frame(0.0);
    h.press(Key::Return); // Rows: S, Back.
    let chans: Vec<Channel> = ["p", "a", "b"]
        .iter()
        .map(|k| h.sounds.channel_of(k).unwrap())
        .collect();
    for _ in 0..200 {
        h.frame(0.05);
        if ended(&h) {
            break;
        }
    }
    assert!(ended(&h), "the closing fade began");
    h.clear();
    h.press(Key::Escape);
    h.frame(0.0);
    let menu = h.app.ctx.audio.music_sound().unwrap();
    assert!(!h.app.ctx.audio.is_paused(menu));
    let cut: Vec<Channel> = h
        .sounds
        .calls()
        .iter()
        .filter_map(|c| match c {
            Call::Stop { channel, fade_ms } if *fade_ms <= 600 => Some(*channel),
            _ => None,
        })
        .collect();
    assert!(chans.iter().all(|c| cut.contains(c)), "{cut:?}");
}
