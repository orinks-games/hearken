//! A brief spoken transition that counts down and moves on by itself.
//! Enter, Space, Escape and F1 are answered with where it is instead of
//! acted on, and every other key does nothing, so a key still held from the
//! screen before cannot act on the next one.
//!
//! Extracted from Freight Fate (commit c5d3ef8d), `TimedMessageState`.

use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::{InputEvent, Key};
use crate::sound::UiSound;
use crate::state::State;

/// What a [`TimedMessage`] does when its countdown ends.
pub type OnComplete<G> = Box<dyn FnOnce(&mut Ctx<G>)>;

pub struct TimedMessage<G: Game> {
    title: String,
    message: String,
    status: String,
    remaining: f64,
    complete_text: String,
    on_complete: Option<OnComplete<G>>,
    complete: bool,
}

impl<G: Game> TimedMessage<G> {
    /// `message` is spoken on arrival and `status` answers a key press or
    /// F1. After `seconds` on top of the stack, `complete_text` (if any) is
    /// queued and `on_complete` runs, usually replacing this screen with
    /// the next.
    pub fn new(
        title: &str,
        message: &str,
        status: &str,
        seconds: f64,
        on_complete: impl FnOnce(&mut Ctx<G>) + 'static,
    ) -> Self {
        TimedMessage {
            title: title.to_string(),
            message: message.to_string(),
            status: status.to_string(),
            remaining: seconds.max(0.0),
            complete_text: String::new(),
            on_complete: Some(Box::new(on_complete)),
            complete: false,
        }
    }

    /// Spoken, queued, when the countdown ends.
    pub fn complete_text(mut self, text: &str) -> Self {
        self.complete_text = text.to_string();
        self
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }

    pub fn remaining(&self) -> f64 {
        self.remaining
    }
}

impl<G: Game> State<G> for TimedMessage<G> {
    fn name(&self) -> &'static str {
        "timed_message"
    }

    fn enter(&mut self, ctx: &mut Ctx<G>) {
        ctx.play_ui(UiSound::Notify);
        ctx.say(self.message.clone());
    }

    fn handle_input(&mut self, ctx: &mut Ctx<G>, event: &InputEvent) {
        // A held key's repeats would say the same status over and over.
        if event.is_repeat() {
            return;
        }
        if matches!(
            event.pressed(),
            Some(Key::Escape | Key::Return | Key::Space | Key::KpEnter)
        ) {
            ctx.say(self.status.clone());
        }
    }

    fn update(&mut self, ctx: &mut Ctx<G>, dt: f64) {
        if self.complete {
            return;
        }
        self.remaining = (self.remaining - dt).max(0.0);
        if self.remaining > 0.0 {
            return;
        }
        self.complete = true;
        if !self.complete_text.is_empty() {
            ctx.say_queued(self.complete_text.clone());
        }
        if let Some(on_complete) = self.on_complete.take() {
            on_complete(ctx);
        }
    }

    fn lines(&self, _ctx: &Ctx<G>) -> Vec<String> {
        vec![
            self.title.clone(),
            String::new(),
            self.status.clone(),
            format!("Ready in {:.1} seconds.", self.remaining),
        ]
    }

    fn help(&self, _ctx: &Ctx<G>) -> String {
        self.status.clone()
    }
}
