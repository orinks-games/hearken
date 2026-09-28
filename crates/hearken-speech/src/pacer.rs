//! Pacing, repeat suppression, and priority for the gameplay event voice.
//!
//! The event channel is a queue the game cannot inspect: it hands lines to a
//! voice that speaks them in submission order, and nothing in the speech
//! backend reports what is still waiting. Three things go wrong when
//! gameplay events talk straight into it:
//!
//! * **The same moment said several times.** A code path that notices
//!   something runs every frame, so one event arrives as three identical
//!   lines inside half a second.
//! * **A standing condition read out forever.** A warning stays true for as
//!   long as its cause does, so it fires again every few seconds for the
//!   rest of the session. The player needs it when it starts and again when
//!   it gets worse, not on a loop.
//! * **The line that mattered buried.** The line the player needed waits
//!   behind chatter and is still waiting once the moment it was about has
//!   passed.
//!
//! [`EventSpeechPacer`] answers all three from one place.
//!
//! Its core job is the backlog projection: each submitted line extends a
//! projected clear time by its estimated speaking duration, so the game
//! knows roughly when the voice falls silent. A queued line that would START
//! speaking more than its priority's budget after the moment it described is
//! by definition stale, and the caller delivers it interrupting instead,
//! which purges the dead backlog. Interrupting lines reset the projection to
//! truth, so estimate drift never outlives one backlog.
//!
//! On top of that it remembers what the player has already heard (so a
//! repeat inside `repeat_window_s` never reaches the voice twice), what each
//! standing condition last said (so it speaks again only when it has
//! something new to say), and how long a line of each priority is willing to
//! wait -- an [`EventPriority::Important`] line waits a moment behind
//! chatter and then goes ahead of it.
//!
//! The purge that delivers an interrupting line cuts both ways: it flushes
//! dead chatter, but it also lands on whatever the voice was mid-way through
//! -- and when that was an `Important` or `Critical` line, the player lost
//! an instruction, not colour. The pacer keeps the newest such line
//! alongside its projected finish time, and an interrupt arriving before
//! that moment hands the line back to the caller to queue right behind the
//! interrupting one: the interrupting line first, then the line it stepped
//! on. Only when the cut landed EARLY, though -- a line the player had
//! mostly heard is dropped rather than said again whole, because a repeat
//! from the top reads as a stutter. See `mostly_heard_fraction`.
//!
//! Durations are estimated from text length at a conservative default
//! speaking rate. A faster voice just flushes a little less eagerly than it
//! could; a slower one flushes a little late but stays bounded -- either way
//! the player never again waits through a paragraph of expired narration.
//!
//! Ported from Freight Fate's `speech_pacing.rs` (commit `c5d3ef8d`), itself
//! a port of `freight_fate/speech_pacing.py`, generalised for Hearken: no
//! `SpeechCategory`/`Disposition` verbosity ladder (a per-game concern, not
//! part of this engine), no delivery receipts (`DeliveryStatus` and the
//! methods that tracked it), and every tuning value that was a private
//! associated constant is now a field on [`PacerConfig`] a game can change.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Instant;

/// Seconds since the first call: the default pacer clock, the equivalent of
/// Python's `time.monotonic`.
pub fn monotonic_seconds() -> f64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// How long a line is willing to wait behind what is already speaking.
///
/// `Ambient` is background chatter -- flavor, colour, routine status.
/// Missing one costs the player nothing.
///
/// `Important` is the line the player needs to act on soon; it waits a
/// moment behind chatter, then goes ahead of it.
///
/// `Critical` is a warning to act on now. It is always delivered
/// interrupting, so it never consults the wait budget at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EventPriority {
    Ambient,
    Important,
    Critical,
}

/// Tuning for [`EventSpeechPacer`]. Every value here was a private
/// associated constant in the ported source; each is now a field so a game
/// can change it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PacerConfig {
    /// How long an `Ambient` line is willing to wait behind a backlog before
    /// a flush purges the channel and speaks it now.
    pub ambient_wait_s: f64,
    /// The same wait budget for an `Important` line -- much shorter, so it
    /// goes ahead of chatter rather than behind it.
    pub important_wait_s: f64,
    /// Per-utterance pause before the voice gets going, folded into the
    /// estimated duration of every line.
    pub base_utterance_s: f64,
    /// Estimated speaking rate used to project a line's duration from its
    /// length, at a conservative default voice.
    pub chars_per_s: f64,
    /// Two identical lines this close together are one thing happening, not
    /// two, whichever code path noticed it.
    pub repeat_window_s: f64,
    /// Bound on the remembered-lines map, so a long session cannot grow it
    /// without limit.
    pub recent_limit: usize,
    /// How long a remembered line is kept once the map is over
    /// `recent_limit`.
    pub recent_memory_s: f64,
    /// After a line is rescued once, further cuts within this window drop it
    /// instead of replaying it -- long enough to cover a burst of stacked
    /// urgent lines, short enough that the same words minutes later are a
    /// new moment that earns its own rescue.
    pub rescue_once_window_s: f64,
    /// How much of a cut line the player must already have heard for the
    /// pacer to drop it rather than hand it back to be said again whole.
    /// The fraction is elapsed speaking time over the line's estimated
    /// duration, so it is only as exact as the duration model.
    pub mostly_heard_fraction: f64,
}

impl Default for PacerConfig {
    fn default() -> Self {
        PacerConfig {
            ambient_wait_s: 3.0,
            important_wait_s: 0.8,
            base_utterance_s: 0.4,
            chars_per_s: 13.0,
            repeat_window_s: 2.5,
            recent_limit: 256,
            recent_memory_s: 300.0,
            rescue_once_window_s: 30.0,
            mostly_heard_fraction: 0.5,
        }
    }
}

/// A source of monotonic seconds; injectable so tests can drive the
/// projection without sleeping.
pub type Clock = Box<dyn FnMut() -> f64>;

/// "Is the moment this line described still live?" -- consulted before a
/// cut line is handed back.
pub type Valid = Box<dyn Fn() -> bool>;

/// An `Important` or `Critical` line handed back for requeueing.
pub type Cut = (String, EventPriority);

// The newest `Important` or `Critical` line submitted, with the
// projection's estimate of when it finishes speaking.
struct Protected {
    text: String,
    priority: EventPriority,
    done_at: f64,
    valid: Option<Valid>,
}

/// Keeps the event voice from performing the past.
///
/// See the module docs for the whole picture. The caller's contract:
///
/// * `is_repeat` decides whether the player has already heard this; a true
///   means say nothing at all.
/// * `note_spoken` records a line that did reach the voice.
/// * `is_silenced_repeat`/`note_silenced` are the same pair for a line the
///   caller silenced some other way (an earcon substituted, or the line cut
///   outright) -- a private namespace so a silenced occurrence can dedupe
///   itself without ever registering as something `is_repeat` would
///   recognise as heard.
/// * `note_interrupt` for an interrupting line (it purges the channel), or
///   `should_flush` for a queued one -- true there means the backlog has
///   gone stale and this line must be submitted interrupting instead.
/// * `note_interrupt` may hand back the `Important` or `Critical` line the
///   purge cut off mid-sentence; the caller resubmits it queued
///   (`note_queued`) so the player still hears it, right behind the line
///   that cut it.
/// * `note_channel_purged` when speech outside the pacer's view (a reply on
///   a shared voice) purges the channel -- same hand-back.
/// * `pause`/`resume` around a screen that takes the player out of
///   gameplay.
pub struct EventSpeechPacer {
    config: PacerConfig,
    clock: Clock,
    clear_at: f64,
    /// text -> when the player last heard it.
    recent: HashMap<String, f64>,
    /// condition key -> the last thing said about it.
    conditions: HashMap<String, String>,
    // The same two maps, but for occurrences the caller silenced (an earcon
    // or nothing, never the words). Kept separate from
    // `recent`/`conditions` on purpose: those two belong to what the player
    // actually heard, and `is_repeat` consults them to decide whether a
    // genuinely spoken line would be news. If a silenced occurrence wrote
    // into that same state, a later change that makes a still-active
    // condition audible again would find the SILENCED text already sitting
    // in `conditions`, read the now-audible occurrence as an unchanged
    // repeat, and skip it -- exactly backwards.
    silenced_recent: HashMap<String, f64>,
    silenced_conditions: HashMap<String, String>,
    /// Set by pause(): the next line purges the channel, so anything the
    /// voice was still holding when the player stepped away cannot surface
    /// behind it.
    purge_next: bool,
    /// The newest `Important` or `Critical` line submitted, with the
    /// projection's estimate of when it finishes speaking. An interrupt
    /// arriving before that moment plausibly cut it off mid-sentence; it is
    /// handed back to the caller so the player still hears it.
    protected: Option<Protected>,
    /// Set by should_flush when its purge cut a line still speaking;
    /// collected once by take_flush_cut. See that method.
    flush_cut: Option<Cut>,
    /// text -> until when further rescues of it are refused. See
    /// `take_protected`: one rescue per line per window.
    rescued_until: HashMap<String, f64>,
    /// The line the latest cut destroyed because the player had already
    /// heard most of it; collected once by `take_mostly_heard` so the
    /// caller can name it. See `mostly_heard_fraction`.
    mostly_heard: Option<String>,
    /// When the voice last went from quiet to speaking: the start of the
    /// backlog `clear_at` projects the end of. See `should_flush`.
    busy_since: f64,
    /// Whether every line in that backlog is `Important` or `Critical`.
    backlog_urgent: bool,
}

impl EventSpeechPacer {
    /// A pacer on the monotonic clock.
    pub fn new(config: PacerConfig) -> Self {
        Self::with_clock(config, Box::new(monotonic_seconds))
    }

    pub fn with_clock(config: PacerConfig, clock: Clock) -> Self {
        Self {
            config,
            clock,
            clear_at: 0.0,
            recent: HashMap::new(),
            conditions: HashMap::new(),
            silenced_recent: HashMap::new(),
            silenced_conditions: HashMap::new(),
            purge_next: false,
            protected: None,
            flush_cut: None,
            rescued_until: HashMap::new(),
            mostly_heard: None,
            busy_since: 0.0,
            backlog_urgent: false,
        }
    }

    /// The tuning this pacer was constructed with.
    pub fn config(&self) -> &PacerConfig {
        &self.config
    }

    /// Swap the clock under a live pacer (tests, or a caller re-driving the
    /// same clock the pacer runs on).
    pub fn set_clock(&mut self, clock: Clock) {
        self.clock = clock;
    }

    fn now(&mut self) -> f64 {
        (self.clock)()
    }

    fn duration_s(&self, text: &str) -> f64 {
        self.config.base_utterance_s + text.chars().count() as f64 / self.config.chars_per_s
    }

    /// How long a line of each priority will wait behind a backlog before it
    /// is better to purge the channel and speak now. An `Important` line has
    /// almost no patience: a line the player needs and a piece of chatter
    /// cut off mid-word both cost something, but the chatter costs less. A
    /// `Critical` line never consults this at all -- it is always delivered
    /// interrupting -- so its budget is zero.
    pub fn wait_budget_s(&self, priority: EventPriority) -> f64 {
        match priority {
            EventPriority::Ambient => self.config.ambient_wait_s,
            EventPriority::Important => self.config.important_wait_s,
            EventPriority::Critical => 0.0,
        }
    }

    /// Extend the projection by `text`, noting when a quiet voice starts
    /// and whether everything it holds is an `Important` or `Critical` line.
    fn extend_projection(&mut self, now: f64, text: &str, priority: EventPriority) {
        if now >= self.clear_at {
            self.busy_since = now;
            self.backlog_urgent = true;
        }
        self.backlog_urgent &= priority >= EventPriority::Important;
        self.clear_at = now.max(self.clear_at) + self.duration_s(text);
    }

    /// Start the projection over with `text`: the purge silenced the voice.
    fn restart_projection(&mut self, now: f64, text: &str, priority: EventPriority) {
        self.clear_at = now;
        self.extend_projection(now, text, priority);
    }

    /// Whether everything the voice holds is `Important` or `Critical`,
    /// started inside its pre-utterance pause and still true: this same
    /// instant, with nothing in it heard yet and nothing stale.
    fn backlog_is_this_instant(&self, now: f64) -> bool {
        self.backlog_urgent
            && now - self.busy_since < self.config.base_utterance_s
            && self
                .protected
                .as_ref()
                .is_none_or(|held| held.valid.as_ref().is_none_or(|valid| valid()))
    }

    // -- what the player has already heard ---------------------------------

    /// True when saying this again would tell the player nothing new.
    ///
    /// `key` names a standing condition -- a state of the world rather than
    /// a moment in it. A condition speaks when it starts and again only when
    /// what there is to say about it has changed, so a worsening number is
    /// news and the same number is not.
    ///
    /// `force` is for a line the player asked for: a status key, a
    /// deliberate replay. It is always heard.
    pub fn is_repeat(
        &mut self,
        text: &str,
        key: Option<&str>,
        force: bool,
        window: Option<f64>,
    ) -> bool {
        if force || text.is_empty() {
            return false;
        }
        if let Some(key) = key {
            if self.conditions.get(key).is_some_and(|said| said == text) {
                return true;
            }
        }
        let budget = window.unwrap_or(self.config.repeat_window_s);
        if budget <= 0.0 {
            return false;
        }
        match self.recent.get(text).copied() {
            Some(last) => self.now() - last < budget,
            None => false,
        }
    }

    /// Record a line that reached the voice.
    pub fn note_spoken(&mut self, text: &str, key: Option<&str>) {
        if text.is_empty() {
            return;
        }
        let now = self.now();
        self.recent.insert(text.to_string(), now);
        if let Some(key) = key {
            self.conditions.insert(key.to_string(), text.to_string());
        }
        if self.recent.len() > self.config.recent_limit {
            let memory_s = self.config.recent_memory_s;
            self.recent.retain(|_, said| now - *said < memory_s);
        }
    }

    /// A standing condition has cleared; let it announce itself afresh.
    pub fn forget_condition(&mut self, key: &str) {
        if let Some(text) = self.conditions.remove(key) {
            self.recent.remove(&text);
        }
        if let Some(text) = self.silenced_conditions.remove(key) {
            self.silenced_recent.remove(&text);
        }
    }

    // -- what the caller silenced (earcon-only or fully quiet) --------------

    /// True when this silenced occurrence was already marked (earcon or
    /// not).
    ///
    /// Mirrors [`Self::is_repeat`]'s rules exactly, but reads a namespace
    /// private to occurrences the caller cut, never `conditions`/`recent`. A
    /// silenced repeat must not go unmarked (its earcon would otherwise fire
    /// every frame), but it must equally never be mistaken for a genuinely
    /// spoken occurrence by [`Self::is_repeat`] once the same condition
    /// becomes audible again -- that would silence the very line the player
    /// just gained the right to hear.
    pub fn is_silenced_repeat(
        &mut self,
        text: &str,
        key: Option<&str>,
        window: Option<f64>,
    ) -> bool {
        if text.is_empty() {
            return false;
        }
        if let Some(key) = key {
            if self
                .silenced_conditions
                .get(key)
                .is_some_and(|said| said == text)
            {
                return true;
            }
        }
        let budget = window.unwrap_or(self.config.repeat_window_s);
        if budget <= 0.0 {
            return false;
        }
        match self.silenced_recent.get(text).copied() {
            Some(last) => self.now() - last < budget,
            None => false,
        }
    }

    /// Record a silenced occurrence (earcon played, or fully quiet).
    pub fn note_silenced(&mut self, text: &str, key: Option<&str>) {
        if text.is_empty() {
            return;
        }
        let now = self.now();
        self.silenced_recent.insert(text.to_string(), now);
        if let Some(key) = key {
            self.silenced_conditions
                .insert(key.to_string(), text.to_string());
        }
        if self.silenced_recent.len() > self.config.recent_limit {
            let memory_s = self.config.recent_memory_s;
            self.silenced_recent
                .retain(|_, said| now - *said < memory_s);
        }
    }

    // -- the backlog projection ---------------------------------------------

    /// Remember the newest line worth rescuing if an interrupt lands on it.
    ///
    /// Public because a caller may re-arm the slot with a liveness check
    /// after a plain submission.
    pub fn track(&mut self, text: &str, priority: EventPriority, valid: Option<Valid>) {
        if priority >= EventPriority::Important {
            self.protected = Some(Protected {
                text: text.to_string(),
                priority,
                done_at: self.clear_at,
                valid,
            });
        }
    }

    /// The line a stale flush cut off mid-sentence, once, for requeueing.
    ///
    /// `note_interrupt` returns its cut directly; `should_flush` cannot,
    /// because its return value is the flush verdict. Same contract either
    /// way: an `Important` or `Critical` line still plausibly speaking is
    /// handed back.
    pub fn take_flush_cut(&mut self) -> Option<Cut> {
        self.flush_cut.take()
    }

    /// The line the latest cut dropped as mostly heard, once, so the caller
    /// can name it.
    ///
    /// The drop itself is the pacer's verdict (`mostly_heard_fraction`);
    /// this only lets the caller say which line it was, the way it names a
    /// requeue.
    pub fn take_mostly_heard(&mut self) -> Option<String> {
        self.mostly_heard.take()
    }

    /// Hand over the protected line if it was plausibly cut mid-speech.
    ///
    /// The slot empties either way: a line is given back at most once per
    /// cut, and a line whose projected finish had already passed was heard
    /// in full, not destroyed. A line cutting itself is one line, not two,
    /// so it is never handed back behind its own delivery.
    ///
    /// A line still speaking is handed back only when the cut landed early
    /// in it. Past `mostly_heard_fraction` of its estimated duration the
    /// player has the instruction and the tail is expendable; saying the
    /// whole line again is the stutter this exists to avoid. A line that had
    /// not started speaking at all (queued behind a backlog, its start still
    /// ahead) has a negative fraction and is always handed back -- the
    /// never-dropped contract for a line the player never heard a word of.
    ///
    /// And at most ONE rescue per line per window. A rescued line is
    /// re-queued and re-protected, so without this a chain of urgent lines
    /// would replay it after every one of them. One rescue is the whole
    /// contract (the cut line gets to finish once); a line cut a second time
    /// inside the window is a moment that has already been overtaken by
    /// events.
    fn take_protected(&mut self, cutting_text: Option<&str>) -> Option<Cut> {
        let held = self.protected.take()?;
        let Protected {
            text,
            priority,
            done_at,
            valid,
        } = held;
        if self.now() >= done_at || Some(text.as_str()) == cutting_text {
            return None;
        }
        if let Some(valid) = valid {
            if !valid() {
                // The moment the line described has passed. Replaying the
                // words verbatim would state something no longer true,
                // which is worse than silence.
                return None;
            }
        }
        let now = self.now();
        let duration = self.duration_s(&text);
        let heard = (now - (done_at - duration)) / duration;
        if heard >= self.config.mostly_heard_fraction {
            // Most of it was heard; the rest is not worth replaying.
            self.mostly_heard = Some(text);
            return None;
        }
        if now < self.rescued_until.get(&text).copied().unwrap_or(0.0) {
            return None;
        }
        let until = self.now() + self.config.rescue_once_window_s;
        self.rescued_until.insert(text.clone(), until);
        Some((text, priority))
    }

    /// Whether the only thing ahead of `text` in the voice is the protected
    /// `Important` or `Critical` line, saying its words right now and still
    /// true (its `valid`, when it has one), with nothing queued behind it.
    ///
    /// "Saying its words" is past the pre-utterance pause: a line cut inside
    /// that pause has not been heard at all, so a flush restarting it ahead
    /// of the new line costs the player nothing. A line past it has been
    /// heard in part, and cutting it either replays it from the top or drops
    /// its tail.
    ///
    /// A line superseded by what happened since is not live, so a flush
    /// takes it and it is never handed back.
    fn only_a_live_line_ahead(&self, now: f64, text: &str) -> bool {
        self.protected.as_ref().is_some_and(|held| {
            let started_at = held.done_at - self.duration_s(&held.text);
            now - started_at >= self.config.base_utterance_s
                && now < held.done_at
                && self.clear_at <= held.done_at
                && held.text != text
                && held.valid.as_ref().is_none_or(|valid| valid())
        })
    }

    /// An interrupting line purges the channel: the projection restarts.
    ///
    /// Returns the `Important` or `Critical` line the purge plausibly cut
    /// off mid-sentence -- its projected finish had not yet passed -- so the
    /// caller can queue it right back behind the interrupting line. Chatter,
    /// lines already heard in full or mostly (`take_mostly_heard` names
    /// those), and a line interrupting itself return `None`: nothing worth
    /// giving back was destroyed.
    pub fn note_interrupt(
        &mut self,
        text: &str,
        priority: EventPriority,
        valid: Option<Valid>,
    ) -> Option<Cut> {
        let cut = self.take_protected(Some(text));
        self.purge_next = false;
        let now = self.now();
        self.restart_projection(now, text, priority);
        self.track(text, priority, valid);
        cut
    }

    /// Extend the projection for a line delivered queued, no verdict asked.
    ///
    /// For deliveries that must never flush: a rescued cut-off line (it has
    /// to fall in BEHIND the line that cut it, never purge it) and lines
    /// riding a shared channel, where the backlog belongs to something else.
    pub fn note_queued(&mut self, text: &str, priority: EventPriority, valid: Option<Valid>) {
        let now = self.now();
        self.extend_projection(now, text, priority);
        self.track(text, priority, valid);
    }

    /// A handed-back line spoken AHEAD of the line that flushed it: the
    /// channel is busy for its length too, but the flushing line stays the
    /// newest protected one, so a later flush rescues that line rather than
    /// this one a second time.
    pub fn note_ahead(&mut self, text: &str) {
        self.clear_at += self.duration_s(text);
    }

    /// Speech outside the pacer's view purged the channel these lines ride
    /// on.
    ///
    /// Returns the cut `Important` or `Critical` line exactly as
    /// [`Self::note_interrupt`] would; the projection falls with the purge
    /// (the interrupting speech itself is not the pacer's to time).
    pub fn note_channel_purged(&mut self) -> Option<Cut> {
        let cut = self.take_protected(None);
        self.clear_at = 0.0;
        cut
    }

    /// Whether the projection says the voice is still speaking.
    ///
    /// The channel cannot be asked directly (nothing reports queue state),
    /// so this is the same estimate the staleness budget runs on. Purges,
    /// pauses, and resets zero the projection, so a silenced channel reads
    /// not-busy at once instead of waiting out a stale estimate.
    pub fn busy(&mut self) -> bool {
        self.now() < self.clear_at
    }

    /// Whether a `Critical` line is still being spoken, by the same
    /// projection.
    pub fn speaking_critical(&mut self) -> bool {
        let now = self.now();
        self.protected
            .as_ref()
            .is_some_and(|held| held.priority == EventPriority::Critical && now < held.done_at)
    }

    /// Whether this queued line would start past its priority's budget.
    ///
    /// A pure reading -- the projection is not touched, nothing is tracked.
    /// The caller uses it to decide a line's fate BEFORE committing it to
    /// the channel: chatter that would start stale can be dropped silently,
    /// where [`Self::should_flush`] would instead deliver it interrupting. A
    /// purge armed by `pause()` is not staleness; that path stays with
    /// `should_flush`, whose first line back purges the backlog.
    pub fn would_start_stale(&mut self, _text: &str, priority: EventPriority) -> bool {
        if self.purge_next {
            return false;
        }
        let now = self.now();
        let start = now.max(self.clear_at);
        let budget = self.wait_budget_s(priority);
        start - now > budget
    }

    /// Decide a queued line's fate and update the projection either way.
    ///
    /// Returns true when the line would otherwise start stale -- the caller
    /// must then submit it interrupting (which purges the dead backlog).
    /// `priority` sets how long this line is willing to wait: an important
    /// line gives a backlog of chatter under a second before it goes in
    /// front of it.
    pub fn should_flush(
        &mut self,
        text: &str,
        priority: EventPriority,
        valid: Option<Valid>,
    ) -> bool {
        let now = self.now();
        self.flush_cut = None;
        if self.purge_next {
            // Coming back from a pause. Whatever the voice was still
            // holding when the player stepped away is stale, so the first
            // line back purges it -- and that one really is stale, so it is
            // dropped rather than rescued.
            self.purge_next = false;
            self.restart_projection(now, text, priority);
            self.protected = None;
            self.track(text, priority, valid);
            return true;
        }
        let start = now.max(self.clear_at);
        let budget = self.wait_budget_s(priority);
        if start - now > budget && self.backlog_is_this_instant(now) {
            // Everything the voice holds is important or critical, still
            // true, and began inside its pre-utterance pause: nothing has
            // been heard and nothing is stale; it is this same instant
            // (chatter in it is still flushed, and a line gone untrue is
            // still superseded). A flush here would purge it only to hand
            // the newest of it back ahead of this line -- the same order,
            // the same wait -- and anything older than that newest line
            // would be lost without a word. It queues instead.
            self.extend_projection(now, text, priority);
            self.track(text, priority, valid);
            return false;
        }
        if start - now > budget && self.only_a_live_line_ahead(now, text) {
            // Nothing stale is waiting: the wait is the rest of one
            // important or critical line the player is hearing, still true.
            // A flush here would purge nothing but that line and either hand
            // it back to be said again from the top ahead of this one, or
            // drop its tail. Waiting it out is shorter than the replay, and
            // it is heard once.
            self.extend_projection(now, text, priority);
            self.track(text, priority, valid);
            return false;
        }
        if start - now > budget {
            // A stale flush takes the backlog -- everything in it describes
            // a moment already past -- but NOT a protected line that is
            // still speaking. The incoming line starting stale says nothing
            // about the outgoing one: a critical warning that began this
            // very tick is not stale, and dropping it here would break the
            // never-dropped contract silently, with no requeue.
            //
            // So the rescue asks how much of the outgoing line the player
            // actually got. Past the pre-utterance window it has said
            // something and its tail is expendable; inside it, nothing was
            // heard and the words are still current, so they are handed
            // back to be queued behind the line that cut them -- the same
            // contract a critical cut has always had. `rescue_once_window_s`
            // still caps it at one hand-back per line, and `take_protected`
            // still drops a line the player had mostly heard rather than
            // restart it (`mostly_heard_fraction`).
            //
            // The rescue is narrow on purpose: an AGED backlog of important
            // announcements really does describe a moment already past, and
            // rescuing those would turn one flush into a recital of
            // everything it had just purged. A critical call is the
            // standing exception -- it is the one line whose worth does not
            // decay while it waits.
            let held_rescuable = self.protected.as_ref().is_some_and(|held| {
                held.priority == EventPriority::Critical
                    || now - (held.done_at - self.duration_s(&held.text))
                        < self.config.base_utterance_s
            });
            if held_rescuable {
                self.flush_cut = self.take_protected(Some(text));
            } else {
                self.protected = None;
            }
            self.restart_projection(now, text, priority);
            self.track(text, priority, valid);
            return true;
        }
        self.extend_projection(now, text, priority);
        self.track(text, priority, valid);
        false
    }

    /// The channel was silenced outside the pacer's view.
    pub fn reset(&mut self) {
        self.clear_at = 0.0;
        self.purge_next = false;
        self.protected = None;
    }

    // -- leaving and returning to gameplay -----------------------------------

    /// The player has stepped away from gameplay (a menu, a loading screen).
    ///
    /// The caller silences the event channel; this drops the projection with
    /// it and arms the purge, so the first line spoken back cannot arrive
    /// behind a backlog describing a moment that has passed.
    pub fn pause(&mut self) {
        self.clear_at = 0.0;
        self.purge_next = true;
        self.protected = None;
    }

    /// Back in gameplay. Nothing from before the pause is news.
    pub fn resume(&mut self) {
        self.clear_at = 0.0;
        self.purge_next = true;
        self.protected = None;
    }
}

#[cfg(test)]
mod tests;
