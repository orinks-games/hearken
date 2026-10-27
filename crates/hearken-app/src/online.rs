//! Verified online runs, the game's side: `ctx.online`.
//!
//! A game that offers online play names its server in `Game::online`. The
//! player connects an account once, from Settings: the game shows a short
//! code, the player enters it on the server's site, and the game keeps the
//! token it is given. After that, `ctx.online.submit(kind, &run)` sends a
//! recorded run (`hearken::replay::Recording`) in the background, and
//! `ctx.online.result(ticket)` hands back whatever the server's handler for
//! that kind answered: a rank, a shared total, nothing at all.
//!
//! Nothing here knows what a kind means. `docs/online.md` is the protocol a
//! server speaks; `server/` in the Hearken repository is one that runs on
//! Vercel, Netlify, Cloudflare Workers, Deno, Bun or plain Node.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hearken_replay::Run;

pub use crate::settings::{OnlineSettings, Secret};
use serde_json::{json, Value};

/// Where a game's server is, and what to call it out loud.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OnlineConfig {
    /// The service's base URL, such as `https://orinks.net/api/games/wickets`.
    /// HTTPS, except for a server on this computer (`http://localhost`,
    /// `http://127.0.0.1`).
    pub url: String,
    /// The site's name as the player hears it: "orinks.net".
    pub site: String,
}

impl OnlineConfig {
    pub fn new(url: impl Into<String>, site: impl Into<String>) -> Self {
        OnlineConfig {
            url: url.into(),
            site: site.into(),
        }
    }

    /// Whether the URL is one the game will send a token to.
    pub fn url_allowed(&self) -> bool {
        let url = self.url.as_str();
        let local = ["http://localhost", "http://127.0.0.1", "http://[::1]"]
            .iter()
            .any(|base| {
                url.strip_prefix(base)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/']))
            });
        let https = url
            .strip_prefix("https://")
            .is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'));
        (https || local) && !url.contains(['?', '#', '@', ' '])
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}/{path}", self.url.trim_end_matches('/'))
    }
}

/// What a request carries.
pub enum Body {
    Json(Value),
    /// A run file.
    Run(Vec<u8>),
}

/// What a server answered.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub status: u16,
    /// The JSON body, or `Null` when there was none.
    pub body: Value,
}

/// How requests reach the server. The engine's own speaks HTTPS; a test
/// swaps in one that answers by itself (`Online::set_transport`).
pub trait Transport: Send + Sync + 'static {
    /// POSTs `body` to `url`, with the token as `Authorization: Bearer`.
    /// `Err` only when no answer came at all (no network, a timeout).
    fn post(&self, url: &str, token: Option<&str>, body: Body) -> Result<Reply, String>;
}

/// The longest answer read from a server.
pub const MAX_REPLY_BYTES: u64 = 256 * 1024;

/// HTTPS through ureq. Redirects are not followed, so a token is only ever
/// sent to the URL the game names.
pub struct HttpTransport {
    agent: ureq::Agent,
}

impl Default for HttpTransport {
    fn default() -> Self {
        HttpTransport {
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .redirects(0)
                .user_agent(concat!("hearken/", env!("CARGO_PKG_VERSION")))
                .build(),
        }
    }
}

impl Transport for HttpTransport {
    fn post(&self, url: &str, token: Option<&str>, body: Body) -> Result<Reply, String> {
        let mut request = self.agent.post(url).set("Accept", "application/json");
        if let Some(token) = token {
            request = request.set("Authorization", &format!("Bearer {token}"));
        }
        let sent = match body {
            Body::Json(value) => request
                .set("Content-Type", "application/json")
                .send_string(&value.to_string()),
            Body::Run(bytes) => request
                .set("Content-Type", "application/octet-stream")
                .send_bytes(&bytes),
        };
        let response = match sent {
            Ok(response) => response,
            Err(ureq::Error::Status(_, response)) => response,
            Err(e) => return Err(e.kind().to_string()),
        };
        let status = response.status();
        let mut text = String::new();
        use std::io::Read;
        let _ = response
            .into_reader()
            .take(MAX_REPLY_BYTES)
            .read_to_string(&mut text);
        Ok(Reply {
            status,
            body: serde_json::from_str(&text).unwrap_or(Value::Null),
        })
    }
}

/// Which part of a successful answer a request hands back.
enum Answer {
    /// `result`: what the handler returned for a run.
    Result,
    /// The whole body: a start's seed and context.
    Whole,
}

/// What the server says to play: the seed and context a run must start
/// from, and anything else the handler added.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub seed: u64,
    pub context: Value,
    /// The whole answer, for fields of the game's own.
    pub answer: Value,
}

impl Plan {
    /// Reads a start's answer; `None` when it holds no seed.
    pub fn from_answer(answer: &Value) -> Option<Plan> {
        let seed = match &answer["seed"] {
            Value::String(s) if s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) => {
                s.parse().ok()?
            }
            _ => return None,
        };
        Some(Plan {
            seed,
            context: answer.get("context").cloned().unwrap_or(Value::Null),
            answer: answer.clone(),
        })
    }

    /// The context as the rules' own type.
    pub fn context<C: serde::de::DeserializeOwned>(&self) -> Option<C> {
        serde_json::from_value(self.context.clone()).ok()
    }
}

/// A submitted run, for asking after its answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ticket(u64);

/// Why something online did not happen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OnlineError {
    /// The game has no server (`Game::online` is `None`).
    NotOffered,
    /// No account is connected.
    NotConnected,
    /// This copy of the game has no rules ID, so no server can replay its
    /// runs: a developer build, or a release made without `rules`.
    NotOnlineBuild,
    /// The run could not be written as a run file.
    BadRun,
    /// A kind that is not 1 to 32 of `a-z`, `0-9` and `-`.
    BadKind,
    /// Too many runs are still on their way.
    Busy,
    /// No answer from the server: no network, or it timed out.
    Offline,
    /// The server no longer knows the account; it has been disconnected.
    SignedOut,
    /// The server asked the game to slow down.
    RateLimited,
    /// The server is down or broken.
    Unavailable,
    /// The server refused the run: its `error` code, and a line for the
    /// player when it gave one.
    Refused {
        code: String,
        message: Option<String>,
    },
    /// The code expired before the player entered it.
    CodeExpired,
}

impl OnlineError {
    /// A short stable name, for logs and a game's own matching.
    pub fn code(&self) -> &str {
        match self {
            OnlineError::NotOffered => "not-offered",
            OnlineError::NotConnected => "not-connected",
            OnlineError::NotOnlineBuild => "not-online-build",
            OnlineError::BadRun => "bad-run",
            OnlineError::BadKind => "bad-kind",
            OnlineError::Busy => "busy",
            OnlineError::Offline => "offline",
            OnlineError::SignedOut => "signed-out",
            OnlineError::RateLimited => "rate-limited",
            OnlineError::Unavailable => "unavailable",
            OnlineError::Refused { code, .. } => code,
            OnlineError::CodeExpired => "code-expired",
        }
    }

    /// What the player hears: what happened and what to do. `site` is the
    /// server's name ("orinks.net").
    pub fn spoken(&self, site: &str) -> String {
        match self {
            OnlineError::NotOffered => "This game has no online play.".into(),
            OnlineError::NotConnected => {
                format!("Not connected to {site}. Connect an account in Settings.")
            }
            OnlineError::NotOnlineBuild => {
                "This copy of the game cannot send runs. Online play needs a released copy.".into()
            }
            OnlineError::BadRun => "This run could not be sent.".into(),
            OnlineError::BadKind => "This run could not be sent.".into(),
            OnlineError::Busy => "Still sending earlier runs. Try again in a moment.".into(),
            OnlineError::Offline => format!("Could not reach {site}. Check your connection."),
            OnlineError::SignedOut => {
                format!("Your {site} account was disconnected. Connect it again in Settings.")
            }
            OnlineError::RateLimited => {
                format!("{site} is busy with your runs. Try again in a minute.")
            }
            OnlineError::Unavailable => format!("{site} is not answering. Try again later."),
            OnlineError::Refused {
                message: Some(m), ..
            } => crate::text::end_sentence(m),
            OnlineError::Refused { .. } => format!("{site} did not accept this run."),
            OnlineError::CodeExpired => "The code expired. Press Enter for a new one.".into(),
        }
    }

    fn from_reply(reply: &Reply) -> OnlineError {
        let code = reply.body["error"].as_str().unwrap_or("").to_string();
        match reply.status {
            401 => OnlineError::SignedOut,
            429 => OnlineError::RateLimited,
            410 => OnlineError::CodeExpired,
            400..=499 => OnlineError::Refused {
                code: if code.is_empty() {
                    format!("http-{}", reply.status)
                } else {
                    code
                },
                message: reply.body["message"]
                    .as_str()
                    .filter(|m| !m.trim().is_empty())
                    .map(|m| m.chars().take(200).collect()),
            },
            _ => OnlineError::Unavailable,
        }
    }
}

impl std::fmt::Display for OnlineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for OnlineError {}

/// Where connecting an account has got to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Connect {
    /// Not connecting.
    Idle,
    /// Asking the server for a code.
    Asking,
    /// Waiting for the player to enter `code` at `url`.
    Waiting {
        code: String,
        url: String,
    },
    /// Connected as `name`.
    Connected {
        name: String,
    },
    Failed(OnlineError),
}

/// Whether `kind` can name a kind of run: 1 to 32 of `a-z`, `0-9` and `-`,
/// not starting with `-`.
pub fn valid_kind(kind: &str) -> bool {
    !kind.is_empty()
        && kind.len() <= 32
        && !kind.starts_with('-')
        && kind
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The most runs on their way at once.
pub const MAX_IN_FLIGHT: usize = 8;
/// The most answers kept for a game to collect.
const MAX_KEPT: usize = 64;
/// The longest a code is waited on, whatever the server says.
const MAX_CONNECT: Duration = Duration::from_secs(15 * 60);

enum Msg {
    Done(Ticket, u64, Result<Value, OnlineError>),
    Code(u64, String, String),
    Connected(u64, String, String),
    ConnectFailed(u64, OnlineError),
}

/// The game's online connection, at `ctx.online`.
pub struct Online {
    config: Option<OnlineConfig>,
    transport: Arc<dyn Transport>,
    token: Option<Secret>,
    name: Option<String>,
    /// Counts tokens, so a late "signed out" for an old token cannot
    /// disconnect the account the player has connected since.
    token_gen: u64,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    next: u64,
    in_flight: HashSet<Ticket>,
    done: VecDeque<(Ticket, Result<Value, OnlineError>)>,
    connect: Connect,
    connect_gen: u64,
    cancel: Option<Arc<AtomicBool>>,
    /// The account changed and `settings.json` needs saving.
    changed: bool,
}

impl Online {
    /// The connection for a game with `config` (`None`: no online play),
    /// signed in with the account saved in `saved`.
    pub fn new(config: Option<OnlineConfig>, saved: &OnlineSettings) -> Online {
        let config = config.filter(|c| {
            let ok = c.url_allowed();
            if !ok {
                log::warn!("online: {} is not an HTTPS URL; online play is off", c.url);
            }
            ok
        });
        let (tx, rx) = channel();
        Online {
            config,
            transport: Arc::new(HttpTransport::default()),
            token: saved.token.clone().filter(|t| !t.0.is_empty()),
            name: saved.name.clone(),
            token_gen: 0,
            tx,
            rx,
            next: 1,
            in_flight: HashSet::new(),
            done: VecDeque::new(),
            connect: Connect::Idle,
            connect_gen: 0,
            cancel: None,
            changed: false,
        }
    }

    /// Sends requests through `transport` instead, as a test does.
    pub fn set_transport(&mut self, transport: Arc<dyn Transport>) {
        self.transport = transport;
    }

    /// Whether the game has a server.
    pub fn is_offered(&self) -> bool {
        self.config.is_some()
    }

    /// The server's name as the player hears it; empty when there is none.
    pub fn site(&self) -> &str {
        self.config.as_ref().map_or("", |c| c.site.as_str())
    }

    /// Whether an account is connected.
    pub fn is_connected(&self) -> bool {
        self.token.is_some()
    }

    /// The name the server knows the player by.
    pub fn player_name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Sends `run` as a run of `kind` in the background. The answer comes
    /// later, through [`Online::result`].
    pub fn submit(&mut self, kind: &str, run: &Run) -> Result<Ticket, OnlineError> {
        let config = self.config.clone().ok_or(OnlineError::NotOffered)?;
        if !valid_kind(kind) {
            return Err(OnlineError::BadKind);
        }
        let token = self.token.clone().ok_or(OnlineError::NotConnected)?;
        if run.build.is_none() {
            return Err(OnlineError::NotOnlineBuild);
        }
        if self.in_flight.len() >= MAX_IN_FLIGHT {
            return Err(OnlineError::Busy);
        }
        let file = run.encode().map_err(|_| OnlineError::BadRun)?;
        let url = config.endpoint(&format!("runs?kind={kind}"));
        Ok(self.send(url, token, Body::Run(file), Answer::Result))
    }

    /// Asks the server what to play for `kind`, for a handler that hands
    /// out seeds (a day's seed kept secret until the day begins, or one of
    /// the player's own). The answer, through [`Online::result`], reads with
    /// [`Plan::from_answer`].
    pub fn start(&mut self, kind: &str) -> Result<Ticket, OnlineError> {
        let config = self.config.clone().ok_or(OnlineError::NotOffered)?;
        if !valid_kind(kind) {
            return Err(OnlineError::BadKind);
        }
        let token = self.token.clone().ok_or(OnlineError::NotConnected)?;
        if self.in_flight.len() >= MAX_IN_FLIGHT {
            return Err(OnlineError::Busy);
        }
        let url = config.endpoint(&format!("start?kind={kind}"));
        Ok(self.send(url, token, Body::Json(json!({})), Answer::Whole))
    }

    fn send(&mut self, url: String, token: Secret, body: Body, answer: Answer) -> Ticket {
        let ticket = Ticket(self.next);
        self.next += 1;
        self.in_flight.insert(ticket);
        let transport = self.transport.clone();
        let tx = self.tx.clone();
        let generation = self.token_gen;
        std::thread::spawn(move || {
            let answer = match transport.post(&url, Some(&token.0), body) {
                Err(e) => {
                    log::info!("online: no answer from the server: {e}");
                    Err(OnlineError::Offline)
                }
                Ok(reply) if reply.status == 200 && reply.body["ok"] == true => Ok(match answer {
                    Answer::Result => reply.body.get("result").cloned().unwrap_or(Value::Null),
                    Answer::Whole => reply.body,
                }),
                Ok(reply) if reply.status == 200 => Err(OnlineError::Unavailable),
                Ok(reply) => Err(OnlineError::from_reply(&reply)),
            };
            let _ = tx.send(Msg::Done(ticket, generation, answer));
        });
        ticket
    }

    /// Whether `ticket`'s run is still on its way.
    pub fn is_pending(&self, ticket: Ticket) -> bool {
        self.in_flight.contains(&ticket)
    }

    /// `ticket`'s answer once it has come, taken: the handler's result, or
    /// why there is none. `None` while it is on its way, and after it has
    /// been taken.
    pub fn result(&mut self, ticket: Ticket) -> Option<Result<Value, OnlineError>> {
        let at = self.done.iter().position(|(t, _)| *t == ticket)?;
        self.done.remove(at).map(|(_, r)| r)
    }

    /// Starts connecting an account: asks the server for a code, then waits
    /// in the background for the player to enter it. Watch
    /// [`Online::connect_state`].
    pub fn connect(&mut self) {
        let Some(config) = self.config.clone() else {
            self.connect = Connect::Failed(OnlineError::NotOffered);
            return;
        };
        self.cancel_connect();
        self.connect_gen += 1;
        let generation = self.connect_gen;
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        self.connect = Connect::Asking;
        let transport = self.transport.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let msg = match activate(&*transport, &config, &cancel, &tx, generation) {
                Ok(Some((token, name))) => Msg::Connected(generation, token, name),
                Ok(None) => return,
                Err(e) => Msg::ConnectFailed(generation, e),
            };
            let _ = tx.send(msg);
        });
    }

    /// Stops waiting on a code.
    pub fn cancel_connect(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        if !matches!(self.connect, Connect::Connected { .. }) {
            self.connect = Connect::Idle;
        }
    }

    /// Where connecting has got to.
    pub fn connect_state(&self) -> &Connect {
        &self.connect
    }

    /// Forgets the account on this computer.
    pub fn disconnect(&mut self) {
        self.cancel_connect();
        self.connect = Connect::Idle;
        self.forget();
    }

    fn forget(&mut self) {
        if self.token.is_some() || self.name.is_some() {
            self.token = None;
            self.name = None;
            self.token_gen += 1;
            self.changed = true;
        }
    }

    /// Collects what the background requests have finished. Called once a
    /// frame by the app; true when the account changed and should be saved.
    pub fn update(&mut self) -> bool {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Done(ticket, generation, answer) => {
                    self.in_flight.remove(&ticket);
                    if answer == Err(OnlineError::SignedOut) && generation == self.token_gen {
                        self.forget();
                    }
                    self.done.push_back((ticket, answer));
                    if self.done.len() > MAX_KEPT {
                        self.done.pop_front();
                    }
                }
                Msg::Code(generation, code, url) if generation == self.connect_gen => {
                    if !matches!(self.connect, Connect::Idle) {
                        self.connect = Connect::Waiting { code, url };
                    }
                }
                Msg::Connected(generation, token, name) if generation == self.connect_gen => {
                    if !matches!(self.connect, Connect::Idle) {
                        self.token = Some(Secret(token));
                        self.name = Some(name.clone());
                        self.token_gen += 1;
                        self.changed = true;
                        self.cancel = None;
                        self.connect = Connect::Connected { name };
                    }
                }
                Msg::ConnectFailed(generation, e)
                    if generation == self.connect_gen && !matches!(self.connect, Connect::Idle) =>
                {
                    self.cancel = None;
                    self.connect = Connect::Failed(e);
                }
                _ => {}
            }
        }
        std::mem::take(&mut self.changed)
    }

    /// The account as `settings.json` keeps it.
    pub fn saved(&self) -> OnlineSettings {
        OnlineSettings {
            token: self.token.clone(),
            name: self.name.clone(),
        }
    }
}

/// A name from the server, cut to something sensible to speak.
fn clean_name(name: Option<&str>) -> String {
    let name: String = name
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(32)
        .collect();
    let name = name.trim();
    if name.is_empty() {
        "your account".to_string()
    } else {
        name.to_string()
    }
}

/// The connect flow: a code from `activate/start`, then `activate/poll`
/// until the player has entered it. `Ok(None)` when cancelled.
fn activate(
    transport: &dyn Transport,
    config: &OnlineConfig,
    cancel: &AtomicBool,
    tx: &Sender<Msg>,
    generation: u64,
) -> Result<Option<(String, String)>, OnlineError> {
    let started = transport
        .post(
            &config.endpoint("activate/start"),
            None,
            Body::Json(json!({})),
        )
        .map_err(|_| OnlineError::Offline)?;
    if started.status != 200 {
        return Err(OnlineError::from_reply(&started));
    }
    let body = &started.body;
    let (Some(device), Some(code)) = (body["device_code"].as_str(), body["user_code"].as_str())
    else {
        return Err(OnlineError::Unavailable);
    };
    let code: String = code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(16)
        .collect();
    let url: String = body["verify_url"]
        .as_str()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect();
    if code.is_empty() || device.len() > 128 {
        return Err(OnlineError::Unavailable);
    }
    let device = device.to_string();
    let _ = tx.send(Msg::Code(generation, code, url));
    let expires = body["expires_in"]
        .as_u64()
        .map_or(MAX_CONNECT, |s| Duration::from_secs(s).min(MAX_CONNECT));
    let mut interval = Duration::from_secs(body["interval"].as_u64().unwrap_or(5).clamp(2, 30));
    let until = Instant::now() + expires;
    while Instant::now() < until {
        // Sleep in small slices, so a cancel is noticed at once.
        let wake = Instant::now() + interval;
        while Instant::now() < wake {
            if cancel.load(Ordering::Relaxed) {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let reply = match transport.post(
            &config.endpoint("activate/poll"),
            None,
            Body::Json(json!({ "device_code": device })),
        ) {
            Ok(reply) => reply,
            // A dropped poll is tried again; the code is still good.
            Err(_) => continue,
        };
        match (reply.status, reply.body["status"].as_str()) {
            (200, Some("ready")) => {
                let Some(token) = reply.body["token"].as_str().filter(|t| {
                    (16..=512).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_graphic())
                }) else {
                    return Err(OnlineError::Unavailable);
                };
                let name = clean_name(reply.body["name"].as_str());
                return Ok(Some((token.to_string(), name)));
            }
            (200, _) => {}
            (429, _) => interval = (interval * 2).min(Duration::from_secs(30)),
            (410, _) => return Err(OnlineError::CodeExpired),
            (500..=599, _) => {}
            _ => return Err(OnlineError::from_reply(&reply)),
        }
    }
    Err(OnlineError::CodeExpired)
}

/// A code as the player hears it: each character on its own, with a pause
/// at the dash. "A, B, C, 4, then D, E, F, 7".
pub fn spell_code(code: &str) -> String {
    code.split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.chars()
                .map(|c| c.to_ascii_uppercase().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .collect::<Vec<_>>()
        .join(", then ")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Answers each request from a script, and remembers what was sent.
    #[derive(Default)]
    pub struct Scripted {
        pub sent: Mutex<Vec<(String, Option<String>)>>,
        pub replies: Mutex<VecDeque<Result<Reply, String>>>,
    }

    impl Scripted {
        pub fn answering(replies: Vec<Result<Reply, String>>) -> Arc<Scripted> {
            Arc::new(Scripted {
                sent: Mutex::default(),
                replies: Mutex::new(replies.into()),
            })
        }
    }

    impl Transport for Scripted {
        fn post(&self, url: &str, token: Option<&str>, _body: Body) -> Result<Reply, String> {
            self.sent
                .lock()
                .unwrap()
                .push((url.to_string(), token.map(String::from)));
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Err("no more replies".into()))
        }
    }

    pub fn reply(status: u16, body: Value) -> Result<Reply, String> {
        Ok(Reply { status, body })
    }

    fn config() -> Option<OnlineConfig> {
        Some(OnlineConfig::new(
            "https://example.test/api/games/g",
            "example.test",
        ))
    }

    fn connected() -> OnlineSettings {
        OnlineSettings {
            token: Some(Secret("t".repeat(32))),
            name: Some("Ada".into()),
        }
    }

    fn run() -> Run {
        Run {
            build: hearken_replay::BuildId([1; 32]),
            rules: "coin-dash".into(),
            seed: 1,
            context: "null".into(),
            ticks: 1,
            inputs: Vec::new(),
        }
    }

    fn wait(online: &mut Online, ticket: Ticket) -> Result<Value, OnlineError> {
        for _ in 0..500 {
            online.update();
            if let Some(answer) = online.result(ticket) {
                return answer;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("no answer");
    }

    #[test]
    fn only_https_or_this_computer() {
        for ok in [
            "https://orinks.net/api/games/wickets",
            "http://localhost:8787",
            "http://127.0.0.1",
        ] {
            assert!(OnlineConfig::new(ok, "x").url_allowed(), "{ok}");
        }
        for bad in [
            "http://orinks.net",
            "http://localhost.evil.test",
            "https://",
            "https://a.test/?x=1",
            "https://user@a.test",
            "ftp://a.test",
        ] {
            assert!(!OnlineConfig::new(bad, "x").url_allowed(), "{bad}");
        }
    }

    #[test]
    fn a_run_goes_with_the_token_and_comes_back_as_the_handlers_result() {
        let transport = Scripted::answering(vec![reply(
            200,
            json!({ "ok": true, "result": { "rank": 14, "of": 230 } }),
        )]);
        let mut online = Online::new(config(), &connected());
        online.set_transport(transport.clone());
        let ticket = online.submit("daily-challenge", &run()).unwrap();
        assert!(online.is_pending(ticket));
        assert_eq!(
            wait(&mut online, ticket),
            Ok(json!({ "rank": 14, "of": 230 }))
        );
        assert!(!online.is_pending(ticket));
        let sent = transport.sent.lock().unwrap();
        assert_eq!(
            sent[0],
            (
                "https://example.test/api/games/g/runs?kind=daily-challenge".to_string(),
                Some("t".repeat(32))
            )
        );
    }

    #[test]
    fn nothing_is_sent_without_an_account_or_a_release() {
        let mut online = Online::new(config(), &OnlineSettings::default());
        assert_eq!(
            online.submit("daily", &run()),
            Err(OnlineError::NotConnected)
        );
        let mut online = Online::new(config(), &connected());
        let mut dev = run();
        dev.build = hearken_replay::BuildId::NONE;
        assert_eq!(
            online.submit("daily", &dev),
            Err(OnlineError::NotOnlineBuild)
        );
        assert_eq!(online.submit("Daily", &run()), Err(OnlineError::BadKind));
        assert_eq!(online.submit("a&b=c", &run()), Err(OnlineError::BadKind));
        let mut none = Online::new(None, &connected());
        assert_eq!(none.submit("daily", &run()), Err(OnlineError::NotOffered));
        let insecure = Some(OnlineConfig::new("http://orinks.net", "orinks.net"));
        assert!(!Online::new(insecure, &connected()).is_offered());
    }

    #[test]
    fn a_signed_out_answer_disconnects_the_account() {
        let transport = Scripted::answering(vec![reply(401, json!({ "error": "signed-out" }))]);
        let mut online = Online::new(config(), &connected());
        online.set_transport(transport);
        let ticket = online.submit("daily", &run()).unwrap();
        assert_eq!(wait(&mut online, ticket), Err(OnlineError::SignedOut));
        assert!(!online.is_connected());
        assert_eq!(online.saved(), OnlineSettings::default());
    }

    #[test]
    fn refusals_carry_the_servers_words() {
        let transport = Scripted::answering(vec![
            reply(
                422,
                json!({ "error": "event-over", "message": "The festival has ended" }),
            ),
            reply(429, json!({ "error": "rate-limited" })),
            reply(503, Value::Null),
            Err("timed out".into()),
        ]);
        let mut online = Online::new(config(), &connected());
        online.set_transport(transport);
        let refusal = OnlineError::Refused {
            code: "event-over".into(),
            message: Some("The festival has ended".into()),
        };
        let refused = Err(refusal.clone());
        let mut answers = Vec::new();
        for _ in 0..4 {
            let ticket = online.submit("event", &run()).unwrap();
            answers.push(wait(&mut online, ticket));
        }
        assert_eq!(
            answers,
            [
                refused.clone(),
                Err(OnlineError::RateLimited),
                Err(OnlineError::Unavailable),
                Err(OnlineError::Offline)
            ]
        );
        assert_eq!(refusal.spoken("example.test"), "The festival has ended.");
    }

    #[test]
    fn connecting_keeps_the_token_once_the_code_is_entered() {
        let transport = Scripted::answering(vec![
            reply(
                200,
                json!({ "device_code": "d".repeat(64), "user_code": "ABCD-EF34",
                        "verify_url": "https://example.test/activate", "interval": 2 }),
            ),
            reply(200, json!({ "status": "pending" })),
            reply(
                200,
                json!({ "status": "ready", "token": "n".repeat(40), "name": "Bo\u{7}" }),
            ),
        ]);
        let mut online = Online::new(config(), &OnlineSettings::default());
        online.set_transport(transport);
        online.connect();
        let mut saw_code = false;
        for _ in 0..1000 {
            let changed = online.update();
            if let Connect::Waiting { code, url } = online.connect_state() {
                assert_eq!(
                    (code.as_str(), url.as_str()),
                    ("ABCD-EF34", "https://example.test/activate")
                );
                saw_code = true;
            }
            if changed {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(saw_code);
        assert_eq!(
            online.connect_state(),
            &Connect::Connected { name: "Bo".into() }
        );
        assert_eq!(online.saved().token, Some(Secret("n".repeat(40))));
        assert!(!format!("{:?}", online.saved()).contains(&"n".repeat(40)));
    }

    #[test]
    fn a_start_hands_back_the_seed_to_play() {
        let transport = Scripted::answering(vec![reply(
            200,
            json!({ "ok": true, "seed": "18446744073709551615", "context": { "seconds": 60 } }),
        )]);
        let mut online = Online::new(config(), &connected());
        online.set_transport(transport.clone());
        let ticket = online.start("daily").unwrap();
        let plan = Plan::from_answer(&wait(&mut online, ticket).unwrap()).unwrap();
        assert_eq!(plan.seed, u64::MAX);
        assert_eq!(plan.context, json!({ "seconds": 60 }));
        assert_eq!(
            transport.sent.lock().unwrap()[0].0,
            "https://example.test/api/games/g/start?kind=daily"
        );
        assert_eq!(Plan::from_answer(&json!({ "seed": 5 })), None);
        assert_eq!(
            Plan::from_answer(&json!({ "seed": "99999999999999999999" })),
            None
        );
    }

    #[test]
    fn codes_are_spelled_out() {
        assert_eq!(spell_code("ABCD-EF34"), "A, B, C, D, then E, F, 3, 4");
    }
}
