mod common;

use common::*;
use hearken_app::{
    Bus, Ctx, Game, Headless, NoAction, NoCategory, Sound, SoundConfig, State, UiSound, Vec3,
};

fn start() -> Headless<TestGame> {
    Headless::new(TestData::default(), Probe::new("root"))
}

#[test]
fn a_headless_game_records_what_it_plays() {
    let mut h = start();
    h.app.ctx.play_ui(UiSound::Move);
    assert_eq!(h.played(), ["ui/menu_move"]);
    h.clear();
    assert!(h.played().is_empty());
}

#[test]
fn a_game_points_an_earcon_at_its_own_sound_or_at_nothing() {
    let mut h = start();
    h.app.ctx.ui_sounds.set(UiSound::Move, Some("ui/my_tick"));
    h.app.ctx.play_ui(UiSound::Move);
    h.app.ctx.ui_sounds.set(UiSound::Move, None);
    assert_eq!(h.app.ctx.play_ui(UiSound::Move), Sound::NONE);
    assert_eq!(h.played(), ["ui/my_tick"]);
}

#[test]
fn saved_volumes_reach_the_buses_and_an_unsaved_one_is_eighty_percent() {
    let mut h = start();
    // Applied once already, on install, before any test ever touches
    // `settings`: the startup path is covered, not just a later re-apply.
    assert_eq!(h.app.ctx.audio.bus_volume(Bus::EFFECTS), 0.8);
    h.app.ctx.settings.audio.volumes.insert("music".into(), 40);
    h.app.ctx.apply_audio_settings();
    let audio = &h.app.ctx.audio;
    assert_eq!(audio.bus_volume(Bus::MUSIC), 0.4);
    assert_eq!(audio.bus_volume(Bus::EFFECTS), 0.8);
    assert_eq!(audio.bus_volume(Bus::MASTER), 1.0);
}

#[test]
fn the_games_occlusion_hook_is_asked_every_frame() {
    let mut h = start();
    h.app
        .ctx
        .audio
        .play_at("sfx/door", Vec3::new(0.0, 1.0, 0.0), Bus::EFFECTS);
    h.frame(0.016);
    let ch = h.sounds.channel_of("sfx/door").unwrap();
    assert_eq!(h.sounds.last_mix(ch).unwrap().lowpass_hz, None);
    h.app.ctx.game.occluded = true;
    h.frame(0.016);
    assert_eq!(h.sounds.last_mix(ch).unwrap().lowpass_hz, Some(800.0));
}

/// A game with its own default volume for one bus, and its own state so it
/// can run headless without the shared `TestGame` fixture.
struct DefaultVolumeGame;

struct Blank;

impl State<DefaultVolumeGame> for Blank {
    fn name(&self) -> &'static str {
        "blank"
    }
    fn help(&self, _ctx: &Ctx<DefaultVolumeGame>) -> String {
        String::new()
    }
}

impl Game for DefaultVolumeGame {
    const TITLE: &'static str = "Default Volume Test";
    const DATA_DIR: &'static str = "HearkenDefaultVolumeTest";
    const ENV_PREFIX: &'static str = "HEARKEN_DEFAULT_VOLUME_TEST";
    type Event = ();
    type Data = ();
    type Settings = ();
    type Action = NoAction;
    type Category = NoCategory;

    fn sound() -> SoundConfig {
        SoundConfig {
            default_volume: |bus| {
                if bus == Bus::MUSIC {
                    50
                } else {
                    hearken_app::default_volume(bus)
                }
            },
            ..SoundConfig::default()
        }
    }
}

#[test]
fn a_games_own_default_volume_applies_without_saving_anything() {
    let h = Headless::<DefaultVolumeGame>::new((), Blank);
    assert_eq!(h.app.ctx.audio.bus_volume(Bus::MUSIC), 0.5);
    assert!(h.app.ctx.settings.audio.volumes.is_empty());
}

#[test]
fn ending_the_run_stops_every_sound() {
    let mut h = start();
    h.app.ctx.audio.start_loop("amb/hum", None, Bus::AMBIENCE);
    h.app.ctx.quit();
    h.app.run(&mut hearken_app::HeadlessShell::default());
    assert!(h
        .sounds
        .calls()
        .iter()
        .any(|c| matches!(c, hearken_app::Call::Stop { .. })));
    assert!(h.app.ctx.audio.live().is_empty());
}
