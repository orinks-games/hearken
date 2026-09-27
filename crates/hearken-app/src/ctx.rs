//! What every screen is handed.

use hearken_speech::SpeechSink;

use crate::game::Game;
use crate::held_keys::HeldKeys;
use crate::paths::Paths;
use crate::settings::Settings;
use crate::state::{StackOp, State};

/// The `log` target every spoken line is written under, so a session log
/// reads as a transcript of the run.
pub const TRANSCRIPT_TARGET: &str = "hearken.transcript";

/// How many spoken lines `Ctx::transcript` keeps.
pub const TRANSCRIPT_CAP: usize = 2000;

pub struct Ctx<G: Game> {
    pub speech: Box<dyn SpeechSink>,
    pub settings: Settings<G::Settings>,
    pub paths: Paths,
    pub game: G::Data,
    /// Which keys are down now, for a screen that polls instead of reacting
    /// to presses.
    pub held: HeldKeys,
    pub running: bool,
    /// Every line spoken this run, newest last. Event lines start `[event] `.
    pub transcript: Vec<String>,
    /// Lines for the player once the first screen has announced itself.
    pub notices: Vec<String>,
    pub frame: u64,
    /// Seconds since the run started, the sum of every frame's `dt`.
    pub clock: f64,
    pending: Vec<StackOp<G>>,
}

impl<G: Game> Ctx<G> {
    pub fn new(
        speech: Box<dyn SpeechSink>,
        settings: Settings<G::Settings>,
        paths: Paths,
        game: G::Data,
    ) -> Self {
        Ctx {
            speech,
            settings,
            paths,
            game,
            held: HeldKeys::default(),
            running: true,
            transcript: Vec::new(),
            notices: Vec::new(),
            frame: 0,
            clock: 0.0,
            pending: Vec::new(),
        }
    }

    /// Speak on the main channel now, cutting off what it was saying.
    pub fn say(&mut self, text: impl Into<String>) {
        self.speak(text.into(), true);
    }

    /// Speak on the main channel after what it is saying.
    pub fn say_queued(&mut self, text: impl Into<String>) {
        self.speak(text.into(), false);
    }

    /// Speak on the gameplay-event channel, which a separate voice can carry
    /// so the screen reader never talks over it.
    pub fn say_event(&mut self, text: impl Into<String>, interrupt: bool) {
        let text = text.into();
        if text.trim().is_empty() {
            return;
        }
        self.speech.say_event(&text, interrupt);
        self.record(format!("[event] {text}"));
    }

    /// Read a settings change back through the voice it changed, so a new
    /// speech rate is heard at that rate; on the main channel when no voice
    /// can be adjusted that way. `setting` is a key such as `"speech_rate"`.
    pub fn say_preview(&mut self, setting: &str, text: impl Into<String>) {
        let text = text.into();
        if text.trim().is_empty() {
            return;
        }
        if !self.speech.say_adjustment_preview(setting, &text, true) {
            self.speech.say(&text, true);
        }
        self.record(text);
    }

    fn speak(&mut self, text: String, interrupt: bool) {
        if text.trim().is_empty() {
            return;
        }
        self.speech.say(&text, interrupt);
        self.record(text);
    }

    fn record(&mut self, line: String) {
        log::info!(target: TRANSCRIPT_TARGET, "{line}");
        self.transcript.push(line);
        if self.transcript.len() > TRANSCRIPT_CAP {
            let excess = self.transcript.len() - TRANSCRIPT_CAP;
            self.transcript.drain(..excess);
        }
    }

    pub fn stop_speech(&mut self) {
        self.speech.stop();
    }

    /// Push the saved speech settings to the voices.
    pub fn apply_speech_settings(&mut self) {
        hearken_speech::apply_speech_settings(self.speech.as_mut(), &mut self.settings.speech);
    }

    pub fn push(&mut self, screen: impl State<G> + 'static) {
        self.pending.push(StackOp::Push(Box::new(screen)));
    }

    pub fn pop(&mut self) {
        self.pending.push(StackOp::Pop);
    }

    pub fn replace(&mut self, screen: impl State<G> + 'static) {
        self.pending.push(StackOp::Replace(Box::new(screen)));
    }

    pub fn reset_to(&mut self, screen: impl State<G> + 'static) {
        self.pending.push(StackOp::Reset(Box::new(screen)));
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    pub(crate) fn take_pending(&mut self) -> Vec<StackOp<G>> {
        std::mem::take(&mut self.pending)
    }

    pub(crate) fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub(crate) fn clear_pending(&mut self) {
        self.pending.clear();
    }
}
