//! The screen stack and the frame loop that drives it.

use crate::controller::CONTROLLER_DISCONNECTED;
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
        // The clock the held-key pulses run on; see `held_keys`.
        self.ctx.held.begin_frame(dt);
        for event in events {
            if self.admit(&event) {
                self.dispatch(&event);
            }
        }
        // A held D-pad left or right repeats, for adjusting a setting.
        for event in self.ctx.controller.tick(dt) {
            self.dispatch(&event);
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
        self.ctx.update_audio();

        self.apply_pending();
        self.ctx.frame += 1;
        self.ctx.clock += dt;
    }

    /// The bookkeeping every event gets before any screen sees it. False
    /// when no screen should see it at all.
    fn admit(&mut self, event: &InputEvent) -> bool {
        match event {
            InputEvent::Quit => {
                self.ctx.running = false;
                return false;
            }
            // Coming back to the window re-checks which screen reader is
            // running and the keyboard's repeat timing, either of which the
            // player may have changed while away, and nothing held before
            // they left is held now. The screen still sees the event.
            InputEvent::WindowFocusGained => {
                self.ctx.speech.request_refresh();
                self.ctx.held.refresh_repeat_timing();
                self.ctx.held.clear_pulses();
            }
            InputEvent::WindowFocusLost => {
                self.ctx.held.clear();
                self.ctx.controller.release_all();
            }
            InputEvent::KeyDown { key, mods, .. } => {
                self.ctx.held.press(*key, *mods);
                self.ctx.controller.note_keyboard();
            }
            InputEvent::KeyUp { key, mods } => self.ctx.held.release(*key, *mods),
            // Only the bound pad's presses and releases, and its removal,
            // reach a screen.
            e if e.is_controller_event() => {
                if !self.ctx.controller.process_event(e) {
                    return false;
                }
                if self.ctx.controller.take_disconnect() {
                    self.ctx.say(CONTROLLER_DISCONNECTED);
                }
            }
            _ => {}
        }
        true
    }

    /// The keys that work on every screen, else the top screen.
    fn dispatch(&mut self, event: &InputEvent) {
        match event.pressed() {
            // F2 silences speech everywhere, so no screen can hold it.
            Some(Key::F2) => {
                self.ctx.stop_speech();
                return;
            }
            // F1, or a controller's Back button, reads the top screen's
            // help.
            Some(Key::F1) => {
                let help = self.states.last().map(|top| top.help(&self.ctx));
                if let Some(help) = help {
                    self.ctx.say(help);
                }
                return;
            }
            _ => {}
        }
        if let Some(top) = self.states.last_mut() {
            top.handle_input(&mut self.ctx, event);
        }
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
        self.ctx.controller.shutdown();
        self.ctx.audio.shutdown();
        self.ctx.save_settings();
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
        // A new screen never inherits the last one's held keys.
        if self.ctx.has_pending() {
            self.ctx.held.clear_pulses();
        }
        for _ in 0..MAX_STACK_ROUNDS {
            if !self.ctx.has_pending() {
                break;
            }
            // Whether the last change this round was a `Pop`, revealing the
            // screen now on top. Resuming it is deferred until the round has
            // drained: a `Push`, `Replace` or `Reset` later in the same round
            // covers or removes it, so it must never speak an announcement
            // for a screen the player never actually lands on.
            let mut resume = false;
            for op in self.ctx.take_pending() {
                match op {
                    StackOp::Push(mut next) => {
                        next.enter(&mut self.ctx);
                        self.states.push(next);
                        resume = false;
                    }
                    StackOp::Pop => {
                        if let Some(mut gone) = self.states.pop() {
                            gone.exit(&mut self.ctx);
                        }
                        // An empty stack is only judged once the whole queue
                        // has drained: popping the root and pushing its
                        // replacement in one handler is allowed.
                        resume = true;
                    }
                    StackOp::Replace(mut next) => {
                        if let Some(mut gone) = self.states.pop() {
                            gone.exit(&mut self.ctx);
                        }
                        next.enter(&mut self.ctx);
                        self.states.push(next);
                        resume = false;
                    }
                    StackOp::Reset(mut next) => {
                        while let Some(mut gone) = self.states.pop() {
                            gone.exit(&mut self.ctx);
                        }
                        next.enter(&mut self.ctx);
                        self.states.push(next);
                        resume = false;
                    }
                }
            }
            if resume {
                if let Some(top) = self.states.last_mut() {
                    top.resume(&mut self.ctx);
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
