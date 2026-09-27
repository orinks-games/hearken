#![allow(dead_code)]

use hearken_app::{Ctx, Game, InputEvent, Key, State};

pub struct TestGame;

#[derive(Default)]
pub struct TestData {
    pub log: Vec<String>,
    pub queued_events: Vec<u32>,
}

impl Game for TestGame {
    const TITLE: &'static str = "Test Game";
    const DATA_DIR: &'static str = "HearkenTestGame";
    const ENV_PREFIX: &'static str = "HEARKEN_TEST_GAME";
    type Event = u32;
    type Data = TestData;
    type Settings = ();

    fn pump(ctx: &mut Ctx<Self>, _dt: f64) -> Vec<u32> {
        std::mem::take(&mut ctx.game.queued_events)
    }

    fn observe(ctx: &mut Ctx<Self>, event: &u32) {
        ctx.game.log.push(format!("observe {event}"));
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
        if let Some((key, _, _)) = event.key_down() {
            ctx.game.log.push(format!("{} key {key:?}", self.name));
            if key == Key::Return {
                if let Some(f) = self.on_return.as_mut() {
                    f(ctx);
                }
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
