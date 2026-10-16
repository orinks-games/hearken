//! Screens, and the changes to the screen stack they ask for.

use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::InputEvent;

/// One screen. Only `name` and `help` are required.
pub trait State<G: Game> {
    /// A stable identifier for logs and tests.
    fn name(&self) -> &'static str;
    /// Pushed onto the stack.
    fn enter(&mut self, _ctx: &mut Ctx<G>) {}
    /// Popped or replaced.
    fn exit(&mut self, _ctx: &mut Ctx<G>) {}
    /// Revealed again when the screen above it closes. Announces afresh by
    /// default.
    fn resume(&mut self, ctx: &mut Ctx<G>) {
        self.enter(ctx)
    }
    /// Revealed again by a quiet pop (`Ctx::pop_quiet`, `Ctx::pop_to_quiet`):
    /// the player is back here and must hear nothing more. Does nothing by
    /// default; a screen whose rows may have changed underneath it rebuilds
    /// them here without speaking, as menus do.
    fn resume_quiet(&mut self, _ctx: &mut Ctx<G>) {}
    /// Revealed again by a pop that carries a result line (`Ctx::pop_with`,
    /// `Ctx::pop_to_with`), such as "Via Ames added.": the result is spoken
    /// first, then where the player is. By default the result interrupts
    /// and `resume` follows it queued, so its announcement cannot cut the
    /// result off. Menus speak the result, then only the current row.
    fn resume_with(&mut self, ctx: &mut Ctx<G>, result: &str) {
        ctx.say(crate::text::end_sentence(result));
        ctx.queued_speech(|ctx| self.resume(ctx));
    }
    fn handle_input(&mut self, _ctx: &mut Ctx<G>, _event: &InputEvent) {}
    /// An event from the game's rules, after `Game::observe`.
    fn on_event(&mut self, _ctx: &mut Ctx<G>, _event: &G::Event) {}
    /// Once per frame, top screen only.
    fn update(&mut self, _ctx: &mut Ctx<G>, _dt: f64) {}
    /// The screen as text, for a sighted helper and for tests (the SDL
    /// window does not draw it). The player gets speech.
    fn lines(&self, _ctx: &Ctx<G>) -> Vec<String> {
        Vec::new()
    }
    /// Spoken on F1.
    fn help(&self, ctx: &Ctx<G>) -> String;
    /// F1 (or a controller's Back) was pressed: speaks [`State::help`] by
    /// default. A screen whose help depends on what it is doing, or that
    /// must remember the press (a second press reading more), takes it here.
    fn help_pressed(&mut self, ctx: &mut Ctx<G>) {
        let help = self.help(ctx);
        ctx.say(help);
    }
}

/// A change to the stack, queued on the context and applied once the
/// current handler has returned, so a screen never changes the stack it is
/// running on.
pub enum StackOp<G: Game> {
    Push(Box<dyn State<G>>),
    /// Close the top screen; the one revealed announces itself (`resume`).
    Pop,
    /// Close the top screen; the one revealed hears `Reveal`.
    PopReveal(Reveal),
    /// Close screens until the top one's `State::name` is the name given;
    /// the one revealed hears `Reveal`. Nothing is closed when no screen on
    /// the stack has that name.
    PopTo(String, Reveal),
    Replace(Box<dyn State<G>>),
    Reset(Box<dyn State<G>>),
}

/// What the screen revealed by a pop hears.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reveal {
    /// Its full announcement, through `State::resume`.
    Announce,
    /// Nothing, through `State::resume_quiet`.
    Quiet,
    /// This result line, then its context, through `State::resume_with`.
    /// A blank result is `Announce`.
    With(String),
}
