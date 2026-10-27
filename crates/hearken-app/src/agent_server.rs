//! `--agent-server`: an MCP server inside the real game, so an agent can
//! play it the way a player does -- keys in, ears out -- while a person
//! listens. Built only with the `agent-server` feature, which no game gets
//! unless it asks; `hearken release` leaves that feature out of the zip
//! players get; and even with it on, only a debug build starts the server.
//!
//! The agent gets the real window loop, real time, real speech (heard on
//! the operator's screen reader) and real sound. Its keys go in as the
//! window's own key events, before the frame, so they pass every screen's
//! normal input handling. What it hears is every spoken line (the game's
//! transcript) and every sound that starts, with where it plays. One
//! inspector tool, `screen`, reads the top screen's text lines; an agent
//! that needs it to play has found something a player cannot hear, and
//! that is a finding.
//!
//! Tools: `listen`, `screen`, `press` (with `times`, and `settle: 0` to
//! reply at once for timing-critical play), `hold`, `release`, `wait`,
//! `wait_for`, `lockstep`, `record_start`, `record_stop` and `quit_game`.
//!
//! `lockstep` (on) holds game time still between tool calls, the way
//! Freight Fate's does. An agent's round trip takes seconds, longer than
//! most reaction windows; in lockstep the game waits while it thinks, so a
//! cue heard in a `wait_for` reply can still be answered in time. Time
//! passes only inside a call: the frames a `press`, `hold` or `release`
//! takes, its settle, and `wait` and `wait_for`. Between calls every frame
//! runs with no time passing: the window answers, a person's keys still
//! land, but no timer moves. Sound already playing plays on, as it does in
//! Freight Fate; the game's own clock is what waits. Off is the default, and
//! what a person playing alongside needs.
//!
//! `record_start` keeps a playtest as two files side by side: a WAV of the
//! final mix, tapped after every bus and effect (Kira backend only; another
//! backend gets the log alone), and a log, timed from the same start, of
//! every spoken line, every sound with its position, and every key, the
//! agent's and any a person pressed. They go where `path` says, or under a
//! dated name in a `recordings` folder beside the game. `record_stop`
//! finishes both and replies with their paths; closing the game does too.
//!
//! Saves go to a sandbox folder (`agent-sandbox` under the game's data
//! folder), and the session holds its own single-instance lock, so the
//! agent never touches a player's settings or saves and never fights a
//! player's copy of the game.
//!
//! Transport: newline-delimited JSON-RPC 2.0 over stdio. `initialize`,
//! `tools/list` and `ping` are answered before any window opens: a client
//! spawns every server it knows just to list its tools, and that must not
//! launch a game. The first tool call boots it. Stdin closing ends the game.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::app::App;
use crate::bindings::Chord;
use crate::game::Game;
use crate::input::{InputEvent, Key};

mod heard;
mod record;

/// One `wait` or `wait_for` may hold the game for at most this long.
const MAX_WAIT_SECONDS: f64 = 300.0;
/// After a press, how long the game runs before the reply, so the reply
/// carries what the press did.
const PRESS_SETTLE_SECONDS: f64 = 0.6;
/// Presses one `press` call may make.
const MAX_REPEAT: u32 = 50;

/// Whether this run was asked to be an agent server: a debug build only,
/// launched with `--agent-server`. A release build never starts it, even
/// with the feature on.
pub fn asked() -> bool {
    cfg!(debug_assertions) && std::env::args().any(|a| a == "--agent-server")
}

type Out = Arc<Mutex<Box<dyn Write + Send>>>;

fn send(out: &Out, msg: &Value) -> bool {
    let mut out = out.lock().unwrap_or_else(|p| p.into_inner());
    writeln!(out, "{msg}").and_then(|_| out.flush()).is_ok()
}

fn reply(out: &Out, id: &Value, text: String, is_error: bool) -> bool {
    send(
        out,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "content": [{ "type": "text", "text": text }], "isError": is_error },
        }),
    )
}

/// What the agent asked the game to do.
#[derive(Debug, Clone, PartialEq)]
enum Ask {
    Listen,
    Screen,
    Press {
        chord: Chord,
        times: u32,
        /// Seconds the game runs after the key comes up, before the reply.
        settle: f64,
    },
    Hold(Chord),
    Release(Chord),
    Wait(f64),
    WaitFor {
        text: String,
        seconds: f64,
    },
    Quit,
    /// Hold game time still between calls (on) or let it run (off).
    Lockstep(bool),
    /// Start recording, to this path or a dated default.
    RecordStart(Option<String>),
    RecordStop,
}

enum Msg {
    Call(Value, Ask),
    /// Stdin closed: the client is gone.
    Hangup,
}

fn tools() -> Value {
    let key = json!({ "type": "string", "description": "A key as the settings file names it, with optional ctrl+, alt+, shift+: enter, escape, up, down, left, right, space, tab, a-z, 0-9, f1-f12, page_up, home, ... e.g. \"ctrl+up\"." });
    json!([
        { "name": "listen", "description": "Everything said (and every sound started) since the last call, without touching the game.", "inputSchema": { "type": "object", "properties": {} } },
        { "name": "screen", "description": "Inspector, not ears: the top screen's name and its text lines. Record anything you needed here that was never spoken.", "inputSchema": { "type": "object", "properties": {} } },
        { "name": "press", "description": "Press and release a key (times: repeat count, default 1), then return what was heard. settle: seconds to keep listening after the key comes up (default 0.6; 0 replies at once, for timing-critical play).", "inputSchema": { "type": "object", "properties": { "key": key, "times": { "type": "integer", "minimum": 1, "maximum": MAX_REPEAT }, "settle": { "type": "number", "minimum": 0, "maximum": MAX_WAIT_SECONDS } }, "required": ["key"] } },
        { "name": "hold", "description": "Hold a key down until release.", "inputSchema": { "type": "object", "properties": { "key": key }, "required": ["key"] } },
        { "name": "release", "description": "Let a held key go.", "inputSchema": { "type": "object", "properties": { "key": key }, "required": ["key"] } },
        { "name": "wait", "description": "Let the game run for some seconds, then return what was heard.", "inputSchema": { "type": "object", "properties": { "seconds": { "type": "number", "minimum": 0, "maximum": MAX_WAIT_SECONDS } }, "required": ["seconds"] } },
        { "name": "wait_for", "description": "Run until a spoken line contains text (case-insensitive), or seconds pass.", "inputSchema": { "type": "object", "properties": { "text": { "type": "string" }, "seconds": { "type": "number", "minimum": 0, "maximum": MAX_WAIT_SECONDS } }, "required": ["text"] } },
        { "name": "lockstep", "description": "Hold game time still between tool calls (on: true), so the game waits while you think and a cue can be answered in time however long the round trip. Time then passes only inside a call: the frames a press, hold or release takes and its settle, and wait and wait_for. Use press with settle 0 to answer a cue and freeze again at once. Sound already playing plays on. Off (false), the default, runs on the wall clock, which a person playing alongside needs.", "inputSchema": { "type": "object", "properties": { "on": { "type": "boolean" } }, "required": ["on"] } },
        { "name": "record_start", "description": "Record the playtest: a WAV of what the player hears, and beside it a log, timed from the same start, of every spoken line, every sound (with where it played) and every key, the agent's and a person's. path: where (its extension is replaced); default a dated name in a recordings folder beside the game.", "inputSchema": { "type": "object", "properties": { "path": { "type": "string" } } } },
        { "name": "record_stop", "description": "Stop recording and finish both files; replies with their paths.", "inputSchema": { "type": "object", "properties": {} } },
        { "name": "quit_game", "description": "Close the game and end this server.", "inputSchema": { "type": "object", "properties": {} } }
    ])
}

fn parse_chord(args: &Value) -> Result<Chord, String> {
    let name = args["key"].as_str().ok_or("key is required")?;
    Chord::parse(&name.trim().to_ascii_lowercase()).ok_or_else(|| format!("unknown key: {name}"))
}

fn seconds(args: &Value, default: f64) -> f64 {
    args["seconds"]
        .as_f64()
        .unwrap_or(default)
        .clamp(0.0, MAX_WAIT_SECONDS)
}

fn parse_call(params: &Value) -> Result<Ask, String> {
    let args = &params["arguments"];
    Ok(match params["name"].as_str().unwrap_or("") {
        "listen" => Ask::Listen,
        "screen" => Ask::Screen,
        "press" => Ask::Press {
            chord: parse_chord(args)?,
            times: args["times"]
                .as_u64()
                .unwrap_or(1)
                .clamp(1, MAX_REPEAT as u64) as u32,
            settle: args["settle"]
                .as_f64()
                .unwrap_or(PRESS_SETTLE_SECONDS)
                .clamp(0.0, MAX_WAIT_SECONDS),
        },
        "hold" => Ask::Hold(parse_chord(args)?),
        "release" => Ask::Release(parse_chord(args)?),
        "wait" => Ask::Wait(seconds(args, 1.0)),
        "wait_for" => Ask::WaitFor {
            text: args["text"]
                .as_str()
                .ok_or("text is required")?
                .to_lowercase(),
            seconds: seconds(args, 30.0),
        },
        "quit_game" => Ask::Quit,
        "lockstep" => Ask::Lockstep(
            args["on"]
                .as_bool()
                .ok_or("on (true or false) is required")?,
        ),
        "record_start" => Ask::RecordStart(args["path"].as_str().map(String::from)),
        "record_stop" => Ask::RecordStop,
        other => return Err(format!("unknown tool: {other}")),
    })
}

/// Answers the handshake itself; queues tool calls for the game.
fn serve(input: Box<dyn BufRead + Send>, out: Out, calls: mpsc::Sender<Msg>) {
    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            send(
                &out,
                &json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "parse error" } }),
            );
            continue;
        };
        // A notification has no id and wants no answer.
        let Some(id) = msg.get("id").cloned() else {
            continue;
        };
        let answer = match msg["method"].as_str().unwrap_or("") {
            "initialize" => json!({
                "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "hearken-agent", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "A Hearken game, played by ear. The first tool call opens the real game window, with speech and sound the operator hears. Keys go in through press, hold and release; listen, wait and wait_for return what was said and which sounds started. For reaction-time play, turn lockstep on: game time then waits between calls.",
            }),
            "tools/list" => json!({ "tools": tools() }),
            "ping" => json!({}),
            "tools/call" => {
                match parse_call(&msg["params"]) {
                    Ok(ask) => {
                        if calls.send(Msg::Call(id, ask)).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        reply(&out, &id, e, true);
                    }
                }
                continue;
            }
            method => {
                send(
                    &out,
                    &json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": format!("no method {method}") } }),
                );
                continue;
            }
        };
        if !send(
            &out,
            &json!({ "jsonrpc": "2.0", "id": id, "result": answer }),
        ) {
            break;
        }
    }
    let _ = calls.send(Msg::Hangup);
}

/// The call the game is working on.
struct Job {
    id: Value,
    ask: Ask,
    /// Seconds of game left before the reply.
    left: f64,
    /// Presses still to send, and whether the last one's key is still down.
    presses: u32,
    down: bool,
    /// A `wait_for` whose text has been heard.
    found: bool,
}

/// The game side of the server: stepped once a frame by `run`.
pub struct AgentServer {
    out: Out,
    calls: Receiver<Msg>,
    queue: VecDeque<(Value, Ask)>,
    job: Option<Job>,
    /// What was heard since the last reply.
    heard: heard::Heard,
    /// `Ctx::said` when last read.
    said: u64,
    recording: Option<record::Recording>,
    /// Hold game time still between calls.
    lockstep: bool,
    /// Seconds of real time since the game opened. The recording log runs
    /// on it, as the WAV does, so the two stay aligned even when lockstep
    /// holds the game's own clock.
    wall: f64,
}

impl AgentServer {
    /// Starts serving stdio and waits for the first tool call, which is
    /// when the game should open. `None` when the client left first.
    pub fn start() -> Option<AgentServer> {
        AgentServer::over(
            Box::new(BufReader::new(std::io::stdin())),
            Box::new(std::io::stdout()),
        )
    }

    /// `start`, over any line source and sink: stdio for a real session, a
    /// pipe in tests.
    fn over(input: Box<dyn BufRead + Send>, output: Box<dyn Write + Send>) -> Option<AgentServer> {
        let out: Out = Arc::new(Mutex::new(output));
        let (tx, calls) = mpsc::channel();
        let serve_out = out.clone();
        std::thread::Builder::new()
            .name("agent-server".into())
            .spawn(move || serve(input, serve_out, tx))
            .ok()?;
        let first = match calls.recv() {
            Ok(Msg::Call(id, ask)) => (id, ask),
            _ => return None,
        };
        log::info!("agent server: first tool call, opening the game");
        Some(AgentServer {
            out,
            calls,
            queue: VecDeque::from([first]),
            job: None,
            heard: heard::Heard::default(),
            said: 0,
            recording: None,
            lockstep: false,
            wall: 0.0,
        })
    }

    /// Before the game runs: the sound tap.
    pub fn attach<G: Game>(&mut self, app: &mut App<G>) {
        app.ctx.audio.keep_heard();
    }

    /// One frame, before `App::step`: take new calls, gather what was heard
    /// last frame, add the agent's keys to `events`, and reply to a call
    /// that is done. `dt` is the frame's length; the return is how far the
    /// step moves the game on, which is 0 between calls in lockstep.
    pub fn frame<G: Game>(
        &mut self,
        app: &mut App<G>,
        events: &mut Vec<InputEvent>,
        dt: f64,
    ) -> f64 {
        self.wall += dt;
        let from_window = events.len();
        let working = self.step(app, events, dt);
        if let Some(rec) = &mut self.recording {
            for (i, e) in events.iter().enumerate() {
                rec.key(
                    self.wall,
                    if i < from_window { "player" } else { "agent" },
                    e,
                );
            }
        }
        if self.lockstep && !working {
            0.0
        } else {
            dt
        }
    }

    /// Works through the calls; true while one still needs game time this
    /// frame.
    fn step<G: Game>(&mut self, app: &mut App<G>, events: &mut Vec<InputEvent>, dt: f64) -> bool {
        loop {
            match self.calls.try_recv() {
                Ok(Msg::Call(id, ask)) => self.queue.push_back((id, ask)),
                Ok(Msg::Hangup) | Err(TryRecvError::Disconnected) => {
                    log::info!("agent server: the client left, closing the game");
                    app.ctx.running = false;
                    return false;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        self.gather(app);

        loop {
            if self.job.is_none() {
                let Some((id, ask)) = self.queue.pop_front() else {
                    return false;
                };
                self.job = self.begin(id, ask, app, events);
                continue;
            }
            let job = self.job.as_mut().expect("a job is running");
            if let Ask::Press { chord, .. } = &job.ask {
                let chord = *chord;
                if job.down {
                    events.push(InputEvent::KeyUp {
                        key: chord.key,
                        mods: chord.mods,
                    });
                    job.down = false;
                    return true;
                } else if job.presses > 0 {
                    events.push(key_down(chord));
                    job.presses -= 1;
                    job.down = true;
                    return true;
                }
            }
            // Judged before this frame runs, so the reply carries every
            // line the waited time produced, and in lockstep no time passes
            // after it.
            let done = match &job.ask {
                Ask::WaitFor { .. } => job.left <= 0.0 || job.found,
                _ => job.left <= 0.0,
            };
            if !done {
                job.left -= dt;
                return true;
            }
            let job = self.job.take().expect("a job is running");
            let mut text = self.heard.take();
            if let Ask::WaitFor { text: want, .. } = &job.ask {
                if !job.found {
                    text = format!("(not heard: \"{want}\")\n{text}");
                }
            }
            if text.is_empty() {
                text = "(silence)".into();
            }
            if !reply(&self.out, &job.id, text, false) {
                app.ctx.running = false;
                return false;
            }
        }
    }

    /// Starts a call. One that needs no game time (listen, screen, quit)
    /// replies now and leaves no job.
    fn begin<G: Game>(
        &mut self,
        id: Value,
        ask: Ask,
        app: &mut App<G>,
        events: &mut Vec<InputEvent>,
    ) -> Option<Job> {
        let mut job = Job {
            id,
            ask: ask.clone(),
            left: PRESS_SETTLE_SECONDS,
            presses: 0,
            down: false,
            found: false,
        };
        match ask {
            Ask::Listen => job.left = 0.0,
            Ask::Screen => {
                let mut text = format!("screen: {}", app.top_name());
                for line in app.top_lines() {
                    text.push('\n');
                    text.push_str(&line);
                }
                reply(&self.out, &job.id, text, false);
                return None;
            }
            Ask::Quit => {
                app.ctx.running = false;
                reply(&self.out, &job.id, "(quitting)".into(), false);
                return None;
            }
            Ask::Lockstep(on) => {
                self.lockstep = on;
                let text = if on {
                    "Lockstep is on: game time waits between calls. It passes only inside press, hold, release, wait and wait_for."
                } else {
                    "Lockstep is off: the game runs on the wall clock again."
                };
                reply(&self.out, &job.id, text.into(), false);
                return None;
            }
            Ask::RecordStart(path) => {
                let (text, failed) = if self.recording.is_some() {
                    ("already recording; record_stop first".to_string(), true)
                } else {
                    match record::Recording::start(&mut app.ctx, path.as_deref(), self.wall) {
                        Ok((rec, text)) => {
                            self.recording = Some(rec);
                            (text, false)
                        }
                        Err(e) => (e, true),
                    }
                };
                reply(&self.out, &job.id, text, failed);
                return None;
            }
            Ask::RecordStop => {
                let text = match self.recording.take() {
                    Some(rec) => rec.stop(self.wall, Some(&mut app.ctx.audio)),
                    None => "not recording".to_string(),
                };
                reply(&self.out, &job.id, text, false);
                return None;
            }
            Ask::Press { times, settle, .. } => {
                job.presses = times;
                job.left = settle;
            }
            Ask::Hold(chord) => events.push(key_down(chord)),
            Ask::Release(chord) => events.push(InputEvent::KeyUp {
                key: chord.key,
                mods: chord.mods,
            }),
            Ask::Wait(s) => job.left = s,
            Ask::WaitFor { seconds, ref text } => {
                job.left = seconds;
                job.found = self.heard.contains(text);
            }
        }
        Some(job)
    }

    fn gather<G: Game>(&mut self, app: &mut App<G>) {
        let ctx = &mut app.ctx;
        let new = (ctx.said - self.said).min(ctx.transcript.len() as u64) as usize;
        self.said = ctx.said;
        let start = ctx.transcript.len() - new;
        let sounds = ctx.audio.take_heard();
        if let Some(rec) = &mut self.recording {
            for line in &ctx.transcript[start..] {
                rec.said(self.wall, line);
            }
            for s in &sounds {
                rec.sound(self.wall, s);
            }
        }
        let want = match &self.job {
            Some(Job {
                ask: Ask::WaitFor { text, .. },
                found: false,
                ..
            }) => Some(text.clone()),
            _ => None,
        };
        let lines = ctx.transcript[start..].iter().cloned().chain(
            sounds
                .iter()
                .map(|s| format!("sound: {}", record::describe_sound(s))),
        );
        for line in lines {
            if want
                .as_deref()
                .is_some_and(|w| line.to_lowercase().contains(w))
            {
                if let Some(job) = &mut self.job {
                    job.found = true;
                }
            }
            self.heard.push(line);
        }
    }

    /// The game is closing: finish a recording, and answer whatever is
    /// still waiting.
    pub fn finish<G: Game>(&mut self, app: &mut App<G>) {
        if let Some(rec) = self.recording.take() {
            rec.stop(self.wall, Some(&mut app.ctx.audio));
        }
        if let Some(job) = self.job.take() {
            let text = self.heard.take();
            reply(
                &self.out,
                &job.id,
                format!("{text}\n(the game closed)"),
                false,
            );
        }
        for (id, _) in self.queue.drain(..) {
            reply(&self.out, &id, "(the game closed)".into(), true);
        }
    }
}

fn key_down(chord: Chord) -> InputEvent {
    let plain = !chord.mods.ctrl && !chord.mods.alt;
    let text = crate::bindings::key_saved_name(chord.key)
        .filter(|n| plain && n.len() == 1)
        .and_then(|n| n.chars().next())
        .map(|c| {
            if chord.mods.shift {
                c.to_ascii_uppercase()
            } else {
                c
            }
        })
        .or((plain && chord.key == Key::Space).then_some(' '));
    InputEvent::KeyDown {
        key: chord.key,
        mods: chord.mods,
        text,
        repeat: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_parse_into_asks() {
        let p = json!({ "name": "press", "arguments": { "key": "Ctrl+Up", "times": 3 } });
        let Ask::Press { chord, times, .. } = parse_call(&p).unwrap() else {
            panic!("a press")
        };
        assert_eq!((chord.key, chord.mods.ctrl, times), (Key::Up, true, 3));
        let w = json!({ "name": "wait_for", "arguments": { "text": "Night ONE", "seconds": 999 } });
        assert_eq!(
            parse_call(&w).unwrap(),
            Ask::WaitFor {
                text: "night one".into(),
                seconds: MAX_WAIT_SECONDS
            }
        );
        assert!(parse_call(&json!({ "name": "press", "arguments": { "key": "nope" } })).is_err());
        assert!(parse_call(&json!({ "name": "fly" })).is_err());
    }

    #[test]
    fn a_letter_press_types_its_letter() {
        let down = key_down(Chord::parse("shift+a").unwrap());
        assert!(matches!(
            down,
            InputEvent::KeyDown {
                text: Some('A'),
                ..
            }
        ));
        let ctrl = key_down(Chord::parse("ctrl+a").unwrap());
        assert!(matches!(ctrl, InputEvent::KeyDown { text: None, .. }));
    }

    #[test]
    fn lockstep_parses() {
        let on = json!({ "name": "lockstep", "arguments": { "on": true } });
        assert_eq!(parse_call(&on).unwrap(), Ask::Lockstep(true));
        assert!(parse_call(&json!({ "name": "lockstep", "arguments": {} })).is_err());
    }
}

/// The server driven end to end over a pipe, against a headless game with
/// a reaction window.
#[cfg(all(test, feature = "testing"))]
mod lockstep_tests {
    use super::*;
    use crate::ctx::Ctx;
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
    #[derive(Default)]
    struct Window {
        clock: f64,
        over: bool,
    }

    const CUE_AT: f64 = 0.5;
    const WINDOW: f64 = 0.3;

    impl State<Reflex> for Window {
        fn name(&self) -> &'static str {
            "window"
        }
        fn help(&self, _ctx: &Ctx<Reflex>) -> String {
            String::new()
        }
        fn update(&mut self, ctx: &mut Ctx<Reflex>, dt: f64) {
            if self.over {
                return;
            }
            let before = self.clock;
            self.clock += dt;
            if before < CUE_AT && self.clock >= CUE_AT {
                ctx.say("Now!");
            }
            if self.clock >= CUE_AT + WINDOW {
                self.over = true;
                ctx.say("Missed.");
            }
        }
        fn handle_input(&mut self, ctx: &mut Ctx<Reflex>, event: &InputEvent) {
            if !self.over && event.pressed() == Some(Key::Space) {
                self.over = true;
                ctx.say(if self.clock >= CUE_AT {
                    "Hit!"
                } else {
                    "Too soon."
                });
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
}
