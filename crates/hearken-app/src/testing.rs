//! A headless app for tests: captured speech, a throwaway data folder, no
//! window. The capture answers like a player with a real voice: a screen
//! reader that can change rate, and a separate event voice.

use std::cell::RefCell;
use std::rc::Rc;

use hearken_speech::{CaptureSpeech, SharedCapture};

use crate::app::App;
use crate::ctx::Ctx;
use crate::game::Game;
use crate::input::{InputEvent, Key};
use crate::paths::Paths;
use crate::settings::Settings;
use crate::state::State;

pub struct Headless<G: Game> {
    pub app: App<G>,
    pub speech: Rc<RefCell<CaptureSpeech>>,
    _dir: tempfile::TempDir,
}

impl<G: Game> Headless<G> {
    pub fn new(game: G::Data, first: impl State<G> + 'static) -> Self {
        let dir = tempfile::tempdir().expect("a temporary data folder");
        let speech = Rc::new(RefCell::new(CaptureSpeech::full_voice()));
        let ctx = Ctx::new(
            Box::new(SharedCapture(speech.clone())),
            Settings::default(),
            Paths::at(dir.path()),
            game,
        );
        Headless {
            app: App::new(ctx, Box::new(first)),
            speech,
            _dir: dir,
        }
    }

    /// One key press, then the rest of that frame.
    pub fn press(&mut self, key: Key) {
        self.send(InputEvent::key(key));
    }

    pub fn send(&mut self, event: InputEvent) {
        self.app.step(vec![event], 0.0);
    }

    pub fn frame(&mut self, dt: f64) {
        self.app.step(Vec::new(), dt);
    }

    /// Main-channel lines spoken since the last `clear`.
    pub fn spoken(&self) -> Vec<String> {
        self.speech.borrow().main_lines()
    }

    pub fn clear(&mut self) {
        self.speech.borrow_mut().clear();
    }
}
