//! The screen stack and the frame loop that drives it.

use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::{InputEvent, Key};
use crate::shell::Shell;
use crate::state::{StackOp, State};

/// Rounds of stack changes applied at once. An `enter` may queue more, and a
/// screen that always does would otherwise hang the game.
const MAX_STACK_ROUNDS: usize = 16;

pub struct App<G: Game> {
    pub ctx: Ctx<G>,
    states: Vec<Box<dyn State<G>>>,
}

impl<G: Game> App<G> {
    /// Enter the first screen, then speak any notices behind its
    /// announcement.
    pub fn new(mut ctx: Ctx<G>, mut first: Box<dyn State<G>>) -> Self {
        first.enter(&mut ctx);
        for notice in std::mem::take(&mut ctx.notices) {
            ctx.say_queued(notice);
        }
        let mut app = App {
            ctx,
            states: vec![first],
        };
        app.apply_pending();
        app
    }

    /// One frame: input to the top screen, the game's events, the top
    /// screen's update, speech upkeep, and the stack changes asked for.
    pub fn step(&mut self, events: Vec<InputEvent>, dt: f64) {
        for event in events {
            match &event {
                InputEvent::Quit => {
                    self.ctx.running = false;
                    continue;
                }
                // Coming back to the window re-checks which screen reader
                // is running; the screen still sees the event.
                InputEvent::WindowFocusGained => self.ctx.speech.request_refresh(),
                // F2 silences speech everywhere, so no screen can hold it.
                InputEvent::KeyDown { key: Key::F2, .. } => {
                    self.ctx.stop_speech();
                    continue;
                }
                // F1 reads the top screen's help.
                InputEvent::KeyDown { key: Key::F1, .. } => {
                    let help = self.states.last().map(|top| top.help(&self.ctx));
                    if let Some(help) = help {
                        self.ctx.say(help);
                    }
                    continue;
                }
                _ => {}
            }
            if let Some(top) = self.states.last_mut() {
                top.handle_input(&mut self.ctx, &event);
            }
        }
        // A screen that pushed or popped from a key must not also get this
        // frame's update.
        self.apply_pending();

        for event in G::pump(&mut self.ctx, dt) {
            G::observe(&mut self.ctx, &event);
            if let Some(top) = self.states.last_mut() {
                top.on_event(&mut self.ctx, &event);
            }
        }
        if let Some(top) = self.states.last_mut() {
            top.update(&mut self.ctx, dt);
        }
        self.ctx.speech.poll(dt);

        self.apply_pending();
        self.ctx.frame += 1;
        self.ctx.clock += dt;
    }

    /// Run until something quits, then let the game finish, save the
    /// settings and shut speech down.
    pub fn run(&mut self, shell: &mut dyn Shell) {
        while self.ctx.running {
            let events = shell.poll();
            let dt = shell.frame();
            self.step(events, dt);
            let lines = self.top_lines();
            shell.render(&lines);
        }
        G::on_exit(&mut self.ctx);
        if let Err(e) = self.ctx.settings.save(&self.ctx.paths) {
            log::error!("could not save settings: {e}");
        }
        self.ctx.speech.shutdown();
    }

    pub fn top_name(&self) -> &'static str {
        self.states.last().map(|s| s.name()).unwrap_or("none")
    }

    pub fn depth(&self) -> usize {
        self.states.len()
    }

    pub fn top_lines(&self) -> Vec<String> {
        self.states
            .last()
            .map(|s| s.lines(&self.ctx))
            .unwrap_or_default()
    }

    fn apply_pending(&mut self) {
        for _ in 0..MAX_STACK_ROUNDS {
            if !self.ctx.has_pending() {
                break;
            }
            for op in self.ctx.take_pending() {
                match op {
                    StackOp::Push(mut next) => {
                        next.enter(&mut self.ctx);
                        self.states.push(next);
                    }
                    StackOp::Pop => {
                        if let Some(mut gone) = self.states.pop() {
                            gone.exit(&mut self.ctx);
                        }
                        // An empty stack is only judged once the whole queue
                        // has drained: popping the root and pushing its
                        // replacement in one handler is allowed.
                        if let Some(top) = self.states.last_mut() {
                            top.resume(&mut self.ctx);
                        }
                    }
                    StackOp::Replace(mut next) => {
                        if let Some(mut gone) = self.states.pop() {
                            gone.exit(&mut self.ctx);
                        }
                        next.enter(&mut self.ctx);
                        self.states.push(next);
                    }
                    StackOp::Reset(mut next) => {
                        while let Some(mut gone) = self.states.pop() {
                            gone.exit(&mut self.ctx);
                        }
                        next.enter(&mut self.ctx);
                        self.states.push(next);
                    }
                }
            }
        }
        if self.ctx.has_pending() {
            log::error!("screen changes kept queueing themselves; dropping the rest");
            self.ctx.clear_pending();
        }
        if self.states.is_empty() {
            self.ctx.running = false;
        }
    }
}
