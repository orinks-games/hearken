//! The `Voice` service and `FakeVoice` in a headless game, with a stand-in
//! engine in place of Kokoro: no model, no network.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use hearken_app::audio::Call;
use hearken_app::screens::SettingsScreen;
use hearken_app::{
    Category, Ctx, EventPriority, Game, Headless, Key, Level, Line, NoAction, Rule, SettingsRow,
    State, Verbosity,
};
use hearken_voice::{
    settings_rows, Error, FakeVoice, HasVoiceSettings, Loader, Role, Speech, Synth, Voice,
    VoiceConfig, VoiceMode, VoiceSettings,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Cat {
    Call,
    Color,
}

impl Category for Cat {
    fn all() -> &'static [Self] {
        &[Cat::Call, Cat::Color]
    }

    fn id(self) -> &'static str {
        match self {
            Cat::Call => "call",
            Cat::Color => "color",
        }
    }
}

#[derive(Serialize, Deserialize, Default, Clone)]
struct Settings {
    voice: VoiceSettings,
}

impl HasVoiceSettings for Settings {
    fn voice_settings(&self) -> &VoiceSettings {
        &self.voice
    }

    fn voice_settings_mut(&mut self) -> &mut VoiceSettings {
        &mut self.voice
    }
}

const ROLES: &[Role] = &[
    Role {
        id: "pbp",
        label: "Play-by-play",
        default_voice: "bf_emma",
        sample: "Welcome in.",
    },
    Role {
        id: "color",
        label: "Color",
        default_voice: "am_michael",
        sample: "Big night tonight.",
    },
];

struct TestGame;

impl Game for TestGame {
    const TITLE: &'static str = "Voice test";
    const DATA_DIR: &'static str = "VoiceTest";
    const ENV_PREFIX: &'static str = "VOICE_TEST";
    type Event = ();
    type Data = ();
    type Settings = Settings;
    type Action = NoAction;
    type Category = Cat;

    fn verbosity() -> Verbosity<Cat> {
        Verbosity {
            levels: &[
                Level {
                    id: "full",
                    label: "Full",
                    rules: &[],
                },
                Level {
                    id: "essential",
                    label: "Essential",
                    rules: &[(Cat::Color, Rule::Silent)],
                },
            ],
        }
    }

    fn settings_rows(_ctx: &Ctx<Self>, mut rows: Vec<SettingsRow<Self>>) -> Vec<SettingsRow<Self>> {
        rows.extend(settings_rows::<Self>(ROLES));
        rows
    }
}

struct Blank;

impl State<TestGame> for Blank {
    fn name(&self) -> &'static str {
        "blank"
    }

    fn help(&self, _ctx: &Ctx<TestGame>) -> String {
        String::new()
    }
}

/// A stand-in engine: one second of audio per 20 characters, made only
/// while its gate is open.
#[derive(Clone)]
struct Stub {
    calls: Arc<AtomicUsize>,
    open: Arc<AtomicBool>,
    fail_load: bool,
}

impl Stub {
    fn new() -> Stub {
        Stub {
            calls: Arc::default(),
            open: Arc::new(AtomicBool::new(true)),
            fail_load: false,
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn voice(&self, h: &Headless<TestGame>) -> Voice<TestGame> {
        let stub = self.clone();
        Voice::with_loader(&h.app.ctx, VoiceConfig::new(ROLES), move |_, _| {
            let stub = stub.clone();
            let loader: Loader = Box::new(move || {
                if stub.fail_load {
                    return Err(Error("the model file is damaged".into()));
                }
                Ok(Box::new(stub) as Box<dyn Synth>)
            });
            Ok(loader)
        })
    }
}

impl Synth for Stub {
    fn synthesize(&mut self, text: &str, _voice: &str, _rate: f32) -> Result<Speech, Error> {
        while !self.open.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1));
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Speech {
            sample_rate: 1000,
            samples: vec![0.1; text.chars().count() * 50],
        })
    }

    fn speakers(&self) -> Vec<String> {
        vec!["bf_emma".into(), "am_michael".into()]
    }
}

fn headless() -> Headless<TestGame> {
    Headless::new((), Blank)
}

/// A game with the stub's voice installed.
fn with_voice(stub: &Stub) -> Headless<TestGame> {
    let mut h = headless();
    let voice = stub.voice(&h);
    h.app.ctx.set_commentary(Box::new(voice));
    h
}

/// Steps zero-length frames until `done`, giving the worker real time.
fn wait(h: &mut Headless<TestGame>, done: impl Fn(&Headless<TestGame>) -> bool) {
    for _ in 0..2000 {
        h.frame(0.0);
        if done(h) {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("timed out; transcript: {:?}", h.app.ctx.transcript);
}

fn voiced(h: &Headless<TestGame>) -> Vec<String> {
    h.app
        .ctx
        .transcript
        .iter()
        .filter_map(|l| l.strip_prefix("[voice] ").map(str::to_string))
        .collect()
}

fn voice_plays(h: &Headless<TestGame>) -> usize {
    h.played()
        .iter()
        .filter(|k| k.starts_with("voice:"))
        .count()
}

fn say(h: &mut Headless<TestGame>, speaker: &str, text: &str) {
    h.app.ctx.commentate(Line::new(speaker, text));
}

/// 20 characters: one second from the stub.
const SHORT: &str = "Tip-off at the Gard.";

#[test]
fn a_line_plays_on_the_voice_bus_in_the_speakers_voice() {
    let stub = Stub::new();
    let mut h = with_voice(&stub);
    say(&mut h, "pbp", SHORT);
    wait(&mut h, |h| voice_plays(h) == 1);
    assert_eq!(h.played(), vec!["voice:bf_emma".to_string()]);
    assert_eq!(voiced(&h), vec![SHORT.to_string()]);
    assert!(h.app.ctx.commentary_speaking());
    let speakers = h.app.ctx.commentary_speakers();
    assert_eq!(speakers, vec!["bf_emma", "am_michael"]);
}

#[test]
fn the_same_line_is_synthesized_once_and_said_once_per_moment() {
    let stub = Stub::new();
    let mut h = with_voice(&stub);
    say(&mut h, "pbp", SHORT);
    say(&mut h, "pbp", SHORT);
    wait(&mut h, |h| voice_plays(h) == 1);
    // Past the line and the repeat window: a new moment, from memory.
    h.frame(3.0);
    say(&mut h, "pbp", SHORT);
    assert_eq!(voice_plays(&h), 2, "a cached line plays the same frame");
    assert_eq!(stub.calls(), 1);
    // Another speaker's voice is another clip.
    h.frame(3.0);
    say(&mut h, "color", SHORT);
    wait(&mut h, |h| voice_plays(h) == 3);
    assert_eq!(stub.calls(), 2);
}

#[test]
fn the_disk_cache_serves_the_next_run() {
    let stub = Stub::new();
    let mut h = with_voice(&stub);
    say(&mut h, "pbp", SHORT);
    wait(&mut h, |h| voice_plays(h) == 1);
    let cache = h.app.ctx.paths.data_dir.join("voice-cache");
    assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 1);

    // A fresh service (a new run) over the same data folder.
    let fresh = Stub::new();
    let voice = fresh.voice(&h);
    h.app.ctx.set_commentary(Box::new(voice));
    h.frame(3.0);
    say(&mut h, "pbp", SHORT);
    wait(&mut h, |h| voice_plays(h) == 2);
    assert_eq!(fresh.calls(), 0);
}

#[test]
fn a_keyed_line_speaks_again_only_when_it_changes() {
    let stub = Stub::new();
    let mut h = with_voice(&stub);
    let score = |text: &str| Line::new("pbp", text).key("score");
    h.app.ctx.commentate(score("Hawks lead 2-0."));
    wait(&mut h, |h| voice_plays(h) == 1);
    h.frame(5.0);
    h.app.ctx.commentate(score("Hawks lead 2-0."));
    h.app.ctx.commentate(score("Hawks lead 4-0."));
    wait(&mut h, |h| voice_plays(h) == 2);
    assert_eq!(voiced(&h), vec!["Hawks lead 2-0.", "Hawks lead 4-0."]);
}

#[test]
fn a_line_still_synthesizing_past_its_moment_is_dropped() {
    let stub = Stub::new();
    stub.open.store(false, Ordering::SeqCst);
    let mut h = with_voice(&stub);
    say(&mut h, "pbp", SHORT);
    h.app
        .ctx
        .commentate(Line::new("pbp", "Timeout on the floor.").priority(EventPriority::Important));
    // Five seconds pass: past the ambient wait (4 s), within the important one (8 s).
    h.frame(5.0);
    stub.open.store(true, Ordering::SeqCst);
    wait(&mut h, |h| voice_plays(h) == 1);
    assert_eq!(voiced(&h), vec!["Timeout on the floor."]);
    assert!(h.app.ctx.transcript.iter().all(|l| !l.contains(SHORT)));
}

#[test]
fn lines_wait_their_turn() {
    let stub = Stub::new();
    let mut h = with_voice(&stub);
    say(&mut h, "pbp", SHORT);
    say(&mut h, "color", "He's on fire tonight.");
    wait(&mut h, |h| voice_plays(h) == 1);
    // The second is ready but waits for the first's second to pass.
    std::thread::sleep(Duration::from_millis(50));
    h.frame(0.0);
    assert_eq!(voice_plays(&h), 1);
    h.frame(0.6);
    h.frame(0.6);
    // Frames update before the clock moves, so this one sees 1.2 s.
    h.frame(0.0);
    assert_eq!(h.played(), vec!["voice:bf_emma", "voice:am_michael"]);
}

#[test]
fn a_critical_line_cuts_off_the_voice_and_the_queue() {
    let stub = Stub::new();
    let mut h = with_voice(&stub);
    say(
        &mut h,
        "pbp",
        "A long line about the history of the building and the team.",
    );
    say(&mut h, "color", "Queued colour that never gets said.");
    wait(&mut h, |h| voice_plays(h) == 1);
    h.app
        .ctx
        .commentate(Line::new("pbp", "Buzzer!").priority(EventPriority::Critical));
    assert!(h
        .sounds
        .calls()
        .iter()
        .any(|c| matches!(c, Call::Stop { .. })));
    wait(&mut h, |h| voice_plays(h) == 2);
    h.frame(10.0);
    h.frame(10.0);
    assert_eq!(
        voiced(&h),
        vec![
            "A long line about the history of the building and the team.",
            "Buzzer!"
        ]
    );
}

#[test]
fn f2_stops_the_voice_and_drops_the_queue() {
    let stub = Stub::new();
    let mut h = with_voice(&stub);
    say(&mut h, "pbp", SHORT);
    say(&mut h, "color", "He's on fire tonight.");
    wait(&mut h, |h| voice_plays(h) == 1);
    h.press(Key::F2);
    assert!(h
        .sounds
        .calls()
        .iter()
        .any(|c| matches!(c, Call::Stop { .. })));
    assert!(!h.app.ctx.commentary_speaking());
    for _ in 0..5 {
        std::thread::sleep(Duration::from_millis(20));
        h.frame(1.0);
    }
    assert_eq!(voice_plays(&h), 1);
}

#[test]
fn primed_lines_are_ready_without_being_said() {
    let stub = Stub::new();
    let mut h = with_voice(&stub);
    let lines = [
        Line::new("pbp", "Welcome to the Garden."),
        Line::new("color", "Big night.").terse("Big."),
    ];
    h.app.ctx.prime_commentary(&lines);
    wait(&mut h, |_| stub.calls() == 3);
    assert_eq!(voice_plays(&h), 0);
    // Give the replies a frame to land in memory.
    h.frame(0.0);
    h.app.ctx.commentate(lines[0].clone());
    assert_eq!(voice_plays(&h), 1, "a primed line plays at once");
    assert_eq!(stub.calls(), 3);
}

#[test]
fn the_mix_ducks_while_the_voice_talks() {
    let stub = Stub::new();
    let mut h = with_voice(&stub);
    h.app.ctx.settings.audio.duck_for_speech = true;
    say(&mut h, "pbp", SHORT);
    wait(&mut h, |h| voice_plays(h) == 1);
    assert!(h.app.ctx.speech_ducked());
    h.frame(0.5);
    assert!(h.app.ctx.speech_ducked());
    for _ in 0..4 {
        h.frame(0.5);
    }
    assert!(!h.app.ctx.speech_ducked());
}

#[test]
fn a_model_that_fails_to_load_falls_back_to_the_screen_reader() {
    let mut stub = Stub::new();
    stub.fail_load = true;
    let mut h = with_voice(&stub);
    wait(&mut h, |h| h.app.ctx.commentary_status().is_some());
    let status = h.app.ctx.commentary_status().unwrap();
    assert!(status.contains("the model file is damaged"), "{status}");
    say(&mut h, "pbp", SHORT);
    assert_eq!(h.speech.borrow().event_lines(), vec![SHORT.to_string()]);
    assert_eq!(voice_plays(&h), 0);
}

#[test]
fn lines_queued_while_loading_fall_back_when_it_fails() {
    let mut stub = Stub::new();
    stub.fail_load = true;
    let mut h = headless();
    // Not yet drained: the line is queued behind the load.
    let voice = stub.voice(&h);
    h.app.ctx.set_commentary(Box::new(voice));
    say(&mut h, "pbp", SHORT);
    wait(&mut h, |h| !h.speech.borrow().event_lines().is_empty());
    assert_eq!(h.speech.borrow().event_lines(), vec![SHORT.to_string()]);
}

#[test]
fn without_the_package_settings_say_why() {
    let mut h = headless();
    let voice = Voice::new(&h.app.ctx, VoiceConfig::new(ROLES));
    h.app.ctx.set_commentary(Box::new(voice));
    let status = h.app.ctx.commentary_status().expect("a reason");
    assert!(
        status == hearken_voice::NOT_INSTALLED || status == hearken_voice::NO_MODEL_IN_BUILD,
        "{status}"
    );
    say(&mut h, "pbp", SHORT);
    assert_eq!(h.speech.borrow().event_lines(), vec![SHORT.to_string()]);

    h.app.ctx.push(SettingsScreen::new());
    h.frame(0.0);
    let lines = h.app.top_lines().join("\n");
    assert!(
        lines.contains("Commentary voice: AI voices, not available, using the screen reader"),
        "{lines}"
    );
    assert!(lines.contains("Play-by-play voice: bf_emma"), "{lines}");
    assert!(lines.contains("Color voice: am_michael"), "{lines}");
    assert!(lines.contains("Commentary speed: 100 percent"), "{lines}");
}

#[test]
fn the_screen_reader_setting_skips_the_model() {
    let stub = Stub::new();
    let mut h = with_voice(&stub);
    h.app.ctx.settings.game.voice.mode = VoiceMode::ScreenReader;
    say(&mut h, "pbp", SHORT);
    assert_eq!(h.speech.borrow().event_lines(), vec![SHORT.to_string()]);
    h.app.ctx.settings.game.voice.mode = VoiceMode::Off;
    h.frame(3.0);
    say(&mut h, "pbp", "Something else.");
    for _ in 0..5 {
        std::thread::sleep(Duration::from_millis(5));
        h.frame(0.0);
    }
    assert_eq!(stub.calls(), 0);
    assert_eq!(voice_plays(&h), 0);
    assert_eq!(h.speech.borrow().event_lines().len(), 1);
}

#[test]
fn the_fake_records_lines_by_the_players_level() {
    let mut h = headless();
    let (fake, log) = FakeVoice::new();
    h.app.ctx.set_commentary(Box::new(fake));
    let call = Line::new("pbp", "Count it!").category(Cat::Call);
    let color = Line::new("color", "What a move.").category(Cat::Color);
    h.app.ctx.commentate(call.clone());
    h.app.ctx.commentate(color.clone());
    assert_eq!(log.texts(), vec!["Count it!", "What a move."]);
    assert_eq!(log.voiced()[1].speaker, "color");
    assert_eq!(log.voiced()[1].category, Some("color"));
    assert_eq!(voiced(&h), vec!["Count it!", "What a move."]);

    log.clear();
    h.app.ctx.settings.speech.verbosity = "essential".into();
    h.app.ctx.commentate(call.clone());
    h.app.ctx.commentate(color.clone());
    assert_eq!(log.texts(), vec!["Count it!"], "essential silences colour");

    log.clear();
    h.app.ctx.settings.game.voice.mode = VoiceMode::ScreenReader;
    h.app.ctx.commentate(call.clone());
    assert!(log.texts().is_empty());
    assert_eq!(
        h.speech.borrow().event_lines(),
        vec!["Count it!".to_string()]
    );

    h.app.ctx.prime_commentary(&[call]);
    assert_eq!(log.primed(), vec!["Count it!"]);
    h.press(Key::F2);
    assert_eq!(log.stops(), 1);
}

#[test]
fn with_no_commentary_lines_go_to_the_event_voice() {
    let mut h = headless();
    h.app
        .ctx
        .commentate(Line::new("pbp", "Count it!").category(Cat::Call));
    assert_eq!(
        h.speech.borrow().event_lines(),
        vec!["Count it!".to_string()]
    );
}
