mod common;

use common::*;
use hearken_app::{Bus, Headless, Sound, UiSound, Vec3};

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
    h.app.ctx.game.occluded = true;
    h.frame(0.016);
    let ch = h.sounds.channel_of("sfx/door").unwrap();
    assert_eq!(h.sounds.last_mix(ch).unwrap().lowpass_hz, Some(800.0));
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
