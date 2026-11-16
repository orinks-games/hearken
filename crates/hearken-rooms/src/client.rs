//! The client: `Rooms`, which talks to the server from worker threads and
//! hands answers back on the game loop, and `Room`, what it knows about
//! each room the player is in.
//!
//! Nothing here blocks a frame. A request spawns a thread and returns a
//! [`Ticket`]; `update` collects what has come back, polls the rooms the
//! game is watching, and says what changed. The connection is an explicit
//! state ([`Connection`]): polling, or offline after two polls in a row
//! got no answer, spoken once by whichever screen is open.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::error::RoomsError;
use crate::protocol::{Event, EventKind, Mute, MuteMode, Phase, RoomId, RoomInfo};
#[cfg(not(feature = "http"))]
use crate::transport::Reply;
use crate::transport::Transport;

/// Where a game's rooms server is, and what to call it out loud.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomsConfig {
    /// The service's base URL, such as `https://orinks.net/api/games/wickets`.
    pub url: String,
    /// The site's name as the player hears it: "orinks.net".
    pub site: String,
}

impl RoomsConfig {
    pub fn new(url: impl Into<String>, site: impl Into<String>) -> Self {
        RoomsConfig {
            url: url.into(),
            site: site.into(),
        }
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}/{path}", self.url.trim_end_matches('/'))
    }
}

/// A request on its way, for asking after its answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ticket(u64);

/// How the client is reaching the server right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connection {
    /// Requests and polls are answered.
    Polling,
    /// Two polls in a row got no answer. Cleared silently when one does.
    Offline,
}

/// What a finished request came to.
#[derive(Clone, Debug, PartialEq)]
pub enum Answer {
    /// A room the player is now in (create, join, find, start).
    Room(RoomId),
    /// The player's rooms, newest first (list).
    List(Vec<RoomInfo>),
    /// The sequence number an event was stored at (turn, chat, leave, finish).
    Seq(u64),
    /// The players the player has muted or blocked (mutes).
    Mutes(Vec<Mute>),
    /// Done, with nothing more to say (set_mute, report).
    Done,
}

/// What changed in `update`, for a screen to speak.
#[derive(Clone, Debug, PartialEq)]
pub enum Notice {
    /// The room's description changed: members, presence, snapshot.
    Updated(RoomId),
    /// New events arrived; the first has this sequence number.
    Events(RoomId, u64),
    /// The room moved to a new phase.
    Phase(RoomId, Phase),
    /// The connection changed state.
    Connection(Connection),
    /// A poll of a watched room was refused; the room is no longer watched.
    Dropped(RoomId, RoomsError),
}

/// Polls while a room is watched: every 2 s, then every 10 s once a minute
/// has passed without a change.
pub const POLL_ACTIVE: Duration = Duration::from_secs(2);
pub const POLL_QUIET: Duration = Duration::from_secs(10);
pub const QUIET_AFTER: Duration = Duration::from_secs(60);
/// The most requests on their way at once.
pub const MAX_IN_FLIGHT: usize = 8;
/// Events kept per room in memory; older ones are dropped from the front.
pub const MAX_KEPT_EVENTS: usize = 4096;
/// The lines read back when a channel is first watched: what was said
/// before the player came in.
pub const HISTORY_LINES: usize = 20;
/// Finished answers kept until the game takes them.
const MAX_KEPT_ANSWERS: usize = 64;

/// What the client knows about one room the player is in.
#[derive(Debug)]
pub struct Room {
    pub info: RoomInfo,
    /// Every event read so far, in sequence order.
    pub events: Vec<Event>,
    watched: bool,
    poll: Option<Ticket>,
    /// When to poll next; `None` is at the next `update`.
    next_poll: Option<Instant>,
    changed_at: Instant,
    /// How many past lines to read before polling onward: a channel
    /// first watched starts from its recent history, not its first line.
    history: Option<usize>,
}

impl Room {
    fn new(info: RoomInfo, now: Instant) -> Room {
        Room {
            info,
            events: Vec::new(),
            watched: false,
            poll: None,
            next_poll: None,
            changed_at: now,
            history: None,
        }
    }

    pub fn id(&self) -> &RoomId {
        &self.info.id
    }

    /// The sequence number of the last event read.
    pub fn read_seq(&self) -> u64 {
        self.events.last().map_or(0, |e| e.seq)
    }

    /// Whether the server has events this client has not read yet.
    pub fn behind(&self) -> bool {
        self.info.seq > self.read_seq()
    }

    /// Whether the room is being polled.
    pub fn is_watched(&self) -> bool {
        self.watched
    }

    /// Events after `seq`, in order.
    pub fn events_after(&self, seq: u64) -> impl Iterator<Item = &Event> {
        self.events.iter().filter(move |e| e.seq > seq)
    }

    /// Chat events read so far, oldest first.
    pub fn chat_events(&self) -> impl Iterator<Item = &Event> {
        self.events.iter().filter(|e| e.kind == EventKind::Chat)
    }

    /// Chat lines read so far, oldest first, as `(name, text)`.
    pub fn chat(&self) -> impl Iterator<Item = (&str, &str)> {
        self.events
            .iter()
            .filter(|e| e.kind == EventKind::Chat)
            .map(|e| {
                (
                    e.name.as_deref().unwrap_or(""),
                    e.text.as_deref().unwrap_or(""),
                )
            })
    }

    /// Takes `events` in, dropping any already read, and says the first
    /// new sequence number when there was one.
    fn absorb(&mut self, events: Vec<Event>) -> Option<u64> {
        let mut first = None;
        let mut last = self.read_seq();
        for event in events {
            if event.seq <= last {
                continue;
            }
            last = event.seq;
            first.get_or_insert(event.seq);
            self.events.push(event);
        }
        if self.events.len() > MAX_KEPT_EVENTS {
            let drop = self.events.len() - MAX_KEPT_EVENTS;
            self.events.drain(..drop);
        }
        first
    }
}

enum Request {
    Create,
    Join,
    Find,
    List,
    Start,
    Post(RoomId),
    Poll(RoomId),
    Mutes,
    SetMute,
    Report,
}

enum Msg {
    Done(Ticket, u64, Result<Value, RoomsError>),
}

/// The game's rooms client. One per game, at `ctx.rooms` in hearken-app.
pub struct Rooms {
    config: Option<RoomsConfig>,
    transport: Arc<dyn Transport>,
    token: Option<String>,
    /// Counts tokens, so a late "signed out" for an old token cannot
    /// disconnect the one connected since.
    token_gen: u64,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    next: u64,
    in_flight: HashMap<Ticket, Request>,
    done: VecDeque<(Ticket, Result<Answer, RoomsError>)>,
    rooms: BTreeMap<RoomId, Room>,
    /// The room the player is looking at: the one chat keys send to.
    focus: Option<RoomId>,
    connection: Connection,
    poll_failures: u32,
    signed_out: bool,
}

impl Rooms {
    /// The client for a game with `config` (`None`: no online play).
    pub fn new(config: Option<RoomsConfig>) -> Rooms {
        let (tx, rx) = channel();
        Rooms {
            config,
            #[cfg(feature = "http")]
            transport: Arc::new(crate::transport::HttpTransport::default()),
            #[cfg(not(feature = "http"))]
            transport: Arc::new(NoTransport),
            token: None,
            token_gen: 0,
            tx,
            rx,
            next: 1,
            in_flight: HashMap::new(),
            done: VecDeque::new(),
            rooms: BTreeMap::new(),
            focus: None,
            connection: Connection::Polling,
            poll_failures: 0,
            signed_out: false,
        }
    }

    /// Sends requests through `transport` instead, as a test does.
    pub fn set_transport(&mut self, transport: Arc<dyn Transport>) {
        self.transport = transport;
    }

    /// The bearer token the player holds: an account's or a guest's. `None`
    /// forgets it and every room with it.
    pub fn set_token(&mut self, token: Option<String>) {
        if self.token == token {
            return;
        }
        self.token = token.filter(|t| !t.is_empty());
        self.token_gen += 1;
        self.signed_out = false;
        self.rooms.clear();
        self.focus = None;
        self.in_flight.clear();
    }

    /// Whether the game has a rooms server.
    pub fn is_offered(&self) -> bool {
        self.config.is_some()
    }

    /// Whether a token is held, so requests can be sent.
    pub fn is_connected(&self) -> bool {
        self.token.is_some()
    }

    /// Whether the server said the token is no longer good. The owner of
    /// the token (`ctx.online`) forgets it; this only reports it.
    pub fn was_signed_out(&mut self) -> bool {
        std::mem::take(&mut self.signed_out)
    }

    /// The server's name as the player hears it; empty when there is none.
    pub fn site(&self) -> &str {
        self.config.as_ref().map_or("", |c| c.site.as_str())
    }

    pub fn connection(&self) -> Connection {
        self.connection
    }

    /// The rooms this client knows, in id order.
    pub fn rooms(&self) -> impl Iterator<Item = &Room> {
        self.rooms.values()
    }

    pub fn room(&self, id: &RoomId) -> Option<&Room> {
        self.rooms.get(id)
    }

    /// Forgets a room this client knows: after leaving, or when the player
    /// is done with a finished one.
    pub fn forget(&mut self, id: &RoomId) {
        self.rooms.remove(id);
        if self.focus.as_ref() == Some(id) {
            self.focus = None;
        }
    }

    /// The room the player is looking at, if any: the last one watched,
    /// until it is unwatched or forgotten. Chat keys send here.
    pub fn focus(&self) -> Option<&RoomId> {
        self.focus
            .as_ref()
            .filter(|id| self.rooms.contains_key(*id))
    }

    /// The room the player is looking at, itself.
    pub fn focused(&self) -> Option<&Room> {
        self.focus().and_then(|id| self.rooms.get(id))
    }

    /// Points the chat keys at `id` without watching it.
    pub fn set_focus(&mut self, id: Option<RoomId>) {
        self.focus = id;
    }

    /// Takes in a room the player is in from a `list` answer, so it can be
    /// watched and played. A room already known keeps its events.
    pub fn adopt(&mut self, info: RoomInfo) {
        let now = Instant::now();
        match self.rooms.get_mut(&info.id) {
            Some(room) => room.info = info,
            None => {
                self.rooms.insert(info.id.clone(), Room::new(info, now));
            }
        }
    }

    fn ready(&self, kind: Option<&str>) -> Result<RoomsConfig, RoomsError> {
        let config = self.config.clone().ok_or(RoomsError::NotOffered)?;
        if self.token.is_none() {
            return Err(RoomsError::NotConnected);
        }
        if let Some(kind) = kind {
            if !crate::valid_kind(kind) {
                return Err(RoomsError::BadKind);
            }
        }
        if self.in_flight.len() >= MAX_IN_FLIGHT {
            return Err(RoomsError::Busy);
        }
        Ok(config)
    }

    /// A new room of `kind`, with `context` for the handler and `open` to
    /// list it for Play with anyone. Answers [`Answer::Room`].
    pub fn create(
        &mut self,
        kind: &str,
        context: &impl serde::Serialize,
        open: bool,
    ) -> Result<Ticket, RoomsError> {
        let config = self.ready(Some(kind))?;
        let context = serde_json::to_value(context).map_err(|_| RoomsError::BadKind)?;
        let url = config.endpoint(&format!("rooms?kind={kind}"));
        Ok(self.send(
            Request::Create,
            Method::Post(json!({ "context": context, "open": open })),
            url,
        ))
    }

    /// Takes a seat in the room with invite `code`. Answers [`Answer::Room`].
    pub fn join(&mut self, code: &str) -> Result<Ticket, RoomsError> {
        let config = self.ready(None)?;
        let code = crate::normalize_code(code).ok_or(RoomsError::BadCode)?;
        let url = config.endpoint("rooms/join");
        Ok(self.send(Request::Join, Method::Post(json!({ "code": code })), url))
    }

    /// Joins an open room of `kind` that is waiting, or makes one. Answers
    /// [`Answer::Room`].
    pub fn find(&mut self, kind: &str) -> Result<Ticket, RoomsError> {
        let config = self.ready(Some(kind))?;
        let url = config.endpoint(&format!("rooms/find?kind={kind}"));
        Ok(self.send(Request::Find, Method::Post(json!({})), url))
    }

    /// Joins the channel of `kind` called `name`, made the first time
    /// anyone asks for it. Answers [`Answer::Room`].
    pub fn channel(&mut self, kind: &str, name: &str) -> Result<Ticket, RoomsError> {
        let config = self.ready(Some(kind))?;
        let name = crate::clean_text(name, crate::MAX_NAME_CHARS);
        if name.is_empty() {
            return Err(RoomsError::BadKind);
        }
        let url = config.endpoint(&format!(
            "rooms/find?kind={kind}&name={}",
            percent_encode(&name)
        ));
        Ok(self.send(Request::Find, Method::Post(json!({})), url))
    }

    /// The room of two of `kind` with the player whose handle this is,
    /// found or made. Answers [`Answer::Room`]; [`RoomsError::Blocked`]
    /// when either has blocked the other.
    pub fn direct(&mut self, kind: &str, handle: &str) -> Result<Ticket, RoomsError> {
        let config = self.ready(Some(kind))?;
        if !crate::valid_handle(handle) {
            return Err(RoomsError::UnknownPlayer);
        }
        let url = config.endpoint(&format!("rooms/direct?kind={kind}"));
        Ok(self.send(
            Request::Find,
            Method::Post(json!({ "handle": handle })),
            url,
        ))
    }

    /// Who the player has muted or blocked. Answers [`Answer::Mutes`].
    pub fn mutes(&mut self) -> Result<Ticket, RoomsError> {
        let config = self.ready(None)?;
        let url = config.endpoint("rooms/mutes");
        Ok(self.send(Request::Mutes, Method::Get, url))
    }

    /// Mutes, blocks or hears again the player with `handle`. Answers
    /// [`Answer::Done`]. The other player is never told.
    pub fn set_mute(&mut self, handle: &str, mode: MuteMode) -> Result<Ticket, RoomsError> {
        let config = self.ready(None)?;
        if !crate::valid_handle(handle) {
            return Err(RoomsError::UnknownPlayer);
        }
        let url = config.endpoint("rooms/mutes");
        Ok(self.send(
            Request::SetMute,
            Method::Post(json!({ "handle": handle, "mode": mode })),
            url,
        ))
    }

    /// Reports the player with `handle` for what they said in room `id`:
    /// the server hands their recent lines there to the game's host, with
    /// `note` when the player gave one. Answers [`Answer::Done`].
    pub fn report(&mut self, id: &RoomId, handle: &str, note: &str) -> Result<Ticket, RoomsError> {
        let config = self.ready(None)?;
        if !crate::valid_handle(handle) {
            return Err(RoomsError::UnknownPlayer);
        }
        let note = crate::clean_text(note, crate::MAX_CHAT_CHARS);
        let url = config.endpoint(&format!("rooms/{id}/report"));
        Ok(self.send(
            Request::Report,
            Method::Post(json!({ "handle": handle, "note": note })),
            url,
        ))
    }

    /// The player's rooms, newest first. Answers [`Answer::List`], and
    /// refreshes every room this client knows.
    pub fn list(&mut self) -> Result<Ticket, RoomsError> {
        let config = self.ready(None)?;
        let url = config.endpoint("rooms");
        Ok(self.send(Request::List, Method::Get, url))
    }

    /// Starts the room, as its host, once enough seats are filled.
    pub fn start(&mut self, id: &RoomId) -> Result<Ticket, RoomsError> {
        let config = self.ready(None)?;
        let url = config.endpoint(&format!("rooms/{id}/start"));
        Ok(self.send(Request::Start, Method::Post(json!({})), url))
    }

    /// Sends a move: the game's inputs as `(code, value)` pairs, codes at
    /// or below [`hearken_replay::MAX_SEATED_CODE`]. The server orders it
    /// and stamps the seat. Answers [`Answer::Seq`]; the turn itself
    /// arrives as an event on the next poll.
    pub fn post_turn(&mut self, id: &RoomId, inputs: &[(u16, i32)]) -> Result<Ticket, RoomsError> {
        let config = self.ready(None)?;
        if inputs
            .iter()
            .any(|(code, _)| *code > hearken_replay::MAX_SEATED_CODE)
        {
            return Err(RoomsError::BadKind);
        }
        let url = config.endpoint(&format!("rooms/{id}/events"));
        let body = json!({ "type": "turn", "inputs": inputs });
        Ok(self.send(Request::Post(id.clone()), Method::Post(body), url))
    }

    /// Sends a line of chat. Answers [`Answer::Seq`].
    pub fn say(&mut self, id: &RoomId, text: &str) -> Result<Ticket, RoomsError> {
        self.say_tagged(id, text, "")
    }

    /// Sends a line of chat with a tag: a short line about the sender that
    /// is spoken with their name ("Ada, Lake Road: ..."). Answers
    /// [`Answer::Seq`].
    pub fn say_tagged(&mut self, id: &RoomId, text: &str, tag: &str) -> Result<Ticket, RoomsError> {
        let config = self.ready(None)?;
        let text = crate::clean_text(text, crate::MAX_CHAT_CHARS);
        if text.is_empty() {
            return Err(RoomsError::Refused {
                code: "empty".into(),
                message: Some("Nothing to send.".into()),
            });
        }
        let tag = crate::clean_text(tag, crate::MAX_TAG_CHARS);
        let body = if tag.is_empty() {
            json!({ "type": "chat", "text": text })
        } else {
            json!({ "type": "chat", "text": text, "tag": tag })
        };
        let url = config.endpoint(&format!("rooms/{id}/events"));
        Ok(self.send(Request::Post(id.clone()), Method::Post(body), url))
    }

    /// Leaves the room: a waiting room frees the seat, a match forfeits it.
    /// Answers [`Answer::Seq`]. The room stays known until `forget`.
    pub fn leave(&mut self, id: &RoomId) -> Result<Ticket, RoomsError> {
        let config = self.ready(None)?;
        let url = config.endpoint(&format!("rooms/{id}/events"));
        Ok(self.send(
            Request::Post(id.clone()),
            Method::Post(json!({ "type": "leave" })),
            url,
        ))
    }

    /// Tells the server the rules have finished, for a kind whose handler
    /// does not replay after each turn. Answers [`Answer::Seq`].
    pub fn finish(&mut self, id: &RoomId) -> Result<Ticket, RoomsError> {
        let config = self.ready(None)?;
        let url = config.endpoint(&format!("rooms/{id}/events"));
        Ok(self.send(
            Request::Post(id.clone()),
            Method::Post(json!({ "type": "finish" })),
            url,
        ))
    }

    /// Polls `id` while `on`: a room screen watches its room, and stops
    /// when it closes. A watched room is polled at once, and becomes the
    /// focus of the chat keys; unwatching it clears that. A channel
    /// watched for the first time reads its recent lines
    /// ([`HISTORY_LINES`]) rather than its whole log.
    pub fn watch(&mut self, id: &RoomId, on: bool) {
        if let Some(room) = self.rooms.get_mut(id) {
            room.watched = on;
            if on {
                room.next_poll = None;
                if room.events.is_empty() && room.info.is_channel() && room.info.seq > 0 {
                    room.history = Some(HISTORY_LINES);
                }
                self.focus = Some(id.clone());
            } else if self.focus.as_ref() == Some(id) {
                self.focus = None;
            }
        }
    }

    /// Polls `id` now, watched or not.
    pub fn refresh(&mut self, id: &RoomId) {
        if let Some(room) = self.rooms.get_mut(id) {
            room.next_poll = None;
        }
    }

    /// Whether `ticket` is still on its way.
    pub fn is_pending(&self, ticket: Ticket) -> bool {
        self.in_flight.contains_key(&ticket)
    }

    /// `ticket`'s answer once it has come, taken. `None` while it is on its
    /// way, and after it has been taken.
    pub fn result(&mut self, ticket: Ticket) -> Option<Result<Answer, RoomsError>> {
        let at = self.done.iter().position(|(t, _)| *t == ticket)?;
        self.done.remove(at).map(|(_, r)| r)
    }

    fn send(&mut self, request: Request, method: Method, url: String) -> Ticket {
        let ticket = Ticket(self.next);
        self.next += 1;
        self.in_flight.insert(ticket, request);
        let transport = self.transport.clone();
        let tx = self.tx.clone();
        let generation = self.token_gen;
        let token = self.token.clone();
        std::thread::spawn(move || {
            let sent = match &method {
                Method::Post(body) => transport.post(&url, token.as_deref(), body),
                Method::Get => transport.get(&url, token.as_deref()),
            };
            let answer = match sent {
                Err(e) => {
                    log::info!("rooms: no answer from the server: {e}");
                    Err(RoomsError::Offline)
                }
                Ok(reply) if reply.status == 200 && reply.body["ok"] == true => Ok(reply.body),
                Ok(reply) if reply.status == 200 => Err(RoomsError::Unavailable),
                Ok(reply) => Err(RoomsError::from_reply(&reply)),
            };
            let _ = tx.send(Msg::Done(ticket, generation, answer));
        });
        ticket
    }

    /// Collects finished requests and polls watched rooms. Once a frame.
    pub fn update(&mut self) -> Vec<Notice> {
        self.update_at(Instant::now())
    }

    /// `update` at `now`, for tests that drive the clock.
    pub fn update_at(&mut self, now: Instant) -> Vec<Notice> {
        let mut notices = Vec::new();
        while let Ok(Msg::Done(ticket, generation, answer)) = self.rx.try_recv() {
            let Some(request) = self.in_flight.remove(&ticket) else {
                continue;
            };
            if generation != self.token_gen {
                continue;
            }
            if answer == Err(RoomsError::SignedOut) {
                self.signed_out = true;
            }
            match request {
                Request::Poll(id) => self.finish_poll(&id, answer, now, &mut notices),
                Request::Create | Request::Join | Request::Find | Request::Start => {
                    let result = answer.and_then(|body| {
                        let info = read_room(&body)?;
                        let id = info.id.clone();
                        self.take_info(info, now, &mut notices);
                        Ok(Answer::Room(id))
                    });
                    self.keep(ticket, result);
                }
                Request::List => {
                    let result = answer.and_then(|body| {
                        let list: Vec<RoomInfo> = body
                            .get("rooms")
                            .cloned()
                            .and_then(|v| serde_json::from_value(v).ok())
                            .ok_or(RoomsError::Unavailable)?;
                        for info in &list {
                            if self.rooms.contains_key(&info.id) {
                                self.take_info(info.clone(), now, &mut notices);
                            }
                        }
                        Ok(Answer::List(list))
                    });
                    self.keep(ticket, result);
                }
                Request::Mutes => {
                    let result = answer.and_then(|body| {
                        let list: Vec<Mute> = body
                            .get("mutes")
                            .cloned()
                            .and_then(|v| serde_json::from_value(v).ok())
                            .ok_or(RoomsError::Unavailable)?;
                        Ok(Answer::Mutes(list))
                    });
                    self.keep(ticket, result);
                }
                Request::SetMute | Request::Report => {
                    let result = answer.map(|_| Answer::Done);
                    if matches!(request, Request::SetMute) && result.is_ok() {
                        // What the player hears changed: read the watched
                        // rooms again from where they are.
                        for room in self.rooms.values_mut().filter(|r| r.watched) {
                            room.next_poll = None;
                        }
                    }
                    self.keep(ticket, result);
                }
                Request::Post(id) => {
                    let result = answer.and_then(|body| {
                        let seq = body["seq"].as_u64().ok_or(RoomsError::Unavailable)?;
                        if let Ok(info) = read_room(&body) {
                            self.take_info(info, now, &mut notices);
                        }
                        // The event itself comes with the next poll; make
                        // that now.
                        if let Some(room) = self.rooms.get_mut(&id) {
                            room.next_poll = None;
                        }
                        Ok(Answer::Seq(seq))
                    });
                    self.keep(ticket, result);
                }
            }
        }
        self.schedule_polls(now);
        notices
    }

    fn keep(&mut self, ticket: Ticket, result: Result<Answer, RoomsError>) {
        self.done.push_back((ticket, result));
        if self.done.len() > MAX_KEPT_ANSWERS {
            self.done.pop_front();
        }
    }

    /// Stores what the server said about a room, noting what changed.
    fn take_info(&mut self, info: RoomInfo, now: Instant, notices: &mut Vec<Notice>) {
        let id = info.id.clone();
        match self.rooms.get_mut(&id) {
            Some(room) => {
                let was = room.info.phase;
                if room.info != info {
                    room.changed_at = now;
                    room.info = info;
                    notices.push(Notice::Updated(id.clone()));
                }
                if room.info.phase != was {
                    notices.push(Notice::Phase(id, room.info.phase));
                }
            }
            None => {
                self.rooms.insert(id.clone(), Room::new(info, now));
                notices.push(Notice::Updated(id));
            }
        }
    }

    fn finish_poll(
        &mut self,
        id: &RoomId,
        answer: Result<Value, RoomsError>,
        now: Instant,
        notices: &mut Vec<Notice>,
    ) {
        let Some(room) = self.rooms.get_mut(id) else {
            return;
        };
        room.poll = None;
        match answer {
            Ok(body) => {
                if self.poll_failures > 0 || self.connection == Connection::Offline {
                    self.poll_failures = 0;
                    if self.connection == Connection::Offline {
                        self.connection = Connection::Polling;
                        notices.push(Notice::Connection(Connection::Polling));
                    }
                }
                let events: Vec<Event> = body
                    .get("events")
                    .cloned()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default();
                let history = room.history.take().is_some();
                if let Some(first) = room.absorb(events) {
                    room.changed_at = now;
                    notices.push(Notice::Events(id.clone(), first));
                }
                // The page covered up to `read_to`, lines the player does
                // not hear included: a marker there keeps the client from
                // asking for them again.
                if let Some(to) = body["read_to"].as_u64() {
                    if to > room.read_seq() {
                        room.events.push(Event::marker(to));
                    }
                }
                let more = !history && body["more"] == true;
                if let Ok(info) = read_room(&body) {
                    self.take_info(info, now, notices);
                }
                if let Some(room) = self.rooms.get_mut(id) {
                    if more || room.behind() {
                        room.next_poll = None;
                    }
                    if room.info.phase.is_final() {
                        room.watched = false;
                    }
                }
            }
            Err(RoomsError::Offline) => {
                self.poll_failures += 1;
                if self.poll_failures >= 2 && self.connection != Connection::Offline {
                    self.connection = Connection::Offline;
                    notices.push(Notice::Connection(Connection::Offline));
                }
            }
            Err(RoomsError::RateLimited) | Err(RoomsError::Unavailable) => {
                // Try again at the quiet pace.
                room.next_poll = Some(now + POLL_QUIET);
            }
            Err(e) => {
                room.watched = false;
                notices.push(Notice::Dropped(id.clone(), e));
            }
        }
    }

    fn schedule_polls(&mut self, now: Instant) {
        let Some(config) = self.config.clone() else {
            return;
        };
        if self.token.is_none() {
            return;
        }
        let due: Vec<(RoomId, u64, Option<usize>)> = self
            .rooms
            .values()
            .filter(|r| r.watched && r.poll.is_none() && r.next_poll.is_none_or(|t| t <= now))
            .map(|r| (r.info.id.clone(), r.read_seq(), r.history))
            .collect();
        for (id, after, history) in due {
            if self.in_flight.len() >= MAX_IN_FLIGHT {
                break;
            }
            let url = match history {
                Some(last) => config.endpoint(&format!("rooms/{id}?last={last}")),
                None => config.endpoint(&format!("rooms/{id}?after={after}")),
            };
            let ticket = self.send(Request::Poll(id.clone()), Method::Get, url);
            if let Some(room) = self.rooms.get_mut(&id) {
                room.poll = Some(ticket);
                // A poll asked for (watch, refresh, more to read) counts
                // as activity: the next ones come at the active pace.
                if room.next_poll.is_none() {
                    room.changed_at = now;
                }
                let quiet = now.duration_since(room.changed_at) > QUIET_AFTER;
                room.next_poll = Some(now + if quiet { POLL_QUIET } else { POLL_ACTIVE });
            }
        }
    }

    /// Lines for the agent server's `screen` and for tests: each room's
    /// state in one line.
    pub fn describe(&self) -> Vec<String> {
        let mut out = vec![format!(
            "rooms: {}{}",
            match self.connection {
                Connection::Polling => "polling",
                Connection::Offline => "offline",
            },
            if self.in_flight.is_empty() {
                String::new()
            } else {
                format!(", {} in flight", self.in_flight.len())
            }
        )];
        for room in self.rooms.values() {
            out.push(format!(
                "room {} {}: {:?}, seq {} read {}{}{}",
                room.info.code,
                room.info.kind,
                room.info.phase,
                room.info.seq,
                room.read_seq(),
                if room.watched { ", watched" } else { "" },
                room.info
                    .next_seat()
                    .map(|s| format!(", next {s}"))
                    .unwrap_or_default()
            ));
        }
        out
    }
}

enum Method {
    Post(Value),
    Get,
}

#[cfg(not(feature = "http"))]
struct NoTransport;

#[cfg(not(feature = "http"))]
impl Transport for NoTransport {
    fn post(&self, _url: &str, _token: Option<&str>, _body: &Value) -> Result<Reply, String> {
        Err("built without the http feature".into())
    }
    fn get(&self, _url: &str, _token: Option<&str>) -> Result<Reply, String> {
        Err("built without the http feature".into())
    }
}

/// `text` as a URL query value: letters, digits and a few marks as they
/// are, everything else as UTF-8 percent escapes.
fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn read_room(body: &Value) -> Result<RoomInfo, RoomsError> {
    body.get("room")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .ok_or(RoomsError::Unavailable)
}
