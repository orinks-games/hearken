use hearken_app::screens::PauseScreen;
use hearken_app::text::sentence_start;
use hearken_app::{Ctx, InputEvent, Key, State};

use crate::{HelloAction, HelloGame, Title, HELP};

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

/// "Space", or "The A button" on a controller, to open a sentence.
fn count_control(ctx: &Ctx<HelloGame>) -> String {
    sentence_start(&ctx.control_name(HelloAction::Count))
}

impl State<HelloGame> for Practice {
    fn name(&self) -> &'static str {
        "practice"
    }

    fn enter(&mut self, ctx: &mut Ctx<HelloGame>) {
        let control = count_control(ctx);
        ctx.say(format!(
            "The practice room. {control} counts, Escape pauses."
        ));
    }

    fn resume(&mut self, ctx: &mut Ctx<HelloGame>) {
        ctx.say(format!("The practice room. The count is {}.", self.count));
    }

    fn handle_input(&mut self, ctx: &mut Ctx<HelloGame>, event: &InputEvent) {
        // A held key repeats; one press is one count, one pause.
        if event.is_repeat() {
            return;
        }
        if ctx.action_for(event) == Some(HelloAction::Count) {
            self.count += 1;
            ctx.say(self.count.to_string());
        } else if event.pressed() == Some(Key::Escape) {
            ctx.push(PauseScreen::<HelloGame>::new(HELP, to_title));
        }
    }

    fn lines(&self, _ctx: &Ctx<HelloGame>) -> Vec<String> {
        vec![format!("Practice room. Count: {}", self.count)]
    }

    fn help(&self, ctx: &Ctx<HelloGame>) -> String {
        format!(
            "{} counts out loud. Escape opens the pause menu.",
            count_control(ctx)
        )
    }
}
