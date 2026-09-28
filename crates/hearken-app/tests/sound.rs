mod common;

use common::*;
use hearken_app::{
    Backend, Bus, Ctx, Game, Headless, MemoryClips, NoAction, NoCategory, Sound, SoundConfig,
    State, UiSound, Vec3,
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

/// A game that tunes its own fades, so the built-in defaults are a value a
/// game can change, not a private constant.
struct FadesGame;

struct FadesBlank;

impl State<FadesGame> for FadesBlank {
    fn name(&self) -> &'static str {
        "blank"
    }
    fn help(&self, _ctx: &Ctx<FadesGame>) -> String {
        String::new()
    }
}

impl Game for FadesGame {
    const TITLE: &'static str = "Fades Test";
    const DATA_DIR: &'static str = "HearkenFadesTest";
    const ENV_PREFIX: &'static str = "HEARKEN_FADES_TEST";
    type Event = ();
    type Data = ();
    type Settings = ();
    type Action = NoAction;
    type Category = NoCategory;

    fn sound() -> SoundConfig {
        SoundConfig {
            fades: hearken_app::Fades {
                bed_ms: 10,
                ..hearken_app::Fades::default()
            },
            ..SoundConfig::default()
        }
    }
}

#[test]
fn a_games_own_fades_reach_the_pool() {
    let h = Headless::<FadesGame>::new((), FadesBlank);
    assert_eq!(h.app.ctx.audio.fades.bed_ms, 10);
    assert_eq!(
        h.app.ctx.audio.fades.music_ms,
        hearken_app::Fades::default().music_ms,
        "everything else keeps the built-in default"
    );
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

/// A game whose own backend fails to open. `run`'s `open_backend` hands this
/// straight back, the same `Result` a failed default backend would return.
struct FailingBackendGame;

impl Game for FailingBackendGame {
    const TITLE: &'static str = "Failing Backend Test";
    const DATA_DIR: &'static str = "HearkenFailingBackendTest";
    const ENV_PREFIX: &'static str = "HEARKEN_FAILING_BACKEND_TEST";
    type Event = ();
    type Data = ();
    type Settings = ();
    type Action = NoAction;
    type Category = NoCategory;

    fn audio_backend() -> Option<Result<Box<dyn Backend>, String>> {
        Some(Err("this game's backend could not be opened".to_string()))
    }
}

#[test]
fn a_games_own_backend_reporting_failure_reaches_the_no_sound_notice() {
    // `Some(Err(..))` from the game's hook must type-check as the closure
    // `Audio::deferred` runs off the game loop, and its error must reach the
    // player the same way a failed default backend's would.
    let mut audio = hearken_audio::Audio::deferred(
        || FailingBackendGame::audio_backend().expect("the game supplies a backend"),
        Box::new(MemoryClips::default()),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while audio.opening() && std::time::Instant::now() < deadline {
        audio.update(&|_| false);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!audio.opening());
    assert_eq!(
        audio.take_notice().as_deref(),
        Some(hearken_audio::NO_SOUND_NOTICE)
    );
}

#[test]
fn a_failed_open_speaks_the_no_sound_notice_exactly_once() {
    let mut h = start();
    h.clear();
    h.app.ctx.install_audio(hearken_audio::Audio::deferred(
        || Err("no device".to_string()),
        Box::new(MemoryClips::default()),
    ));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while h.app.ctx.audio.opening() && std::time::Instant::now() < deadline {
        h.frame(0.01);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!h.app.ctx.audio.opening());
    assert_eq!(h.spoken(), [hearken_audio::NO_SOUND_NOTICE]);
    // Further frames must not repeat it: the notice is one-shot.
    h.frame(0.01);
    h.frame(0.01);
    assert_eq!(h.spoken(), [hearken_audio::NO_SOUND_NOTICE]);
}
