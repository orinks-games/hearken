//! Gameplay speech: the game's verbosity rules, the event-speech pacer, and
//! the duck that steps the mix back while the event voice talks.
//!
//! Ported from Freight Fate's `speech_delivery.rs` (commit `c5d3ef8d`):
//! `say_event_with`, the cut-line hand-back, the speech and earcon ducks, and
//! pausing event speech. Left out: the message review log, delivery
//! receipts, the "Transitions" verbosity rule, and `SpokenMessage` rendering
//! inside delivery (a game passes the text, and a terse one, itself).

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

use hearken_audio::Bus;
use hearken_speech::{
    Category, Cut, EventPriority, EventSpeechPacer, Level, PacerConfig, Rule, Valid, Verbosity,
};

use crate::ctx::{Ctx, TRANSCRIPT_TARGET};
use crate::game::Game;

/// Added to `ctx.clock` for the pacer's clock, so game time never reads as
/// the `0.0` the pacer uses to mean "idle".
const PACER_CLOCK_OFFSET: f64 = 1000.0;

/// How one gameplay line is delivered: see [`Ctx::say_event_with`].
pub struct SayEvent<C> {
    interrupt: bool,
    priority: Option<EventPriority>,
    category: Option<C>,
    key: Option<String>,
    terse: Option<String>,
    force: bool,
    valid: Option<Valid>,
}

impl<C> Default for SayEvent<C> {
    fn default() -> Self {
        SayEvent {
            interrupt: false,
            priority: None,
            category: None,
            key: None,
            terse: None,
            force: false,
            valid: None,
        }
    }
}

impl<C> SayEvent<C> {
    /// A queued line with no category.
    pub fn new() -> Self {
        Self::default()
    }

    /// Cut off what the event voice is saying.
    pub fn interrupt(mut self, on: bool) -> Self {
        self.interrupt = on;
        self
    }

    /// How long the line will wait behind others. Defaults to `Critical`
    /// for an interrupting line, else `Ambient`.
    pub fn priority(mut self, priority: EventPriority) -> Self {
        self.priority = Some(priority);
        self
    }

    /// The game's category for the line, which the player's verbosity level
    /// has a rule for.
    pub fn category(mut self, category: C) -> Self {
        self.category = Some(category);
        self
    }

    /// A standing condition's key: the line speaks again only when its text
    /// changes, or after `Ctx::clear_condition`.
    pub fn key(mut self, key: &str) -> Self {
        self.key = Some(key.to_string());
        self
    }

    /// What a `Terse` rule says instead of the full line.
    pub fn terse(mut self, text: &str) -> Self {
        self.terse = Some(text.to_string());
        self
    }

    /// A line the player asked for: it skips the verbosity rules and the
    /// repeat checks.
    pub fn force(mut self) -> Self {
        self.force = true;
        self
    }

    /// Whether the moment the line describes is still true, asked before a
    /// cut-off line is said again.
    pub fn valid(mut self, still_true: impl Fn() -> bool + 'static) -> Self {
        self.valid = Some(Box::new(still_true));
        self
    }
}

/// The context's gameplay-speech state.
pub(crate) struct Delivery<C: 'static> {
    pub(crate) pacer: Option<EventSpeechPacer>,
    pub(crate) verbosity: Verbosity<C>,
    /// `(key, text)` of every first-time line already said.
    pub(crate) first_times: HashSet<(String, String)>,
    pub(crate) ducked: bool,
    /// `ctx.clock` seconds the duck holds until at least; `0.0` when unset.
    pub(crate) duck_until: f64,
    /// What the pacer reads as the time.
    pub(crate) clock: Rc<Cell<f64>>,
}

impl<C: Category> Delivery<C> {
    pub(crate) fn new(pacer: Option<PacerConfig>, verbosity: Verbosity<C>) -> Self {
        for problem in verbosity.problems() {
            log::warn!("verbosity table: {problem}");
        }
        let clock = Rc::new(Cell::new(PACER_CLOCK_OFFSET));
        let pacer = pacer.map(|config| {
            let clock = clock.clone();
            EventSpeechPacer::with_clock(config, Box::new(move || clock.get()))
        });
        Delivery {
            pacer,
            verbosity,
            first_times: HashSet::new(),
            ducked: false,
            duck_until: 0.0,
            clock,
        }
    }
}

impl<G: Game> Ctx<G> {
    /// Speak on the gameplay-event channel, which a separate voice can carry
    /// so the screen reader never talks over it.
    pub fn say_event(&mut self, text: impl Into<String>, interrupt: bool) {
        self.say_event_with(text, SayEvent::new().interrupt(interrupt));
    }

    /// Speak a gameplay line through the player's verbosity level and the
    /// pacer.
    ///
    /// The line's category picks a rule at the saved level: it may be said
    /// whole, said tersely, said only the first time, replaced by an earcon,
    /// or not said at all. A line the player has just heard, or a standing
    /// condition that has not changed, is not said again. Chatter that would
    /// only start once its moment has passed is dropped; a queued line
    /// stuck behind a stale backlog purges it and speaks now. An interrupt
    /// that cuts off an important line mid-sentence says that line again
    /// right after it.
    pub fn say_event_with(&mut self, text: impl Into<String>, opts: SayEvent<G::Category>) {
        let text = text.into();
        if text.trim().is_empty() {
            return;
        }
        let SayEvent {
            mut interrupt,
            priority,
            category,
            key,
            terse,
            force,
            valid,
        } = opts;
        let key = key.as_deref();
        // A first-time line counts as said only once it reaches the voice,
        // so one the pacer drops can still be said the next time.
        let mut first_time = None;
        let rule = match category {
            Some(c) if !force => self
                .delivery
                .verbosity
                .rule(&self.settings.speech.verbosity, c),
            _ => Rule::Full,
        };
        let text = match rule {
            Rule::Silent | Rule::Earcon(_) => {
                if let Some(pacer) = self.delivery.pacer.as_mut() {
                    if pacer.is_silenced_repeat(&text, key, None) {
                        return;
                    }
                    pacer.note_silenced(&text, key);
                }
                if let Rule::Earcon(sound) = rule {
                    self.audio.play(sound, Bus::UI);
                    self.engage_duck(None);
                }
                log::info!(target: TRANSCRIPT_TARGET, "[verbosity] not spoken: {text}");
                return;
            }
            Rule::FirstTime => {
                let seen = (key.unwrap_or("").to_string(), text.clone());
                if self.delivery.first_times.contains(&seen) {
                    return;
                }
                first_time = Some(seen);
                text
            }
            Rule::Terse => terse.unwrap_or(text),
            Rule::Full => text,
        };
        // A terse line of nothing says nothing.
        if text.trim().is_empty() {
            return;
        }
        let priority = priority.unwrap_or(if interrupt {
            EventPriority::Critical
        } else {
            EventPriority::Ambient
        });
        let separate = self.speech.has_separate_event_voice();
        // A line a flush cut off, said ahead of this one; and one an
        // interrupt cut off, said behind it.
        let mut ahead = None;
        let mut cut = None;
        if let Some(pacer) = self.delivery.pacer.as_mut() {
            if pacer.is_repeat(&text, key, force, None) {
                // Already in the player's ear: not spoken, not recorded.
                return;
            }
            if !interrupt
                && priority == EventPriority::Ambient
                && separate
                && pacer.would_start_stale(&text, priority)
            {
                // Chatter that would start after its moment is dropped, not
                // promoted to an interrupt, and not marked heard: an
                // identical later moment speaks fresh.
                log::info!(target: TRANSCRIPT_TARGET, "[pacer] stale ambient dropped: {text}");
                return;
            }
            pacer.note_spoken(&text, key);
            if interrupt {
                cut = pacer.note_interrupt(&text, priority, valid);
            } else if !separate {
                pacer.note_queued(&text, priority, valid);
            } else if pacer.should_flush(&text, priority, valid) {
                // The backlog has gone stale: purging it and speaking now is
                // this line's honest delivery.
                log::info!(target: TRANSCRIPT_TARGET, "[pacer] stale event backlog flushed");
                ahead = pacer.take_flush_cut();
                interrupt = true;
            }
        }
        if let Some(seen) = first_time {
            self.delivery.first_times.insert(seen);
        }
        if self.delivery.pacer.is_none() {
            self.speech.say_event(&text, interrupt);
        } else if separate {
            // What a flush hands back came before this line and is still
            // true, so it is said first, purging the backlog, and this line
            // queues behind it.
            if let Some(first) = ahead {
                self.deliver_cut(first, true);
                interrupt = false;
            }
            self.speech.say_event(&text, interrupt);
        } else {
            // Events share the main voice: an interrupt stops it, and the
            // line queues on it.
            if interrupt {
                self.speech.stop_main();
            }
            self.speech.say(&text, false);
        }
        self.record(format!("[event] {text}"));
        self.engage_duck(Some(&text));
        if let Some(heard) = self
            .delivery
            .pacer
            .as_mut()
            .and_then(|pacer| pacer.take_mostly_heard())
        {
            log::info!(target: TRANSCRIPT_TARGET, "[pacer] cut line mostly heard, dropped: {heard}");
        }
        if let Some(cut) = cut {
            self.deliver_cut(cut, false);
        }
    }

    /// Say a line the pacer handed back: queued behind the interrupt that
    /// cut it, or, for a flush, interrupting ahead of the line that flushed.
    fn deliver_cut(&mut self, (text, priority): Cut, interrupt: bool) {
        log::info!(target: TRANSCRIPT_TARGET, "[pacer] cut line requeued: {text}");
        if let Some(pacer) = self.delivery.pacer.as_mut() {
            if interrupt {
                pacer.note_ahead(&text);
            } else {
                pacer.note_queued(&text, priority, None);
            }
        }
        if self.speech.has_separate_event_voice() {
            self.speech.say_event(&text, interrupt);
        } else {
            self.speech.say(&text, false);
        }
        self.record(format!("[event] {text}"));
    }

    /// A standing condition has cleared: its line may announce itself again.
    pub fn clear_condition(&mut self, key: &str) {
        if let Some(pacer) = self.delivery.pacer.as_mut() {
            pacer.forget_condition(key);
        }
    }

    /// Every first-time line may be said once more, as for a new player.
    pub fn reset_first_times(&mut self) {
        self.delivery.first_times.clear();
    }

    /// The player stepped out of gameplay (a menu, a pause): silence the
    /// event voice and drop its backlog.
    pub fn pause_event_speech(&mut self) {
        if let Some(pacer) = self.delivery.pacer.as_mut() {
            pacer.pause();
        }
        self.speech.stop_event();
    }

    /// Back in gameplay; nothing from before the pause is news.
    pub fn resume_event_speech(&mut self) {
        if let Some(pacer) = self.delivery.pacer.as_mut() {
            pacer.resume();
        }
        self.speech.stop_event();
    }

    /// The game's verbosity table.
    pub fn verbosity(&self) -> Verbosity<G::Category> {
        self.delivery.verbosity
    }

    /// The player's verbosity level: the saved one, or the game's first when
    /// the saved one is gone. `None` for a game with no levels.
    pub fn verbosity_level(&self) -> Option<&'static Level<G::Category>> {
        self.delivery
            .verbosity
            .level(&self.settings.speech.verbosity)
    }

    /// Whether the mix is stepped back under speech right now.
    pub fn speech_ducked(&self) -> bool {
        self.delivery.ducked
    }

    /// Step the ducked buses back for a spoken line, or for an earcon
    /// (`None`) standing in for one. A pacer's projection says when a line
    /// ends; without one, and for an earcon, the duck holds for its own
    /// estimate.
    fn engage_duck(&mut self, line: Option<&str>) {
        if !self.settings.audio.duck_for_speech {
            return;
        }
        let sound = self.sound_config();
        let hold = match line {
            None => Some(sound.earcon_duck_s),
            Some(text) if self.delivery.pacer.is_none() => {
                let pace = PacerConfig::default();
                Some(pace.base_utterance_s + text.chars().count() as f64 / pace.chars_per_s)
            }
            Some(_) => None,
        };
        let level = sound.duck_level;
        if let Some(hold) = hold {
            self.delivery.duck_until = self.delivery.duck_until.max(self.clock + hold);
        }
        self.delivery.ducked = true;
        self.audio.set_duck(level);
    }

    /// Per frame: lift the duck once its hold has passed and the pacer says
    /// the event voice is done.
    pub(crate) fn update_duck(&mut self) {
        if !self.delivery.ducked || self.clock < self.delivery.duck_until {
            return;
        }
        if self
            .delivery
            .pacer
            .as_mut()
            .is_some_and(|pacer| pacer.busy())
        {
            return;
        }
        self.delivery.ducked = false;
        self.delivery.duck_until = 0.0;
        self.audio.set_duck(1.0);
    }

    /// Per frame, once `clock` has advanced: the pacer sees the same time.
    pub(crate) fn sync_pacer_clock(&mut self) {
        self.delivery.clock.set(self.clock + PACER_CLOCK_OFFSET);
    }
}
