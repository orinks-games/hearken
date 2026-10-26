//! `FakeVoice`: commentary for `Headless` tests, with no model and no
//! waiting. Lines are recorded the moment they are spoken.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use hearken_app::{Category, Commentary, Ctx, EventPriority, Game, Line};

use crate::settings::{HasVoiceSettings, VoiceMode};

/// One line the fake voiced.
#[derive(Clone, Debug, PartialEq)]
pub struct Voiced {
    pub speaker: String,
    /// The text after the verbosity rules (a terse line's terse text).
    pub text: String,
    pub priority: EventPriority,
    /// The line's category id, if it had one.
    pub category: Option<&'static str>,
}

/// The fake's record, shared with the test that installed it.
#[derive(Clone, Default)]
pub struct VoicedLog {
    lines: Rc<RefCell<Vec<Voiced>>>,
    primed: Rc<RefCell<Vec<String>>>,
    stops: Rc<RefCell<u32>>,
}

impl VoicedLog {
    /// Every line voiced since the last `clear`.
    pub fn voiced(&self) -> Vec<Voiced> {
        self.lines.borrow().clone()
    }

    /// Just the text of each.
    pub fn texts(&self) -> Vec<String> {
        self.lines.borrow().iter().map(|v| v.text.clone()).collect()
    }

    /// The text of every line primed.
    pub fn primed(&self) -> Vec<String> {
        self.primed.borrow().clone()
    }

    /// How many times the voice was stopped (F2, a pause).
    pub fn stops(&self) -> u32 {
        *self.stops.borrow()
    }

    pub fn clear(&self) {
        self.lines.borrow_mut().clear();
        self.primed.borrow_mut().clear();
        *self.stops.borrow_mut() = 0;
    }
}

/// Commentary that voices every line at once into a [`VoicedLog`],
/// following the player's voice mode and verbosity level as [`crate::Voice`]
/// does: `ScreenReader` sends lines to the event voice, `Off` drops them.
pub struct FakeVoice {
    log: VoicedLog,
    first_times: HashSet<(String, String)>,
}

impl FakeVoice {
    /// The fake and the handle a test reads it through.
    pub fn new() -> (FakeVoice, VoicedLog) {
        let log = VoicedLog::default();
        (
            FakeVoice {
                log: log.clone(),
                first_times: HashSet::new(),
            },
            log,
        )
    }
}

impl<G: Game> Commentary<G> for FakeVoice
where
    G::Settings: HasVoiceSettings,
{
    fn speak(&mut self, ctx: &mut Ctx<G>, line: Line<G::Category>) {
        match ctx.settings.game.voice_settings().mode {
            VoiceMode::Off => return,
            VoiceMode::ScreenReader => return ctx.say_line(line),
            VoiceMode::Ai => {}
        }
        let Some(text) = crate::rules::resolve(ctx, &line, &mut self.first_times) else {
            return;
        };
        ctx.transcribe(format!("[voice] {text}"));
        self.log.lines.borrow_mut().push(Voiced {
            speaker: line.speaker,
            text,
            priority: line.priority,
            category: line.category.map(Category::id),
        });
    }

    fn prime(&mut self, _ctx: &mut Ctx<G>, lines: &[Line<G::Category>]) {
        self.log
            .primed
            .borrow_mut()
            .extend(lines.iter().map(|l| l.text.clone()));
    }

    fn stop(&mut self, _ctx: &mut Ctx<G>) {
        *self.log.stops.borrow_mut() += 1;
    }

    fn speakers(&self) -> Vec<String> {
        crate::KOKORO_VOICES.iter().map(|v| v.to_string()).collect()
    }
}
