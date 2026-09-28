#![allow(dead_code)]

use hearken_app::{
    Action, Chord, ControllerButton, Ctx, Game, InputEvent, Key, PadChord, State, Vec3,
};

pub struct TestGame;

#[derive(Default)]
pub struct TestData {
    pub log: Vec<String>,
    pub queued_events: Vec<u32>,
    pub occluded: bool,
}

impl Game for TestGame {
    const TITLE: &'static str = "Test Game";
    const DATA_DIR: &'static str = "HearkenTestGame";
    const ENV_PREFIX: &'static str = "HEARKEN_TEST_GAME";
    type Event = u32;
    type Data = TestData;
    type Settings = ();
    type Action = TestAction;
    type Category = hearken_app::NoCategory;

    fn pump(ctx: &mut Ctx<Self>, _dt: f64) -> Vec<u32> {
        std::mem::take(&mut ctx.game.queued_events)
    }

    fn observe(ctx: &mut Ctx<Self>, event: &u32) {
        ctx.game.log.push(format!("observe {event}"));
    }

    fn occluded(data: &TestData, _listener: Vec3, _pos: Vec3) -> bool {
        data.occluded
    }
}

/// Jump: Space, pad A. Duck: C or keypad 0, pad X. Open the map: Shift M,
/// keyboard only. Horn: right bumper plus Y, pad only. Tab is the game's
/// own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TestAction {
    Jump,
    Duck,
    Map,
    Horn,
}

impl Action for TestAction {
    fn all() -> &'static [Self] {
        &[
            TestAction::Jump,
            TestAction::Duck,
            TestAction::Map,
            TestAction::Horn,
        ]
    }
    fn id(self) -> &'static str {
        match self {
            TestAction::Jump => "jump",
            TestAction::Duck => "duck",
            TestAction::Map => "map",
            TestAction::Horn => "horn",
        }
    }
    fn label(self) -> &'static str {
        match self {
            TestAction::Jump => "Jump",
            TestAction::Duck => "Duck",
            TestAction::Map => "Open the map",
            TestAction::Horn => "Horn",
        }
    }
    fn default_keys(self) -> &'static [Chord] {
        match self {
            TestAction::Jump => const { &[Chord::plain(Key::Space)] },
            TestAction::Duck => const { &[Chord::plain(Key::C), Chord::plain(Key::Kp0)] },
            TestAction::Map => const { &[Chord::shift(Key::M)] },
            TestAction::Horn => &[],
        }
    }
    fn default_pad(self) -> &'static [PadChord] {
        match self {
            TestAction::Jump => const { &[PadChord::plain(ControllerButton::A)] },
            TestAction::Duck => const { &[PadChord::plain(ControllerButton::X)] },
            TestAction::Map => &[],
            TestAction::Horn => const { &[PadChord::modified(ControllerButton::Y)] },
        }
    }
    fn reserved_key(chord: &Chord) -> Option<&'static str> {
        (chord.key == Key::Tab).then_some("Tab reads the score")
    }
}

/// A screen that writes every hook it sees into `ctx.game.log`, and runs
/// `on_return` when Return is pressed.
pub struct Probe {
    pub name: &'static str,
    #[allow(clippy::type_complexity)]
    on_return: Option<Box<dyn FnMut(&mut Ctx<TestGame>)>>,
}

impl Probe {
    pub fn new(name: &'static str) -> Self {
        Probe {
            name,
            on_return: None,
        }
    }

    pub fn on_return(mut self, f: impl FnMut(&mut Ctx<TestGame>) + 'static) -> Self {
        self.on_return = Some(Box::new(f));
        self
    }
}

impl State<TestGame> for Probe {
    fn name(&self) -> &'static str {
        self.name
    }
    fn enter(&mut self, ctx: &mut Ctx<TestGame>) {
        ctx.game.log.push(format!("{} enter", self.name));
    }
    fn exit(&mut self, ctx: &mut Ctx<TestGame>) {
        ctx.game.log.push(format!("{} exit", self.name));
    }
    fn resume(&mut self, ctx: &mut Ctx<TestGame>) {
        ctx.game.log.push(format!("{} resume", self.name));
    }
    fn handle_input(&mut self, ctx: &mut Ctx<TestGame>, event: &InputEvent) {
        let Some((key, _, _)) = event.key_down() else {
            ctx.game.log.push(format!("{} input {event:?}", self.name));
            return;
        };
        ctx.game.log.push(format!("{} key {key:?}", self.name));
        if key == Key::Return {
            if let Some(f) = self.on_return.as_mut() {
                f(ctx);
            }
        }
    }
    fn on_event(&mut self, ctx: &mut Ctx<TestGame>, event: &u32) {
        ctx.game.log.push(format!("{} event {event}", self.name));
    }
    fn update(&mut self, ctx: &mut Ctx<TestGame>, _dt: f64) {
        ctx.game.log.push(format!("{} update", self.name));
    }
    fn lines(&self, _ctx: &Ctx<TestGame>) -> Vec<String> {
        vec![self.name.to_string()]
    }
    fn help(&self, _ctx: &Ctx<TestGame>) -> String {
        format!("{} help", self.name)
    }
}

/// The log since the last call, without the per-frame update lines.
pub fn take_log(ctx: &mut Ctx<TestGame>) -> Vec<String> {
    std::mem::take(&mut ctx.game.log)
        .into_iter()
        .filter(|line| !line.ends_with(" update"))
        .collect()
}
