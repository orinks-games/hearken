mod common;

use common::*;
use hearken_app::{
    Ctx, EventPriority, Game, Headless, NoAction, NoCategory, SayEvent, SoundConfig, State,
};
use hearken_speech::SpeechChannel;

fn start() -> Headless<TestGame> {
    Headless::new(TestData::default(), Probe::new("root"))
}

fn events(h: &Headless<TestGame>) -> Vec<String> {
    h.speech.borrow().event_lines()
}

fn say(h: &mut Headless<TestGame>, text: &str, opts: SayEvent<TestCategory>) {
    h.app.ctx.say_event_with(text, opts);
}

#[test]
fn an_event_is_spoken_on_the_event_voice_and_kept_in_the_transcript() {
    let mut h = start();
    say(&mut h, "Wind rising.", SayEvent::new());
    assert_eq!(events(&h), ["Wind rising."]);
    assert_eq!(
        h.app.ctx.transcript.last().map(String::as_str),
        Some("[event] Wind rising.")
    );
}

#[test]
fn the_same_line_twice_in_a_moment_is_said_once_and_again_later() {
    let mut h = start();
    say(&mut h, "Bump.", SayEvent::new());
    say(&mut h, "Bump.", SayEvent::new());
    assert_eq!(events(&h), ["Bump."]);
    h.frame(3.0);
    say(&mut h, "Bump.", SayEvent::new());
    assert_eq!(events(&h), ["Bump.", "Bump."]);
}

#[test]
fn a_standing_condition_speaks_again_only_when_it_changes_or_is_cleared() {
    let mut h = start();
    say(&mut h, "Hull damaged.", SayEvent::new().key("hull"));
    h.frame(10.0);
    say(&mut h, "Hull damaged.", SayEvent::new().key("hull"));
    say(&mut h, "Hull badly damaged.", SayEvent::new().key("hull"));
    h.app.ctx.clear_condition("hull");
    h.frame(10.0);
    say(&mut h, "Hull badly damaged.", SayEvent::new().key("hull"));
    assert_eq!(
        events(&h),
        [
            "Hull damaged.",
            "Hull badly damaged.",
            "Hull badly damaged."
        ]
    );
}

#[test]
fn at_quiet_chatter_is_its_earcon_and_no_words() {
    let mut h = start();
    h.app.ctx.settings.speech.verbosity = "quiet".into();
    say(
        &mut h,
        "A gull calls.",
        SayEvent::new().category(TestCategory::Chatter),
    );
    assert!(events(&h).is_empty());
    assert_eq!(h.played(), ["ui/tick"]);
}

#[test]
fn terse_says_the_short_line_and_silent_says_and_plays_nothing() {
    let mut h = start();
    h.app.ctx.settings.speech.verbosity = "minimal".into();
    say(
        &mut h,
        "Brake now, the pier is close.",
        SayEvent::new()
            .category(TestCategory::Safety)
            .terse("Brake!"),
    );
    say(
        &mut h,
        "A gull calls.",
        SayEvent::new().category(TestCategory::Chatter),
    );
    assert_eq!(events(&h), ["Brake!"]);
    assert!(h.played().is_empty());
}

#[test]
fn a_first_time_line_waits_for_reset_first_times() {
    let mut h = start();
    let hint = || SayEvent::new().category(TestCategory::Hint);
    say(&mut h, "Press J to jump.", hint());
    h.frame(10.0);
    say(&mut h, "Press J to jump.", hint());
    h.app.ctx.reset_first_times();
    say(&mut h, "Press J to jump.", hint());
    assert_eq!(events(&h), ["Press J to jump.", "Press J to jump."]);
}

#[test]
fn a_line_the_player_asked_for_skips_the_verbosity_rules() {
    let mut h = start();
    h.app.ctx.settings.speech.verbosity = "quiet".into();
    say(
        &mut h,
        "Tide is high.",
        SayEvent::new().category(TestCategory::Chatter).force(),
    );
    assert_eq!(events(&h), ["Tide is high."]);
}

#[test]
fn a_saved_level_the_game_no_longer_has_is_its_first_level() {
    let mut h = start();
    h.app.ctx.settings.speech.verbosity = "gone".into();
    say(
        &mut h,
        "A gull calls.",
        SayEvent::new().category(TestCategory::Chatter),
    );
    assert_eq!(events(&h), ["A gull calls."]);
    assert_eq!(h.app.ctx.verbosity_level().map(|l| l.id), Some("full"));
}

#[test]
fn the_duck_lowers_music_while_event_speech_talks_and_lifts_after() {
    let mut h = start();
    h.app.ctx.settings.audio.duck_for_speech = true;
    h.app.ctx.audio.play_music(Some("music/x"), true);
    let ch = h.sounds.channel_of("music/x").unwrap();
    say(
        &mut h,
        "Critical warning, the engine is on fire.",
        SayEvent::new()
            .priority(EventPriority::Critical)
            .interrupt(true),
    );
    assert!(h.app.ctx.speech_ducked());
    assert!((h.sounds.last_mix(ch).unwrap().gain - 0.8 * 0.5).abs() < 1e-4);
    for _ in 0..10 {
        h.frame(1.0);
    }
    assert!(!h.app.ctx.speech_ducked());
    assert!((h.sounds.last_mix(ch).unwrap().gain - 0.8).abs() < 1e-4);
}

#[test]
fn with_the_duck_off_speech_leaves_the_music_alone() {
    let mut h = start();
    h.app.ctx.settings.audio.duck_for_speech = false;
    h.app.ctx.audio.play_music(Some("music/x"), true);
    h.sounds.clear();
    say(&mut h, "Hello.", SayEvent::new());
    assert!(!h.app.ctx.speech_ducked());
    assert!(h.sounds.calls().is_empty());
}

#[test]
fn pausing_event_speech_silences_the_event_voice() {
    let mut h = start();
    let before = h.speech.borrow().stop_event_calls();
    h.app.ctx.pause_event_speech();
    assert_eq!(h.speech.borrow().stop_event_calls(), before + 1);
}

#[test]
fn an_important_line_an_interrupt_cuts_off_is_said_again_behind_it() {
    let mut h = start();
    let turn = || SayEvent::new().priority(EventPriority::Important);
    say(&mut h, "Turn left at the pier.", turn());
    say(&mut h, "Brake!", SayEvent::new().interrupt(true));
    assert_eq!(
        h.speech.borrow().calls(SpeechChannel::Event),
        [
            ("Turn left at the pier.".to_string(), false),
            ("Brake!".to_string(), true),
            ("Turn left at the pier.".to_string(), false),
        ]
    );
    assert_eq!(
        h.app.ctx.transcript.last().map(String::as_str),
        Some("[event] Turn left at the pier.")
    );
}

#[test]
fn a_cut_line_no_longer_true_is_not_said_again() {
    let mut h = start();
    say(
        &mut h,
        "Turn left at the pier.",
        SayEvent::new()
            .priority(EventPriority::Important)
            .valid(|| false),
    );
    say(&mut h, "Brake!", SayEvent::new().interrupt(true));
    assert_eq!(events(&h), ["Turn left at the pier.", "Brake!"]);
}

/// A game that switched the pacer off.
struct Unpaced;

impl Game for Unpaced {
    const TITLE: &'static str = "Unpaced";
    const DATA_DIR: &'static str = "HearkenUnpaced";
    const ENV_PREFIX: &'static str = "HEARKEN_UNPACED";
    type Event = ();
    type Data = ();
    type Settings = ();
    type Action = NoAction;
    type Category = NoCategory;

    fn sound() -> SoundConfig {
        SoundConfig {
            pacer: None,
            ..SoundConfig::default()
        }
    }
}

struct Blank;

impl State<Unpaced> for Blank {
    fn name(&self) -> &'static str {
        "blank"
    }
    fn help(&self, _ctx: &Ctx<Unpaced>) -> String {
        String::new()
    }
}

#[test]
fn without_the_pacer_every_line_reaches_the_event_voice() {
    let mut h = Headless::<Unpaced>::new((), Blank);
    h.app.ctx.say_event_with("Bump.", SayEvent::new());
    h.app.ctx.say_event_with("Bump.", SayEvent::new());
    assert_eq!(h.speech.borrow().event_lines(), ["Bump.", "Bump."]);
}
