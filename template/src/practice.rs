use hearken_app::screens::PauseScreen;
use hearken_app::{Ctx, InputEvent, Key, State};

use crate::{HelloGame, Title, HELP};

/// A room with one thing to do: count out loud.
pub struct Practice {
    count: u32,
}

impl Practice {
    pub fn new() -> Self {
        Practice { count: 0 }
    }
}

impl Default for Practice {
    fn default() -> Self {
        Self::new()
    }
}

fn to_title(ctx: &mut Ctx<HelloGame>) {
    ctx.reset_to(Title::new());
}

impl State<HelloGame> for Practice {
    fn name(&self) -> &'static str {
        "practice"
    }

    fn enter(&mut self, ctx: &mut Ctx<HelloGame>) {
        ctx.say("The practice room. Space counts, Escape pauses.");
    }

    fn resume(&mut self, ctx: &mut Ctx<HelloGame>) {
        ctx.say(format!("The practice room. The count is {}.", self.count));
    }

    fn handle_input(&mut self, ctx: &mut Ctx<HelloGame>, event: &InputEvent) {
        // A held key repeats; one press is one count, one pause.
        if event.is_repeat() {
            return;
        }
        let Some((key, _, _)) = event.key_down() else {
            return;
        };
        match key {
            Key::Space => {
                self.count += 1;
                ctx.say(self.count.to_string());
            }
            Key::Escape => ctx.push(PauseScreen::<HelloGame>::new(HELP, to_title)),
            _ => {}
        }
    }

    fn lines(&self, _ctx: &Ctx<HelloGame>) -> Vec<String> {
        vec![format!("Practice room. Count: {}", self.count)]
    }

    fn help(&self, _ctx: &Ctx<HelloGame>) -> String {
        "Space counts out loud. Escape opens the pause menu.".to_string()
    }
}
