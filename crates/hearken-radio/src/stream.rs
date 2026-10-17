//! The game's side of a station: a cheap handle, and the fields it shares
//! with the worker thread.

use std::fmt;
use std::io;
use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use hearken_audio::{Feed, FeedRead};

use crate::ring::Ring;
use crate::worker;

/// An internet radio station, playing on its own thread.
///
/// Opening returns at once; the station connects and buffers behind it.
/// Hand [`Stream::feed`] to `Audio::start_feed` to hear it. The worker
/// stops when this handle, its clones and the feed are all dropped.
#[derive(Clone)]
pub struct Stream(Arc<Handle>);

/// Where a station is up to. `state` never blocks, so poll it every frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamState {
    /// Connecting and buffering, before the first audio is ready; calling
    /// once more, if the first call fails for a reason worth retrying.
    Connecting,
    /// Buffered and playing.
    Playing,
    /// The station dropped and is being called again; what was buffered
    /// plays out, then silence. New audio is held until the prebuffer is
    /// rebuilt.
    Reconnecting,
    /// Given up, for the reason given. Open a new `Stream` to try again.
    Failed(String),
}

/// How patient a stream is. The defaults suit a real station over the
/// internet.
#[derive(Clone, Debug)]
pub struct StreamOptions {
    /// How long connecting, and every read after, may take.
    pub timeout: Duration,
    /// The wait before each reconnect; when they run out, the stream fails.
    /// A station that has never played gets only the first, and none when
    /// its host is unknown, refuses the connection or fails TLS.
    pub retry_delays: Vec<Duration>,
    /// Playback this long earns a dropped stream its full set of retries.
    pub healthy_after: Duration,
    /// This long with no decoded audio counts as a dropped stream. Waiting
    /// for room in a full buffer does not count.
    pub stall: Duration,
    /// How much audio to buffer before playing; kept below `buffer`.
    pub prebuffer: Duration,
    /// The most audio to hold. Once it is held, the station is read no
    /// further until the feed makes room, so a paused stream resumes where
    /// it paused.
    pub buffer: Duration,
    /// Who is calling, sent as the `User-Agent` of every request: a game
    /// names itself, so a station's logs say who its listeners are.
    pub user_agent: String,
}

/// The `User-Agent` a stream sends unless its options say otherwise.
pub const DEFAULT_USER_AGENT: &str = concat!("Hearken/", env!("CARGO_PKG_VERSION"));

/// The now-playing metadata exactly as the station last sent it, for a game
/// with its own rules for reading a title out of it. [`Stream::title`] is
/// Hearken's own reading of the same thing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawTitle {
    /// The latest ICY metadata block, its zero padding removed, in whatever
    /// bytes the station sent (UTF-8 from most, Latin-1 from some):
    /// `StreamTitle='Band - Song';StreamUrl='';`.
    Icy(Vec<u8>),
    /// The current Ogg, Opus or FLAC track's comments, each `KEY=value` with
    /// the key as the encoder wrote it.
    Comments(Vec<String>),
    /// The current HLS segment's `#EXTINF` after the colon:
    /// `duration,title`, the title empty when the segment has none.
    Extinf(String),
}

impl Default for StreamOptions {
    fn default() -> StreamOptions {
        StreamOptions {
            timeout: Duration::from_secs(10),
            retry_delays: [1, 2, 4, 8, 16].map(Duration::from_secs).to_vec(),
            healthy_after: Duration::from_secs(30),
            stall: Duration::from_secs(10),
            // A cushion against a slow network. A game that wants a quicker
            // tune-in lowers it: Freight Fate plays after 300 ms, in a
            // 1.5 s buffer.
            prebuffer: Duration::from_millis(1_500),
            buffer: Duration::from_secs(4),
            user_agent: DEFAULT_USER_AGENT.to_owned(),
        }
    }
}

impl Stream {
    /// Starts playing `url` with the default options.
    pub fn open(url: &str) -> Stream {
        Stream::open_with(url, StreamOptions::default())
    }

    /// Starts playing `url`. Never blocks: a bad URL or a failure to start
    /// shows up as [`StreamState::Failed`].
    pub fn open_with(url: &str, mut options: StreamOptions) -> Stream {
        // Only the feed makes room in a full buffer, and it is silent until
        // the stream plays: the prebuffer must come first.
        let below = options.buffer.saturating_sub(Duration::from_millis(1));
        options.prebuffer = options.prebuffer.min(below);
        let shared = Arc::new(Shared::new(url, &options));
        let worker = shared.clone();
        let url = url.to_owned();
        let spawned = thread::Builder::new()
            .name("hearken-radio".into())
            .spawn(move || worker::run(&url, &worker, &options));
        if spawned.is_err() {
            shared.set_state(StreamState::Failed("could not start".into()));
        }
        Stream(Arc::new(Handle { shared }))
    }

    /// Where the station is up to. Never blocks, so it can be polled every
    /// frame.
    pub fn state(&self) -> StreamState {
        self.0.shared.state()
    }

    /// The latest now-playing title, where the station sends one.
    pub fn title(&self) -> Option<String> {
        lock(&self.0.shared.title).clone()
    }

    /// The latest now-playing metadata as the station sent it, before
    /// Hearken reads a title out of it; `None` until any arrives.
    pub fn raw_title(&self) -> Option<RawTitle> {
        lock(&self.0.shared.raw_title).clone()
    }

    /// The audio, for `Audio::start_feed`. Silent until the stream first
    /// plays, and after a reconnect until the prebuffer is rebuilt; between,
    /// whatever is buffered.
    pub fn feed(&self) -> Arc<dyn Feed> {
        self.0.clone()
    }
}

impl fmt::Debug for Stream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stream")
            .field("url", &self.0.shared.url)
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

/// What every [`Stream`] clone and the feed hold. The worker holds only the
/// [`Shared`] fields, so dropping the last handle is what stops it.
struct Handle {
    shared: Arc<Shared>,
}

impl Feed for Handle {
    fn read(&self, out: &mut [[f32; 2]]) -> FeedRead {
        self.shared.read(out)
    }
}

impl Drop for Handle {
    /// Tells the worker to stop and returns at once; the worker is never
    /// joined, and notices within one read.
    fn drop(&mut self) {
        *lock(&self.shared.closed) = true;
        self.shared.wake.notify_all();
        // Through the ring's lock, so a push between seeing the ring full and
        // waiting still hears this.
        drop(lock(&self.shared.ring));
        self.shared.room.notify_all();
    }
}

/// Why one connection to a station ended.
pub(crate) enum Fault {
    /// Calling again would not help: a 4xx, nothing playable.
    Fatal(String),
    /// Nothing answered: no such host, nothing listening, or TLS failed.
    /// Fatal until the station has played, so a dead or mistyped one is
    /// reported in seconds; after, a blip in the network, worth calling
    /// again.
    Unreachable(String),
    /// Worth calling again: a timeout, a reset, a 5xx, the stream ending or
    /// stalling.
    Retry(String),
}

/// A socket that times out or resets is worth calling again. Its kind
/// ("timed out") says enough; the OS's message is a paragraph.
impl From<io::Error> for Fault {
    fn from(error: io::Error) -> Fault {
        Fault::Retry(error.kind().to_string())
    }
}

/// The fields the game, the audio thread and the worker share. Every lock is
/// held only to copy in or out, never across I/O. `closed` may be locked
/// inside `ring`, never the other way round.
pub(crate) struct Shared {
    /// The station, for the log.
    url: String,
    /// Buffered this much, the stream plays.
    prebuffer: Duration,
    state: Mutex<StreamState>,
    /// When the station was last called, and from when that call began to
    /// play until its latest audio: the log's timing, and whether a call
    /// that dropped had played long enough to earn back its retries.
    call: Mutex<(Instant, Option<(Instant, Instant)>)>,
    title: Mutex<Option<String>>,
    raw_title: Mutex<Option<RawTitle>>,
    ring: Mutex<Ring>,
    /// Signalled when the feed takes audio or the stream closes, for a push
    /// waiting on a full ring.
    room: Condvar,
    /// Set when the stream plays: until then the feed stays silent, so the
    /// prebuffer is heard as a cushion, not drained as it fills. Through a
    /// drop it stays set while the buffer plays out, and is cleared when
    /// the new call's audio arrives.
    started: AtomicBool,
    closed: Mutex<bool>,
    wake: Condvar,
}

impl Shared {
    fn new(url: &str, options: &StreamOptions) -> Shared {
        Shared {
            url: url.to_owned(),
            prebuffer: options.prebuffer,
            state: Mutex::new(StreamState::Connecting),
            call: Mutex::new((Instant::now(), None)),
            title: Mutex::new(None),
            raw_title: Mutex::new(None),
            ring: Mutex::new(Ring::new(options.buffer)),
            room: Condvar::new(),
            started: AtomicBool::new(false),
            closed: Mutex::new(false),
            wake: Condvar::new(),
        }
    }

    pub(crate) fn state(&self) -> StreamState {
        lock(&self.state).clone()
    }

    /// Logs a failure, and the time from calling to playing; never under a
    /// lock, so a slow log cannot hold up the game reading the state.
    pub(crate) fn set_state(&self, state: StreamState) {
        if let StreamState::Failed(reason) = &state {
            log::info!("radio: {} failed ({reason})", self.url);
        }
        let playing = state == StreamState::Playing;
        let was = mem::replace(&mut *lock(&self.state), state);
        if playing && was != StreamState::Playing {
            self.started.store(true, Ordering::Release);
            let called = {
                let mut call = lock(&self.call);
                let now = Instant::now();
                call.1 = Some((now, now));
                call.0
            };
            let after = called.elapsed().as_millis();
            log::info!("radio: {} playing after {after} ms", self.url);
        }
    }

    /// The worker is calling the station, again or for the first time: the
    /// call's timings start now.
    pub(crate) fn calling(&self) {
        *lock(&self.call) = (Instant::now(), None);
    }

    /// How long the current call played, up to its latest audio: the dead
    /// air before a timeout or stall is not good playback. Zero until it
    /// plays.
    pub(crate) fn played(&self) -> Duration {
        let span = lock(&self.call).1;
        span.map_or(Duration::ZERO, |(from, to)| to.duration_since(from))
    }

    pub(crate) fn set_title(&self, title: Option<String>) {
        *lock(&self.title) = title;
    }

    pub(crate) fn set_raw_title(&self, raw: RawTitle) {
        *lock(&self.raw_title) = Some(raw);
    }

    /// Adds decoded frames at `rate`, which must not be 0, once the ring has
    /// room: the station is read no faster than it is heard, as BASS did.
    /// Returns at once when the stream closes. Plays once the prebuffer is
    /// buffered.
    pub(crate) fn push(&self, rate: u32, frames: Vec<[f32; 2]>) {
        let ring = lock(&self.ring);
        let mut ring = self
            .room
            .wait_while(ring, |ring| ring.full() && !self.closed())
            .unwrap_or_else(PoisonError::into_inner);
        ring.push(rate, frames);
        let ready = ring.buffered() >= self.prebuffer;
        drop(ring);
        if ready {
            self.set_state(StreamState::Playing);
        } else if *lock(&self.state) == StreamState::Reconnecting {
            // Back from a drop, the feed is silent until the prebuffer is
            // rebuilt, so it is heard as a cushion again rather than played
            // packet by packet as it lands.
            self.started.store(false, Ordering::Release);
        }
        if let Some((_, latest)) = &mut lock(&self.call).1 {
            *latest = Instant::now();
        }
    }

    /// Every handle is gone: the worker should stop.
    pub(crate) fn closed(&self) -> bool {
        *lock(&self.closed)
    }

    /// Waits `d`, or less if the stream closes meanwhile. False if it closed.
    pub(crate) fn sleep(&self, d: Duration) -> bool {
        let closed = lock(&self.closed);
        let (closed, _) = self
            .wake
            .wait_timeout_while(closed, d, |closed| !*closed)
            .unwrap_or_else(PoisonError::into_inner);
        !*closed
    }

    fn read(&self, out: &mut [[f32; 2]]) -> FeedRead {
        if !self.started.load(Ordering::Acquire) {
            return FeedRead {
                frames: 0,
                sample_rate: 0,
            };
        }
        let read = lock(&self.ring).read(out);
        // Never blocks: wakes the worker, if it is waiting for room.
        if read.frames > 0 {
            self.room.notify_one();
        }
        read
    }
}

/// Locks through poisoning: a panicked worker is reported as `Failed`, and
/// must not take the audio thread down with it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_feed_is_silent_until_the_stream_first_plays() {
        let shared = Shared::new("test", &StreamOptions::default());
        shared.push(1_000, vec![[0.5; 2]; 100]);
        let mut out = [[0.0; 2]; 10];
        assert_eq!(shared.read(&mut out).frames, 0);
        shared.set_state(StreamState::Playing);
        shared.set_state(StreamState::Reconnecting);
        assert_eq!(shared.read(&mut out).frames, 10, "the buffer plays out");
    }

    #[test]
    fn a_reconnect_is_heard_once_the_prebuffer_is_rebuilt() {
        let options = StreamOptions {
            prebuffer: Duration::from_millis(100),
            ..StreamOptions::default()
        };
        let shared = Shared::new("test", &options);
        // A tenth of a second at 1 kHz.
        shared.push(1_000, vec![[0.5; 2]; 100]);
        assert_eq!(shared.state(), StreamState::Playing);
        let mut out = [[0.0; 2]; 200];
        assert_eq!(shared.read(&mut out).frames, 100);
        shared.set_state(StreamState::Reconnecting);
        shared.push(1_000, vec![[0.5; 2]; 50]);
        assert_eq!(shared.read(&mut out).frames, 0, "held");
        shared.push(1_000, vec![[0.5; 2]; 50]);
        assert_eq!(shared.state(), StreamState::Playing);
        assert_eq!(shared.read(&mut out).frames, 100);
    }
}
