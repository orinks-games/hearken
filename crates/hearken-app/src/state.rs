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
}

/// A change to the stack, queued on the context and applied once the
/// current handler has returned, so a screen never changes the stack it is
/// running on.
pub enum StackOp<G: Game> {
    Push(Box<dyn State<G>>),
    Pop,
    Replace(Box<dyn State<G>>),
    Reset(Box<dyn State<G>>),
}
