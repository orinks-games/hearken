//! What every screen is handed.

use hearken_speech::SpeechSink;

use crate::bindings::KeyBindings;
use crate::controller::{ControllerManager, InputDevice};
use crate::game::Game;
use crate::held_keys::HeldKeys;
use crate::input::InputEvent;
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
    /// The bound game controller. Headless contexts get one that never
    /// reaches a real subsystem; `run` swaps in SDL's.
    pub controller: ControllerManager,
    /// The game's controls, with the player's changes. Change them through
    /// `KeyBindings::set_chord` and friends, then `save_bindings`.
    pub bindings: KeyBindings<G::Action>,
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
        let controller = ControllerManager::detached(settings.controller.enabled);
        let bindings = KeyBindings::from_saved(&settings.bindings);
        Ctx {
            speech,
            settings,
            paths,
            game,
            held: HeldKeys::default(),
            controller,
            bindings,
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

    /// Write `settings.json`. A failure is logged: the game plays on with
    /// the settings it has.
    pub fn save_settings(&self) {
        if let Err(e) = self.settings.save(&self.paths) {
            log::error!("could not save settings: {e}");
        }
    }

    /// Keep the player's bindings in the settings and save them.
    pub fn save_bindings(&mut self) {
        self.bindings.store(&mut self.settings.bindings);
        self.save_settings();
    }

    /// The game action a key press or pad button means, by the player's
    /// bindings. `None` for anything else, and for a press no action has.
    pub fn action_for(&self, event: &InputEvent) -> Option<G::Action> {
        match event {
            InputEvent::KeyDown { key, mods, .. } => self.bindings.action_for(*key, *mods),
            InputEvent::ControllerButtonDown { button, .. } => self
                .bindings
                .pad_action_for(*button, self.controller.modifier()),
            _ => None,
        }
    }

    /// Whether `action`'s key or pad button is held down right now.
    pub fn action_held(&self, action: G::Action) -> bool {
        self.bindings.pressed(&self.held, action)
            || self.bindings.pad_chords(action).iter().any(|chord| {
                self.controller.is_down(chord.button)
                    && (!chord.modified || self.controller.modifier())
            })
    }

    /// `action`'s control as a prompt names it: the pad button while the
    /// player is on a controller and the action has one, else the key.
    pub fn control_name(&self, action: G::Action) -> String {
        if self.controller.device() == InputDevice::Controller
            && !self.bindings.pad_chords(action).is_empty()
        {
            self.bindings.pad_spoken(action)
        } else {
            self.bindings.spoken(action)
        }
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
