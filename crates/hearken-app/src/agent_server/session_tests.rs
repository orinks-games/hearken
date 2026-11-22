//! The server driven end to end over a pipe, against a headless game with
//! a reaction window.

use super::*;
use crate::ctx::Ctx;
use crate::phase::{Phase, Phases};
use crate::shell::FRAME_SECONDS;
use crate::state::State;
use crate::testing::Headless;
use std::io::PipeWriter;
use std::time::{Duration, Instant};

struct Reflex;

impl Game for Reflex {
    const TITLE: &'static str = "Reflex";
    const DATA_DIR: &'static str = "HearkenReflexTest";
    const ENV_PREFIX: &'static str = "HEARKEN_REFLEX_TEST";
    type Event = ();
    type Data = ();
    type Settings = ();
    type Action = crate::bindings::NoAction;
    type Category = hearken_speech::NoCategory;
}

/// "Now!" after half a second, then a third of a second to press space.
/// Its phases are explicit, as a game's should be (see `phase`).
struct Window {
    clock: f64,
    phases: Phases<Cue>,
}

impl Default for Window {
    fn default() -> Self {
        Window {
            clock: 0.0,
            phases: Phases::new(Cue::Waiting),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cue {
    /// Before the cue: a press is too soon.
    Waiting,
    /// The window after the cue: a press hits.
    Open,
    /// Answered or missed.
    Done,
}

impl Phase for Cue {
    fn name(self) -> &'static str {
        match self {
            Cue::Waiting => "waiting",
            Cue::Open => "open",
            Cue::Done => "done",
        }
    }
}

const CUE_AT: f64 = 0.5;
const WINDOW: f64 = 0.3;

impl State<Reflex> for Window {
    fn name(&self) -> &'static str {
        "window"
    }
    fn phase(&self) -> Option<&'static str> {
        Some(self.phases.name())
    }
    fn help(&self, _ctx: &Ctx<Reflex>) -> String {
        String::new()
    }
    fn update(&mut self, ctx: &mut Ctx<Reflex>, dt: f64) {
        if self.phases.is(Cue::Done) {
            return;
        }
        self.clock += dt;
        // The window closes at a fixed moment, however late the frame
        // that opened it landed.
        if self.phases.is(Cue::Waiting) && self.clock >= CUE_AT {
            self.phases
                .set_for(ctx, Cue::Open, CUE_AT + WINDOW - self.clock);
            ctx.say("Now!");
        }
        if self.phases.tick(dt) && self.phases.set(ctx, Cue::Done) {
            ctx.say("Missed.");
        }
    }
    fn handle_input(&mut self, ctx: &mut Ctx<Reflex>, event: &InputEvent) {
        if event.pressed() != Some(Key::Space) {
            return;
        }
        let hit = self.phases.is(Cue::Open);
        if self.phases.set(ctx, Cue::Done) {
            ctx.say(if hit { "Hit!" } else { "Too soon." });
        }
    }
}

/// What the server writes, kept for the test to read.
#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Sink {
    fn replies(&self) -> Vec<Value> {
        String::from_utf8_lossy(&self.0.lock().unwrap())
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }
}

struct Session {
    game: Headless<Reflex>,
    agent: AgentServer,
    to_server: PipeWriter,
    sink: Sink,
    next_id: u64,
}

fn call_line(id: u64, name: &str, args: Value) -> String {
    format!(
        "{}\n",
        json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": name, "arguments": args } })
    )
}

impl Session {
    /// The game opens on the first call, as a real session does.
    fn open(name: &str, args: Value) -> (Session, String) {
        Session::open_with(name, args, |_| {})
    }

    /// `open`, with `setup` run on the game before its first frame.
    fn open_with(
        name: &str,
        args: Value,
        setup: impl FnOnce(&mut App<Reflex>),
    ) -> (Session, String) {
        let (from_client, mut to_server) = std::io::pipe().unwrap();
        let sink = Sink::default();
        to_server
            .write_all(call_line(1, name, args).as_bytes())
            .unwrap();
        let agent = AgentServer::over(
            Box::new(BufReader::new(from_client)),
            Box::new(sink.clone()),
        )
        .expect("the first call opens the game");
        let mut session = Session {
            game: Headless::new((), Window::default()),
            agent,
            to_server,
            sink,
            next_id: 2,
        };
        setup(&mut session.game.app);
        session.agent.attach(&mut session.game.app);
        let text = session.until_reply(1);
        (session, text)
    }

    /// One frame of the real loop: the server, then the game.
    fn frame(&mut self) {
        let mut events = Vec::new();
        let dt = self
            .agent
            .frame(&mut self.game.app, &mut events, FRAME_SECONDS);
        self.game.app.step(events, dt);
    }

    /// Frames with no call in flight: an agent thinking.
    fn think(&mut self, seconds: f64) {
        for _ in 0..(seconds / FRAME_SECONDS).round() as u32 {
            self.frame();
        }
    }

    fn until_reply(&mut self, id: u64) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(r) = self.sink.replies().into_iter().find(|r| r["id"] == id) {
                return r["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
            }
            assert!(Instant::now() < deadline, "no reply to call {id}");
            // Let the serve thread hand the call over before a frame
            // runs, so a frame never passes waiting for the pipe.
            std::thread::sleep(Duration::from_millis(2));
            self.frame();
        }
    }

    fn call(&mut self, name: &str, args: Value) -> String {
        let id = self.next_id;
        self.next_id += 1;
        self.to_server
            .write_all(call_line(id, name, args).as_bytes())
            .unwrap();
        // The call is on its way; give it time to reach the channel so
        // no frame runs on the wall clock while it travels.
        std::thread::sleep(Duration::from_millis(20));
        self.until_reply(id)
    }

    fn clock(&self) -> f64 {
        self.game.app.ctx.clock
    }
}

#[test]
fn in_lockstep_a_slow_agent_still_answers_a_cue_in_time() {
    let (mut s, text) = Session::open("lockstep", json!({ "on": true }));
    assert!(text.starts_with("Lockstep is on"), "{text}");
    assert!(s
        .call("wait_for", json!({ "text": "now", "seconds": 5 }))
        .contains("Now!"));
    // Three seconds of thinking, ten times the window.
    let held = s.clock();
    s.think(3.0);
    assert_eq!(s.clock(), held, "game time held still between calls");
    let text = s.call("press", json!({ "key": "space", "settle": 0 }));
    assert!(text.contains("Hit!"), "{text}");
    // A press with settle 0 takes its two frames and no more.
    assert!((s.clock() - held - 2.0 * FRAME_SECONDS).abs() < 1e-9);
}

#[test]
fn without_lockstep_the_same_agent_is_too_late() {
    let (mut s, _) = Session::open("listen", json!({}));
    assert!(s
        .call("wait_for", json!({ "text": "now", "seconds": 5 }))
        .contains("Now!"));
    s.think(3.0);
    let text = s.call("press", json!({ "key": "space", "settle": 0 }));
    assert!(!text.contains("Hit!"), "{text}");
    assert!(s.game.app.ctx.transcript.iter().any(|l| l == "Missed."));
}

#[test]
fn a_wait_in_lockstep_runs_its_time_and_then_holds() {
    let (mut s, _) = Session::open("lockstep", json!({ "on": true }));
    let start = s.clock();
    s.call("wait", json!({ "seconds": 0.25 }));
    let after = s.clock();
    assert!(
        (after - start - 0.25).abs() < FRAME_SECONDS,
        "ran {}",
        after - start
    );
    s.think(1.0);
    assert_eq!(s.clock(), after);
    // Off again, the game runs between calls.
    s.call("lockstep", json!({ "on": false }));
    s.think(0.5);
    assert!(s.clock() > after + 0.4);
}

#[test]
fn the_screen_tool_names_the_phase_and_the_stack_and_listen_does_not() {
    let (mut s, text) = Session::open("screen", json!({}));
    assert!(
        text.starts_with("screen: window, phase: waiting\nstack: window"),
        "{text}"
    );
    let heard = s.call("wait_for", json!({ "text": "now", "seconds": 5 }));
    assert!(!heard.contains("[phase]"), "{heard}");
    let text = s.call("screen", json!({}));
    assert!(text.starts_with("screen: window, phase: open"), "{text}");
    assert_eq!(
        s.game.transitions(),
        ["window: waiting -> open"],
        "{:?}",
        s.game.changes()
    );
}

#[test]
fn a_person_can_still_play_while_the_game_is_held() {
    let (mut s, _) = Session::open("lockstep", json!({ "on": true }));
    s.call("wait_for", json!({ "text": "now", "seconds": 5 }));
    let held = s.clock();
    // A key from the window lands on a held frame.
    let mut events = vec![InputEvent::key(Key::Space)];
    let dt = s.agent.frame(&mut s.game.app, &mut events, FRAME_SECONDS);
    assert_eq!(dt, 0.0);
    s.game.app.step(events, dt);
    assert_eq!(s.clock(), held);
    assert!(s.game.app.ctx.transcript.iter().any(|l| l == "Hit!"));
}

/// A sound device that can record, opened when the test lets it.
struct Recorder(Option<std::path::PathBuf>);

impl hearken_audio::Backend for Recorder {
    fn name(&self) -> String {
        "recorder".into()
    }
    fn play(
        &mut self,
        _key: &str,
        _source: hearken_audio::Source,
        _looping: bool,
        _mix: hearken_audio::Mix,
    ) -> Result<hearken_audio::Channel, hearken_audio::BackendError> {
        Ok(0)
    }
    fn set_mix(&mut self, _: hearken_audio::Channel, _: hearken_audio::Mix, _: u32) {}
    fn stop(&mut self, _: hearken_audio::Channel, _: u32) {}
    fn pause(&mut self, _: hearken_audio::Channel, _: bool) {}
    fn is_playing(&mut self, _: hearken_audio::Channel) -> bool {
        false
    }
    fn start_capture(&mut self, path: &std::path::Path) -> Result<(), hearken_audio::BackendError> {
        self.0 = Some(path.to_path_buf());
        Ok(())
    }
    fn stop_capture(&mut self) -> Option<Result<std::path::PathBuf, hearken_audio::BackendError>> {
        self.0.take().map(Ok)
    }
}

#[test]
fn a_recording_asked_for_first_waits_for_the_sound_device() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("take");
    let (open, opened) = std::sync::mpsc::channel::<()>();
    // The device opens a moment after the game does, as a real one does.
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        let _ = open.send(());
    });
    let (mut s, text) = Session::open_with(
        "record_start",
        json!({ "path": path.to_string_lossy() }),
        |app| {
            app.ctx.install_audio(hearken_audio::Audio::deferred(
                move || {
                    opened.recv().ok();
                    Ok(Box::new(Recorder(None)) as Box<dyn hearken_audio::Backend>)
                },
                Box::new(hearken_audio::MemoryClips::default()),
            ));
            assert!(app.ctx.audio.opening());
        },
    );
    assert!(text.contains("sound: "), "{text}");
    assert!(!text.contains("No sound file"), "{text}");
    let text = s.call("record_stop", json!({}));
    assert!(text.contains("take.wav"), "{text}");
}

#[test]
fn the_recording_log_keeps_up_with_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("take");
    let (mut s, _) = Session::open("record_start", json!({ "path": path.to_string_lossy() }));
    s.call("wait_for", json!({ "text": "now", "seconds": 5 }));
    // Read before record_stop: every line is already on disk.
    let log = std::fs::read_to_string(path.with_extension("log")).unwrap();
    assert!(log.contains("said: Now!"), "{log}");
    s.call("record_stop", json!({}));
}

#[test]
fn an_unknown_key_is_told_the_punctuation_names() {
    let (mut s, _) = Session::open("listen", json!({}));
    let text = s.call("press", json!({ "key": "'" }));
    assert!(text.contains("apostrophe"), "{text}");
    assert!(text.contains("semicolon"), "{text}");
    let text = s.call("press", json!({ "key": "apostrophe", "settle": 0 }));
    assert!(!text.contains("unknown key"), "{text}");
}
