//! A second spoken channel for a game's own voices: a broadcast crew, a
//! narrator. The engine holds one [`Commentary`] on the context, updates it
//! every frame, stops it on F2 beside speech, and keeps the mix ducked while
//! it talks. With none installed, a line goes to the event voice through
//! [`Ctx::say_event_with`], so a game can always call [`Ctx::commentate`].
//!
//! `hearken-voice` provides the AI voice (`Voice`) and a test double
//! (`FakeVoice`); the engine itself depends on neither.

use hearken_speech::EventPriority;

use crate::ctx::Ctx;
use crate::game::Game;
use crate::speech_delivery::SayEvent;

/// One line for a game's voices.
#[derive(Clone, Debug, PartialEq)]
pub struct Line<C> {
    /// What is said.
    pub text: String,
    /// Who says it: a role the game names (`"play_by_play"`), which the
    /// commentary maps to a voice.
    pub speaker: String,
    /// How long the line may wait. `Critical` interrupts.
    pub priority: EventPriority,
    /// A standing condition's key: said again only when its text changes.
    pub key: Option<String>,
    /// The game's category, which the player's verbosity level has a rule for.
    pub category: Option<C>,
    /// What a `Terse` rule says instead.
    pub terse: Option<String>,
}

impl<C> Line<C> {
    /// An `Ambient` line with no category.
    pub fn new(speaker: impl Into<String>, text: impl Into<String>) -> Self {
        Line {
            text: text.into(),
            speaker: speaker.into(),
            priority: EventPriority::Ambient,
            key: None,
            category: None,
            terse: None,
        }
    }

    pub fn priority(mut self, priority: EventPriority) -> Self {
        self.priority = priority;
        self
    }

    pub fn key(mut self, key: &str) -> Self {
        self.key = Some(key.to_string());
        self
    }

    pub fn category(mut self, category: C) -> Self {
        self.category = Some(category);
        self
    }

    pub fn terse(mut self, text: &str) -> Self {
        self.terse = Some(text.to_string());
        self
    }
}

/// A game's own voices, held by the context (`Ctx::set_commentary`).
pub trait Commentary<G: Game> {
    /// Queue `line`. Returns at once; the line plays when it is ready.
    fn speak(&mut self, ctx: &mut Ctx<G>, line: Line<G::Category>);
    /// Get `lines` ready ahead of time, such as a screen's fixed lines on
    /// entering it. Nothing is said.
    fn prime(&mut self, _ctx: &mut Ctx<G>, _lines: &[Line<G::Category>]) {}
    /// Once per frame, after the top screen's update.
    fn update(&mut self, _ctx: &mut Ctx<G>, _dt: f64) {}
    /// Silence the voice and drop what is queued (F2, a pause).
    fn stop(&mut self, _ctx: &mut Ctx<G>) {}
    /// Whether a line is playing now.
    fn is_speaking(&self) -> bool {
        false
    }
    /// The voices a speaker can be set to, for Settings.
    fn speakers(&self) -> Vec<String> {
        Vec::new()
    }
    /// A sentence for Settings on why the voices are, or are not, in use.
    fn status(&self) -> Option<String> {
        None
    }
    /// Look for the voices again, as after installing them.
    fn reload(&mut self, _ctx: &mut Ctx<G>) {}
}

impl<G: Game> Ctx<G> {
    /// Installs `commentary` in place of any before it, which is stopped.
    pub fn set_commentary(&mut self, commentary: Box<dyn Commentary<G>>) {
        if let Some(mut old) = self.commentary.take() {
            old.stop(self);
        }
        self.commentary = Some(commentary);
    }

    /// Removes the commentary; lines go to the event voice again.
    pub fn take_commentary(&mut self) -> Option<Box<dyn Commentary<G>>> {
        self.commentary.take()
    }

    /// Whether a commentary is installed.
    pub fn has_commentary(&self) -> bool {
        self.commentary.is_some()
    }

    /// Say `line` through the commentary, or through the event voice when
    /// there is none.
    pub fn commentate(&mut self, line: Line<G::Category>) {
        match self.commentary.take() {
            Some(mut c) => {
                c.speak(self, line);
                self.restore_commentary(c);
            }
            None => self.say_line(line),
        }
    }

    /// Get `lines` ready ahead of time; see [`Commentary::prime`].
    pub fn prime_commentary(&mut self, lines: &[Line<G::Category>]) {
        if let Some(mut c) = self.commentary.take() {
            c.prime(self, lines);
            self.restore_commentary(c);
        }
    }

    /// Silence the commentary and drop its queue.
    pub fn stop_commentary(&mut self) {
        if let Some(mut c) = self.commentary.take() {
            c.stop(self);
            self.restore_commentary(c);
        }
    }

    /// Ask the commentary to look for its voices again.
    pub fn reload_commentary(&mut self) {
        if let Some(mut c) = self.commentary.take() {
            c.reload(self);
            self.restore_commentary(c);
        }
    }

    pub fn commentary_speaking(&self) -> bool {
        self.commentary.as_ref().is_some_and(|c| c.is_speaking())
    }

    pub fn commentary_speakers(&self) -> Vec<String> {
        self.commentary
            .as_ref()
            .map(|c| c.speakers())
            .unwrap_or_default()
    }

    pub fn commentary_status(&self) -> Option<String> {
        self.commentary.as_ref().and_then(|c| c.status())
    }

    /// Say `line` on the event voice, through the verbosity rules and the
    /// pacer, as the commentary's fallback. A `Critical` line interrupts.
    pub fn say_line(&mut self, line: Line<G::Category>) {
        let mut opts = SayEvent::new()
            .priority(line.priority)
            .interrupt(line.priority == EventPriority::Critical);
        if let Some(c) = line.category {
            opts = opts.category(c);
        }
        if let Some(key) = &line.key {
            opts = opts.key(key);
        }
        if let Some(terse) = &line.terse {
            opts = opts.terse(terse);
        }
        self.say_event_with(line.text, opts);
    }

    pub(crate) fn update_commentary(&mut self, dt: f64) {
        if let Some(mut c) = self.commentary.take() {
            c.update(self, dt);
            self.restore_commentary(c);
        }
    }

    /// Puts `c` back unless a call inside it installed another.
    fn restore_commentary(&mut self, c: Box<dyn Commentary<G>>) {
        if self.commentary.is_none() {
            self.commentary = Some(c);
        }
    }
}
