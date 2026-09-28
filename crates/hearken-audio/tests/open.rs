use std::sync::mpsc;
use std::time::{Duration, Instant};

use hearken_audio::*;

fn none(_: Vec3) -> bool {
    false
}

#[test]
fn the_game_runs_silent_until_the_device_opens_then_its_loops_and_volumes_move_over() {
    let (backend, rec) = RecordingBackend::new();
    let (go, wait) = mpsc::channel::<()>();
    let mut audio = Audio::deferred(
        move || {
            wait.recv().ok();
            Ok(Box::new(backend) as Box<dyn Backend>)
        },
        Box::new(MemoryClips::default()),
    );
    assert!(audio.opening());
    audio.set_bus_volume(Bus::MUSIC, 0.5);
    audio.play_music(Some("music/title"), true);
    let hum = audio.start_loop("sfx/hum", None, Bus::EFFECTS);
    let blip = audio.play("ui/menu_move", Bus::UI);
    go.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while audio.opening() && Instant::now() < deadline {
        audio.update(&none);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!audio.opening());
    assert_eq!(
        rec.played(),
        ["music/title", "sfx/hum"],
        "one-shots are not replayed"
    );
    assert_eq!(
        rec.last_mix(rec.channel_of("music/title").unwrap())
            .unwrap()
            .gain,
        0.5
    );
    assert!(audio.is_playing(hum), "the handle survives the move");
    assert!(!audio.is_playing(blip));
    assert_eq!(audio.take_notice(), None);
}

#[test]
fn a_device_that_will_not_open_leaves_silence_and_says_so_once() {
    let mut audio = Audio::deferred(
        || Err("no device".to_string()),
        Box::new(MemoryClips::default()),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while audio.opening() && Instant::now() < deadline {
        audio.update(&none);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(audio.take_notice().as_deref(), Some(NO_SOUND_NOTICE));
    assert_eq!(audio.take_notice(), None);
    assert_eq!(audio.backend_name(), "silent");
}

#[test]
fn quitting_while_the_device_is_still_opening_is_bounded() {
    let mut audio = Audio::deferred(
        || {
            std::thread::sleep(Duration::from_secs(10));
            Ok(Box::new(SilentBackend::default()) as Box<dyn Backend>)
        },
        Box::new(MemoryClips::default()),
    );
    let started = Instant::now();
    audio.shutdown();
    assert!(started.elapsed() < SHUTDOWN_WAIT + Duration::from_millis(500));
    audio.shutdown(); // idempotent
}
