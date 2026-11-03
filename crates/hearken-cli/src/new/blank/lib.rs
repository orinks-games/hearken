//! Hello Hearken: a blank Hearken game. One screen: Space plays a sound,
//! F1 says the keys, Escape quits. Grow it from here.

use hearken::{Action, Bus, Chord, ControllerButton, Ctx, InputEvent, Key, PadChord, State};

/// The help document, compiled in.
pub const HELP: &str = include_str!("../help.md");

pub struct HelloGame;

impl hearken::Game for HelloGame {
    const TITLE: &'static str = "Hello Hearken";
    const DATA_DIR: &'static str = "HelloHearken";
    const ENV_PREFIX: &'static str = "HELLO_HEARKEN";
    type Event = ();
    type Data = ();
    type Settings = ();
    type Action = HelloAction;
    type Category = hearken::NoCategory;
}

/// The controls the player can move to other keys and buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HelloAction {
    Ping,
}

impl Action for HelloAction {
    fn all() -> &'static [Self] {
        &[HelloAction::Ping]
    }
    fn id(self) -> &'static str {
        "ping"
    }
    fn label(self) -> &'static str {
        "Ping"
    }
    fn default_keys(self) -> &'static [Chord] {
        const { &[Chord::plain(Key::Space)] }
    }
    fn default_pad(self) -> &'static [PadChord] {
        const { &[PadChord::plain(ControllerButton::A)] }
    }
}

/// The only screen.
#[derive(Default)]
pub struct Start;

impl State<HelloGame> for Start {
    fn name(&self) -> &'static str {
        "start"
    }

    fn enter(&mut self, ctx: &mut Ctx<HelloGame>) {
        let ping = ctx.control_name(HelloAction::Ping);
        ctx.say(format!("Hello Hearken. {ping} pings, Escape quits."));
    }

    fn handle_input(&mut self, ctx: &mut Ctx<HelloGame>, event: &InputEvent) {
        if event.is_repeat() {
            return;
        }
        if ctx.action_for(event) == Some(HelloAction::Ping) {
            // sounds/ping.ogg (or .wav, .opus, ...); a beep until it exists.
            ctx.audio.play("ping", Bus::EFFECTS);
        } else if event.pressed() == Some(Key::Escape) {
            ctx.quit();
        }
    }

    fn lines(&self, _ctx: &Ctx<HelloGame>) -> Vec<String> {
        vec!["Hello Hearken".to_string()]
    }

    fn help(&self, ctx: &Ctx<HelloGame>) -> String {
        format!(
            "{} plays the ping sound. Escape quits.",
            ctx.control_name(HelloAction::Ping)
        )
    }
}
