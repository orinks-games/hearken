//! `--agent-server`: an MCP server inside the real game, so an agent can
//! play it the way a player does -- keys in, ears out -- while a person
//! listens. Built only with the `agent-server` feature; `hearken release`
//! leaves that feature out of the zip players get.
//!
//! The agent gets the real window loop, real time, real speech (heard on
//! the operator's screen reader) and real sound. Its keys go in as the
//! window's own key events, before the frame, so they pass every screen's
//! normal input handling. What it hears is every spoken line (the game's
//! transcript) and the key of every sound that starts. One inspector tool,
//! `screen`, reads the top screen's text lines; an agent that needs it to
//! play has found something a player cannot hear, and that is a finding.
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
use std::io::{BufRead, Write};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::app::App;
use crate::bindings::Chord;
use crate::game::Game;
use crate::input::{InputEvent, Key};

/// One `wait` or `wait_for` may hold the game for at most this long.
const MAX_WAIT_SECONDS: f64 = 300.0;
/// After a press, how long the game runs before the reply, so the reply
/// carries what the press did.
const PRESS_SETTLE_SECONDS: f64 = 0.6;
/// Presses one `press` call may make.
const MAX_REPEAT: u32 = 50;

/// Whether this run was asked to be an agent server.
pub fn asked() -> bool {
    std::env::args().any(|a| a == "--agent-server")
}

type Out = Arc<Mutex<std::io::Stdout>>;

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
    Press { chord: Chord, times: u32 },
    Hold(Chord),
    Release(Chord),
    Wait(f64),
    WaitFor { text: String, seconds: f64 },
    Quit,
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
        { "name": "press", "description": "Press and release a key (times: repeat count, default 1), then return what was heard.", "inputSchema": { "type": "object", "properties": { "key": key, "times": { "type": "integer", "minimum": 1, "maximum": MAX_REPEAT } }, "required": ["key"] } },
        { "name": "hold", "description": "Hold a key down until release.", "inputSchema": { "type": "object", "properties": { "key": key }, "required": ["key"] } },
        { "name": "release", "description": "Let a held key go.", "inputSchema": { "type": "object", "properties": { "key": key }, "required": ["key"] } },
        { "name": "wait", "description": "Let the game run for some seconds, then return what was heard.", "inputSchema": { "type": "object", "properties": { "seconds": { "type": "number", "minimum": 0, "maximum": MAX_WAIT_SECONDS } }, "required": ["seconds"] } },
        { "name": "wait_for", "description": "Run until a spoken line contains text (case-insensitive), or seconds pass.", "inputSchema": { "type": "object", "properties": { "text": { "type": "string" }, "seconds": { "type": "number", "minimum": 0, "maximum": MAX_WAIT_SECONDS } }, "required": ["text"] } },
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
        other => return Err(format!("unknown tool: {other}")),
    })
}

/// Answers the handshake itself; queues tool calls for the game.
fn serve(out: Out, calls: mpsc::Sender<Msg>) {
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
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
                "instructions": "A Hearken game, played by ear. The first tool call opens the real game window, with speech and sound the operator hears. Keys go in through press, hold and release; listen, wait and wait_for return what was said and which sounds started.",
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
}

/// The game side of the server: stepped once a frame by `run`.
pub struct AgentServer {
    out: Out,
    calls: Receiver<Msg>,
    queue: VecDeque<(Value, Ask)>,
    job: Option<Job>,
    /// What was heard since the last reply.
    heard: Vec<String>,
    /// `Ctx::said` when last read.
    said: u64,
}

impl AgentServer {
    /// Starts serving stdio and waits for the first tool call, which is
    /// when the game should open. `None` when the client left first.
    pub fn start() -> Option<AgentServer> {
        let out: Out = Arc::new(Mutex::new(std::io::stdout()));
        let (tx, calls) = mpsc::channel();
        let serve_out = out.clone();
        std::thread::Builder::new()
            .name("agent-server".into())
            .spawn(move || serve(serve_out, tx))
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
            heard: Vec::new(),
            said: 0,
        })
    }

    /// Before the game runs: the sound tap.
    pub fn attach<G: Game>(&mut self, app: &mut App<G>) {
        app.ctx.audio.keep_heard();
    }

    /// One frame, before `App::step`: take new calls, gather what was heard
    /// last frame, add the agent's keys to `events`, and reply to a call
    /// that is done. `dt` is last frame's length.
    pub fn frame<G: Game>(&mut self, app: &mut App<G>, events: &mut Vec<InputEvent>, dt: f64) {
        loop {
            match self.calls.try_recv() {
                Ok(Msg::Call(id, ask)) => self.queue.push_back((id, ask)),
                Ok(Msg::Hangup) | Err(TryRecvError::Disconnected) => {
                    log::info!("agent server: the client left, closing the game");
                    app.ctx.running = false;
                    return;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        self.gather(app);

        while self.job.is_none() {
            let Some((id, ask)) = self.queue.pop_front() else {
                break;
            };
            self.job = self.begin(id, ask, app, events);
        }
        let Some(job) = &mut self.job else { return };
        if let Ask::Press { chord, .. } = &job.ask {
            let chord = *chord;
            if job.down {
                events.push(InputEvent::KeyUp {
                    key: chord.key,
                    mods: chord.mods,
                });
                job.down = false;
            } else if job.presses > 0 {
                events.push(key_down(chord));
                job.presses -= 1;
                job.down = true;
            }
            if job.presses > 0 || job.down {
                return;
            }
        }
        job.left -= dt;
        let done = match &job.ask {
            Ask::WaitFor { text, .. } => {
                job.left <= 0.0 || self.heard.iter().any(|l| l.to_lowercase().contains(text))
            }
            _ => job.left <= 0.0,
        };
        if done {
            let job = self.job.take().expect("a job is running");
            let mut text = std::mem::take(&mut self.heard).join("\n");
            if let Ask::WaitFor { text: want, .. } = &job.ask {
                if !text.to_lowercase().contains(want.as_str()) {
                    text = format!("(not heard: \"{want}\")\n{text}");
                }
            }
            if text.is_empty() {
                text = "(silence)".into();
            }
            if !reply(&self.out, &job.id, text, false) {
                app.ctx.running = false;
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
            Ask::Press { times, .. } => job.presses = times,
            Ask::Hold(chord) => events.push(key_down(chord)),
            Ask::Release(chord) => events.push(InputEvent::KeyUp {
                key: chord.key,
                mods: chord.mods,
            }),
            Ask::Wait(s) => job.left = s,
            Ask::WaitFor { seconds, .. } => job.left = seconds,
        }
        Some(job)
    }

    fn gather<G: Game>(&mut self, app: &mut App<G>) {
        let ctx = &mut app.ctx;
        let new = (ctx.said - self.said).min(ctx.transcript.len() as u64) as usize;
        self.said = ctx.said;
        let start = ctx.transcript.len() - new;
        self.heard.extend(ctx.transcript[start..].iter().cloned());
        self.heard.extend(
            ctx.audio
                .take_heard()
                .into_iter()
                .map(|k| format!("sound: {k}")),
        );
    }

    /// The game is closing: answer whatever is still waiting.
    pub fn finish(&mut self) {
        if let Some(job) = self.job.take() {
            let text = std::mem::take(&mut self.heard).join("\n");
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
        let Ask::Press { chord, times } = parse_call(&p).unwrap() else {
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
}
