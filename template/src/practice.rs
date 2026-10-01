use std::f32::consts::TAU;

use hearken::screens::PauseScreen;
use hearken::text::sentence_start;
use hearken::{Bus, Ctx, InputEvent, Key, Sound, State, Vec3};

use crate::sound::engine;
use crate::{HelloAction, HelloGame, Title, HELP};

/// How often the beacon pings. A game would reasonably tune this.
const PING_SECONDS: f64 = 0.8;

/// A room with two things to do: count out loud, and listen to an engine
/// idle while a beacon circles you.
pub struct Practice {
    count: u32,
    engine: Sound,
    throttle: f32,
    beacon_angle: f32,
    ping_in: f64,
}

impl Practice {
    pub fn new() -> Self {
        Practice {
            count: 0,
            engine: Sound::NONE,
            throttle: 0.0,
            beacon_angle: 0.0,
            ping_in: PING_SECONDS,
        }
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
        self.engine = ctx
            .audio
            .start_voice(engine, &[("throttle", 0.0)], None, Bus::EFFECTS);
        self.ping_in = PING_SECONDS;
        let count = count_control(ctx);
        let throttle = ctx.control_name(HelloAction::Throttle);
        ctx.say(format!(
            "The practice room. A beacon circles you. {count} counts, hold {throttle} to rev the engine, Escape pauses."
        ));
    }

    fn exit(&mut self, ctx: &mut Ctx<HelloGame>) {
        ctx.audio.stop(self.engine, 200);
    }

    /// Pause already resumed the engine and the beacon: nothing to say about
    /// sound here, only the count.
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

    fn update(&mut self, ctx: &mut Ctx<HelloGame>, dt: f64) {
        self.beacon_angle += dt as f32 * 0.2 * TAU;

        let target = if ctx.action_held(HelloAction::Throttle) {
            1.0
        } else {
            0.0
        };
        self.throttle += (target - self.throttle) * (dt as f32 * 3.0).min(1.0);
        ctx.audio
            .set_voice_param(self.engine, "throttle", self.throttle);

        self.ping_in -= dt;
        if self.ping_in <= 0.0 {
            self.ping_in += PING_SECONDS;
            let a = self.beacon_angle;
            ctx.audio.play_at(
                "sfx/beacon",
                Vec3::new(4.0 * a.sin(), 4.0 * a.cos(), 0.0),
                Bus::EFFECTS,
            );
        }
    }

    fn lines(&self, _ctx: &Ctx<HelloGame>) -> Vec<String> {
        vec![format!("Practice room. Count: {}", self.count)]
    }

    fn help(&self, ctx: &Ctx<HelloGame>) -> String {
        format!(
            "{} counts out loud. Hold {} to rev the engine. Escape opens the pause menu.",
            count_control(ctx),
            ctx.control_name(HelloAction::Throttle)
        )
    }
}
