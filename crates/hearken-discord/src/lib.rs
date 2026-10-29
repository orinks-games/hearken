//! Discord Rich Presence for a Hearken game: the "Playing ..." status a
//! player's friends see in Discord.
//!
//! Generalized from Freight Fate's service. Nothing here knows about any
//! game: the game gives its own Discord application ID, and its screens
//! report a [`Presence`] (two short, player-facing lines). This crate owns
//! the IPC pipe, the throttle, the idle clear and the shutdown.
//!
//! Everything is best-effort and silent. When Discord is closed, not
//! installed, quits mid-session or refuses the application ID, the game
//! starts, plays and quits exactly as it would without it. All pipe I/O runs
//! on a worker thread; the game loop only stores the latest presence under
//! a lock. Failures are logged at debug level and never reach the player.
//!
//! Build with the `ipc` feature for the real client. Without it the service
//! stays dormant, so a game's presence code still compiles.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Discord cuts both visible lines at 128 characters.
pub const MAX_FIELD_LEN: usize = 128;

/// Discord rate-limits presence updates (about 5 per 20 seconds). Changes
/// are coalesced and never sent more than once per this many seconds;
/// identical presences are dropped.
pub const MIN_UPDATE_INTERVAL_S: f64 = 15.0;

/// After this long with an unchanged presence the status comes down, so a
/// player who walked away from a paused game is not shown playing it all
/// evening. Any change puts it straight back up.
pub const IDLE_CLEAR_S: f64 = 30.0 * 60.0;

/// How often the worker wakes on its own, which also flushes a throttled
/// change.
const WORKER_TICK_S: f64 = MIN_UPDATE_INTERVAL_S;

/// Seconds between connection attempts while Discord is not running.
pub const RECONNECT_INTERVAL_S: f64 = 30.0;

/// How long quitting, or switching the feature off, waits for a worker
/// that holds a live Discord client, so its status is cleared before the
/// pipe goes. A worker with no client is never waited for (see
/// `Inner::wait_for_worker`).
const WORKER_JOIN: Duration = Duration::from_secs(2);

/// What the player is doing, as Discord shows it under the game's name.
///
/// `activity` is the first line ("In the lobby"); `detail` an optional
/// second ("Playing Belfry"). Both must be plain, public game content:
/// never a save path, a player's typed name, or debug data. Equality drives
/// de-duplication, so reporting the same presence every frame costs nothing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Presence {
    pub activity: String,
    pub detail: String,
}

impl Presence {
    /// A one-line presence.
    pub fn new(activity: impl Into<String>) -> Self {
        Presence {
            activity: activity.into(),
            detail: String::new(),
        }
    }

    /// Adds the second line.
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }
}

fn truncate(text: &str, limit: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= limit {
        return text;
    }
    let head: String = text.chars().take(limit.saturating_sub(1)).collect();
    format!("{}…", head.trim_end())
}

/// One SET_ACTIVITY's fields: `details` always, `state` when the second
/// line is not empty, `start` (Unix seconds) for an elapsed-time counter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityPayload {
    pub details: String,
    pub state: Option<String>,
    pub start: Option<i64>,
}

/// A [`Presence`] as Discord's update payload: empty lines left out, both
/// lines clamped to [`MAX_FIELD_LEN`].
pub fn format_activity(presence: &Presence, start: Option<f64>) -> ActivityPayload {
    let detail = truncate(&presence.detail, MAX_FIELD_LEN);
    ActivityPayload {
        details: truncate(&presence.activity, MAX_FIELD_LEN),
        state: (!detail.is_empty()).then_some(detail),
        start: start.map(|s| s.trunc() as i64),
    }
}

/// Judges one IPC reply: a CLOSE frame (opcode 2) or an `ERROR` event is
/// Discord refusing the command; anything else acknowledges it.
pub fn check_reply(op: u32, reply: &serde_json::Value) -> Result<(), String> {
    if op == 2 {
        return Err(format!("Discord closed the connection: {reply}"));
    }
    if reply.get("evt").and_then(|e| e.as_str()) == Some("ERROR") {
        let message = reply
            .pointer("/data/message")
            .and_then(|m| m.as_str())
            .unwrap_or("no message");
        return Err(format!("Discord refused the command: {message}"));
    }
    Ok(())
}

/// The part of a Discord RPC client this service uses. Tests supply a fake.
pub trait RpcClient: Send {
    fn connect(&mut self) -> Result<(), String>;
    fn update(&mut self, payload: &ActivityPayload) -> Result<(), String>;
    fn clear(&mut self) -> Result<(), String>;
    fn close(&mut self) -> Result<(), String>;
}

/// Builds a client for an application ID.
pub type RpcFactory = Arc<dyn Fn(&str) -> Result<Box<dyn RpcClient>, String> + Send + Sync>;

/// Seconds on some clock. The service reads time only through this, so
/// tests can step it.
pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

/// Seconds since the service's first read of the monotonic clock.
pub fn monotonic_clock() -> Clock {
    let start = Instant::now();
    Arc::new(move || start.elapsed().as_secs_f64())
}

#[cfg(feature = "ipc")]
mod ipc {
    use discord_rich_presence::activity::{Activity, Timestamps};
    use discord_rich_presence::{DiscordIpc, DiscordIpcClient};

    use super::{check_reply, ActivityPayload, RpcClient};

    /// The real client over Discord's local IPC pipe.
    pub struct DiscordRpcClient {
        client: DiscordIpcClient,
    }

    impl DiscordRpcClient {
        pub fn new(app_id: &str) -> Self {
            DiscordRpcClient {
                client: DiscordIpcClient::new(app_id),
            }
        }

        /// Reads Discord's answer to the command just sent. The IPC crate
        /// never reads it, so a long session would pile one unread reply
        /// per update into the pipe, and a refused command (a rate limit, a
        /// payload Discord would not take) would fail silently with the old
        /// status left up (Freight Fate, 2026-10-03: a driver shown at 0%
        /// for seven hours). Reading it keeps the pipe drained and turns a
        /// refusal into an error, which reconnects.
        fn await_reply(&mut self) -> Result<(), String> {
            let (op, reply) = self.client.recv().map_err(|e| e.to_string())?;
            check_reply(op, &reply)
        }
    }

    impl RpcClient for DiscordRpcClient {
        fn connect(&mut self) -> Result<(), String> {
            self.client.connect().map_err(|e| e.to_string())
        }

        fn update(&mut self, payload: &ActivityPayload) -> Result<(), String> {
            let mut activity = Activity::new().details(payload.details.as_str());
            if let Some(state) = &payload.state {
                activity = activity.state(state.as_str());
            }
            if let Some(start) = payload.start {
                activity = activity.timestamps(Timestamps::new().start(start));
            }
            self.client
                .set_activity(activity)
                .map_err(|e| e.to_string())?;
            self.await_reply()
        }

        fn clear(&mut self) -> Result<(), String> {
            self.client.clear_activity().map_err(|e| e.to_string())?;
            self.await_reply()
        }

        fn close(&mut self) -> Result<(), String> {
            self.client.close().map_err(|e| e.to_string())
        }
    }
}

#[cfg(feature = "ipc")]
pub use ipc::DiscordRpcClient;

/// The real client's factory, or `None` when built without the `ipc`
/// feature (the service then stays dormant).
pub fn default_rpc_factory() -> Option<RpcFactory> {
    #[cfg(feature = "ipc")]
    {
        Some(Arc::new(|app_id: &str| {
            Ok(Box::new(DiscordRpcClient::new(app_id)) as Box<dyn RpcClient>)
        }))
    }
    #[cfg(not(feature = "ipc"))]
    {
        None
    }
}

/// Clear then close, each step best-effort.
fn teardown(mut rpc: Box<dyn RpcClient>, send_close: bool) {
    if send_close {
        let _ = rpc.clear();
        let _ = rpc.close();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// A settable flag a thread can wait on with a timeout.
#[derive(Default)]
struct Event {
    set: Mutex<bool>,
    cv: Condvar,
}

impl Event {
    fn set(&self) {
        *lock(&self.set) = true;
        self.cv.notify_all();
    }

    fn clear(&self) {
        *lock(&self.set) = false;
    }

    fn is_set(&self) -> bool {
        *lock(&self.set)
    }

    fn wait_seconds(&self, seconds: f64) {
        let secs = if seconds.is_finite() && seconds > 0.0 {
            seconds
        } else {
            0.0
        };
        let guard = lock(&self.set);
        if *guard {
            return;
        }
        let _ = self
            .cv
            .wait_timeout_while(guard, Duration::from_secs_f64(secs), |set| !*set);
    }
}

fn join_with_timeout(handle: JoinHandle<()>, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !handle.is_finished() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    if handle.is_finished() {
        let _ = handle.join();
    }
}

/// Everything [`DiscordPresence::new`] takes.
pub struct DiscordPresenceOptions {
    /// The game's Discord application ID, from
    /// <https://discord.com/developers/applications>. Discord shows the
    /// application's name as the game being played.
    pub app_id: String,
    /// The player's setting.
    pub enabled: bool,
    /// Show an elapsed-time counter from when the session started.
    pub show_elapsed: bool,
    pub min_interval_s: f64,
    /// Seconds of an unchanged presence before it is taken down.
    pub idle_clear_s: f64,
    pub clock: Clock,
    /// `None` leaves the service dormant.
    pub rpc_factory: Option<RpcFactory>,
    /// Unix seconds the session started; `None` means now.
    pub session_start: Option<f64>,
    /// False drives the same logic on the caller's thread, for tests.
    pub threaded: bool,
}

impl DiscordPresenceOptions {
    /// The real, threaded, enabled service for `app_id`.
    pub fn new(app_id: impl Into<String>) -> Self {
        DiscordPresenceOptions {
            app_id: app_id.into(),
            enabled: true,
            show_elapsed: true,
            min_interval_s: MIN_UPDATE_INTERVAL_S,
            idle_clear_s: IDLE_CLEAR_S,
            clock: monotonic_clock(),
            rpc_factory: default_rpc_factory(),
            session_start: None,
            threaded: true,
        }
    }
}

#[derive(Default)]
struct State {
    desired: Option<Presence>,
    last_sent: Option<Presence>,
    last_send_t: Option<f64>,
    /// When `desired` last changed; the idle clock reads from here.
    desired_changed_t: Option<f64>,
    /// Whether Discord is showing our activity now.
    showing: bool,
    rpc: Option<Box<dyn RpcClient>>,
    last_connect_attempt: Option<f64>,
}

struct Inner {
    available: bool,
    enabled: AtomicBool,
    app_id: String,
    min_interval: f64,
    idle_clear: f64,
    clock: Clock,
    rpc_factory: Option<RpcFactory>,
    session_start: Option<f64>,
    threaded: bool,
    state: Mutex<State>,
    wake: Event,
    stop: Event,
    thread: Mutex<Option<JoinHandle<()>>>,
    started: AtomicBool,
}

/// The best-effort Rich Presence service.
///
/// [`update`](Self::update) returns at once and never blocks the game loop.
/// A worker thread owns the pipe: it connects (retrying every
/// [`RECONNECT_INTERVAL_S`] while Discord is closed), sends the latest
/// presence subject to de-duplication and throttling, and reconnects if
/// Discord goes away. [`shutdown`](Self::shutdown) clears the status and
/// stops the worker.
#[derive(Clone)]
pub struct DiscordPresence {
    inner: Arc<Inner>,
}

impl DiscordPresence {
    pub fn new(options: DiscordPresenceOptions) -> Self {
        let app_id = options.app_id.trim().to_string();
        // Live only when there is a client to talk through and an ID to
        // give it.
        let available = options.rpc_factory.is_some() && !app_id.is_empty();
        let session_start = options.show_elapsed.then(|| {
            options.session_start.unwrap_or_else(|| {
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs_f64())
                    .unwrap_or(0.0)
            })
        });
        DiscordPresence {
            inner: Arc::new(Inner {
                available,
                enabled: AtomicBool::new(options.enabled && available),
                app_id,
                min_interval: options.min_interval_s.max(0.0),
                idle_clear: options.idle_clear_s.max(1.0),
                clock: options.clock,
                rpc_factory: options.rpc_factory,
                session_start,
                threaded: options.threaded,
                state: Mutex::new(State::default()),
                wake: Event::default(),
                stop: Event::default(),
                thread: Mutex::new(None),
                started: AtomicBool::new(false),
            }),
        }
    }

    /// Whether this service can ever reach Discord: built with `ipc` and
    /// given an application ID.
    pub fn available(&self) -> bool {
        self.inner.available
    }

    pub fn enabled(&self) -> bool {
        self.inner.enabled()
    }

    /// Whether the worker holds a live connection to Discord.
    pub fn connected(&self) -> bool {
        lock(&self.inner.state).rpc.is_some()
    }

    /// Starts the worker. Safe to call when disabled, or twice.
    pub fn start(&self) {
        if !self.enabled() || self.inner.started.swap(true, Ordering::SeqCst) {
            return;
        }
        self.inner.stop.clear();
        if self.inner.threaded {
            let inner = Arc::clone(&self.inner);
            let handle = thread::Builder::new()
                .name("discord-presence".to_string())
                .spawn(move || inner.run())
                .ok();
            *lock(&self.inner.thread) = handle;
        } else {
            self.inner.pump();
        }
    }

    /// One connect-then-maybe-send cycle now, as the worker does on its
    /// own heartbeat. For tests of the unthreaded service.
    pub fn tick(&self) {
        self.inner.pump();
    }

    /// Reports the player's latest activity. Non-blocking; a presence equal
    /// to the last one is dropped before it even wakes the worker. `None`
    /// leaves the status as it is.
    pub fn update(&self, presence: Option<Presence>) {
        let Some(presence) = presence else { return };
        if !self.enabled() {
            return;
        }
        {
            let mut st = lock(&self.inner.state);
            if st.desired.as_ref() == Some(&presence) {
                return;
            }
            st.desired = Some(presence);
            st.desired_changed_t = Some((self.inner.clock)());
        }
        if self.inner.threaded {
            self.inner.wake.set();
        } else {
            self.inner.pump();
        }
    }

    /// Clears the status and stops the worker. Never panics; safe to call
    /// twice or without `start`.
    pub fn shutdown(&self) {
        self.inner.stop.set();
        self.inner.wake.set();
        let handle = lock(&self.inner.thread).take();
        if let Some(handle) = handle {
            self.inner.wait_for_worker(handle);
        }
        self.inner.close();
        self.inner.started.store(false, Ordering::SeqCst);
    }

    /// The player's switch, at runtime (from Settings).
    pub fn set_enabled(&self, enabled: bool) {
        let enabled = enabled && self.inner.available;
        if enabled == self.enabled() {
            return;
        }
        self.inner.enabled.store(enabled, Ordering::SeqCst);
        if enabled {
            // Re-show the last presence, reconnecting at once rather than
            // waiting out the backoff. Throwing the switch also proves the
            // player is here, so the idle clock starts over.
            {
                let mut st = lock(&self.inner.state);
                st.last_sent = None;
                st.last_connect_attempt = None;
                st.desired_changed_t = Some((self.inner.clock)());
            }
            self.start();
        } else {
            let was_started = self.inner.started.load(Ordering::SeqCst);
            self.inner.stop.set();
            self.inner.wake.set();
            let handle = lock(&self.inner.thread).take();
            if let (Some(handle), true) = (handle, was_started) {
                self.inner.wait_for_worker(handle);
            }
            self.inner.close();
            self.inner.started.store(false, Ordering::SeqCst);
            self.inner.stop.clear();
        }
    }
}

impl Inner {
    fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// Waits for the worker only while waiting achieves something.
    ///
    /// A worker holding a live client must be off the pipe before the
    /// status is cleared, or it re-shows what was just cleared. A worker
    /// with no client has nothing to race with, and it is exactly then that
    /// waiting is expensive: Discord's handshake is a blocking pipe read
    /// with no timeout, and Discord stops answering handshakes for a while
    /// after a game is launched several times quickly. A worker parked in
    /// that read would cost the player seconds of silence at quit.
    fn wait_for_worker(&self, handle: JoinHandle<()>) {
        if lock(&self.state).rpc.is_none() {
            return;
        }
        join_with_timeout(handle, WORKER_JOIN);
    }

    fn run(&self) {
        // `enabled` as well as `stop`: switching the feature off sets the
        // stop flag, tidies up and clears it again, and a worker parked in
        // the handshake through all of that must still leave.
        while !self.stop.is_set() && self.enabled() {
            self.pump();
            self.wake.wait_seconds(self.worker_wait());
            self.wake.clear();
        }
    }

    /// Short while a change waits out the throttle, else the heartbeat.
    fn worker_wait(&self) -> f64 {
        let st = lock(&self.state);
        let pending = st.desired.is_some() && st.desired != st.last_sent;
        if pending {
            if let Some(last) = st.last_send_t {
                let remaining = self.min_interval - ((self.clock)() - last);
                if remaining > 0.0 {
                    return remaining.min(WORKER_TICK_S);
                }
            }
        }
        WORKER_TICK_S
    }

    /// One connect-then-maybe-send cycle. Swallows every RPC error.
    fn pump(&self) {
        if self.stop.is_set() || !self.enabled() {
            return;
        }
        if !self.ensure_connected() {
            return;
        }
        // The pipe write runs outside the lock, so a slow pipe never stalls
        // the game loop's next `update`.
        let (desired, now, mut rpc) = {
            let mut st = lock(&self.state);
            let Some(desired) = st.desired.clone() else {
                return;
            };
            let now = (self.clock)();
            let idle_for = st.desired_changed_t.map_or(0.0, |t| now - t);
            if idle_for >= self.idle_clear {
                // `last_sent` keeps the idle presence, so the next change
                // still reads as a change and re-shows it.
                drop(st);
                self.hide(now);
                return;
            }
            if st.last_sent.as_ref() == Some(&desired) {
                return;
            }
            if let Some(last) = st.last_send_t {
                if now - last < self.min_interval {
                    return; // the worker re-checks once the window closes
                }
            }
            (desired, now, st.rpc.take())
        };
        let payload = format_activity(&desired, self.session_start);
        let result = match rpc.as_mut() {
            Some(rpc) => rpc.update(&payload),
            None => Err("not connected".to_string()),
        };
        let mut st = lock(&self.state);
        match result {
            Ok(()) => {
                st.rpc = rpc;
                st.last_sent = Some(desired);
                st.last_send_t = Some(now);
                st.showing = true;
            }
            Err(e) => {
                log::debug!("Discord presence update failed; will reconnect: {e}");
                st.last_send_t = None;
                st.last_sent = None;
                drop(st);
                if let Some(rpc) = rpc {
                    teardown(rpc, true);
                }
            }
        }
    }

    fn ensure_connected(&self) -> bool {
        let Some(factory) = &self.rpc_factory else {
            return false;
        };
        {
            let mut st = lock(&self.state);
            if st.rpc.is_some() {
                return true;
            }
            let now = (self.clock)();
            if let Some(last) = st.last_connect_attempt {
                if now - last < RECONNECT_INTERVAL_S {
                    return false; // Discord is probably not running
                }
            }
            st.last_connect_attempt = Some(now);
        }
        let mut rpc = match factory(&self.app_id) {
            Ok(rpc) => rpc,
            Err(e) => {
                log::debug!("Discord not available; presence stays hidden: {e}");
                return false;
            }
        };
        if let Err(e) = rpc.connect() {
            log::debug!("Discord not available; presence stays hidden: {e}");
            teardown(rpc, false);
            return false;
        }
        // The handshake can outlast the reason for it: quitting does not
        // wait on a worker with no client, and switching the feature off
        // clears the stop flag again. A reply after either is dropped, or it
        // would show the player in a game they shut, or after they said no.
        if self.stop.is_set() || !self.enabled() {
            teardown(rpc, false);
            return false;
        }
        lock(&self.state).rpc = Some(rpc);
        true
    }

    /// Takes an idle presence down, once.
    fn hide(&self, now: f64) {
        let mut rpc = {
            let mut st = lock(&self.state);
            if !st.showing {
                return;
            }
            st.rpc.take()
        };
        let result = match rpc.as_mut() {
            Some(rpc) => rpc.clear(),
            None => Err("not connected".to_string()),
        };
        let mut st = lock(&self.state);
        match result {
            Ok(()) => {
                st.rpc = rpc;
                st.showing = false;
                // A clear spends the same rate-limit budget as an update.
                st.last_send_t = Some(now);
            }
            Err(e) => {
                log::debug!("Discord presence clear failed; will reconnect: {e}");
                st.last_send_t = None;
                st.last_sent = None;
                st.showing = false;
                drop(st);
                if let Some(rpc) = rpc {
                    teardown(rpc, true);
                }
            }
        }
    }

    fn close(&self) {
        let rpc = {
            let mut st = lock(&self.state);
            st.last_send_t = None;
            st.last_sent = None;
            st.showing = false;
            st.rpc.take()
        };
        if let Some(rpc) = rpc {
            teardown(rpc, true);
        }
    }
}
