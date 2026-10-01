use std::sync::{mpsc, Arc};
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

fn opened(rec_backend: RecordingBackend) -> (Audio, std::sync::mpsc::Sender<()>) {
    let (go, wait) = mpsc::channel::<()>();
    let audio = Audio::deferred(
        move || {
            wait.recv().ok();
            Ok(Box::new(rec_backend) as Box<dyn Backend>)
        },
        Box::new(MemoryClips::default()),
    );
    (audio, go)
}

fn wait_for_open(audio: &mut Audio) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while audio.opening() && Instant::now() < deadline {
        audio.update(&none);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!audio.opening());
}

#[test]
fn a_paused_loop_stays_paused_after_the_device_opens() {
    let (backend, rec) = RecordingBackend::new();
    let (mut audio, go) = opened(backend);
    let manual = audio.start_loop("sfx/manual", None, Bus::EFFECTS);
    audio.pause(manual);
    audio.start_loop("sfx/all_paused", None, Bus::EFFECTS);
    audio.pause_all();
    go.send(()).unwrap();
    wait_for_open(&mut audio);

    let manual_ch = rec.channel_of("sfx/manual").unwrap();
    let all_paused_ch = rec.channel_of("sfx/all_paused").unwrap();
    assert!(rec.calls().contains(&Call::Pause {
        channel: manual_ch,
        paused: true
    }));
    assert!(rec.calls().contains(&Call::Pause {
        channel: all_paused_ch,
        paused: true
    }));
}

#[test]
fn a_loop_the_opened_backend_refuses_falls_back_to_its_default_and_is_noted_missing() {
    let (backend, rec) = RecordingBackend::new();
    let (go, wait) = mpsc::channel::<()>();
    let clips = MemoryClips::default().with("sfx/hum", &b"hum"[..]);
    let mut audio = Audio::deferred(
        move || {
            wait.recv().ok();
            Ok(Box::new(backend) as Box<dyn Backend>)
        },
        Box::new(clips),
    );
    let hum = audio.start_loop("sfx/hum", None, Bus::EFFECTS);
    assert!(
        audio.missing_clips().is_empty(),
        "the clip exists; not missing yet"
    );
    rec.fail_next_play("no voice channels left");
    go.send(()).unwrap();
    wait_for_open(&mut audio);

    assert!(
        audio.is_playing(hum),
        "falls back to the default clip instead of being dropped"
    );
    assert_eq!(audio.missing_clips(), ["sfx/hum"]);
    let default = hearken_synth::builtin_wav("sfx/hum");
    assert!(
        matches!(rec.calls().last(), Some(Call::Play { clip: Some(b), .. }) if **b == *default)
    );
}

#[test]
fn a_voice_started_while_opening_is_replayed_and_keeps_its_parameters() {
    use hearken_synth::fundsp::prelude32::*;
    fn hum(p: &hearken_synth::Params) -> Box<dyn AudioUnit> {
        Box::new(var(&p.get("throttle")))
    }
    let (backend, rec) = RecordingBackend::new();
    let (mut audio, go) = opened(backend);
    let v = audio.start_voice(hum, &[("throttle", 0.0)], None, hearken_audio::Bus::EFFECTS);
    assert!(audio.set_voice_param(v, "throttle", 0.8));
    go.send(()).unwrap();
    wait_for_open(&mut audio);

    assert_eq!(
        rec.played(),
        ["voice"],
        "the voice is replayed once, onto the opened backend"
    );
    assert!(audio.is_playing(v), "the handle survives the swap");
    let live = audio.live();
    let entry = live
        .iter()
        .find(|l| l.sound == v)
        .expect("the voice is still live");
    assert_eq!(entry.params.as_ref().unwrap().value("throttle"), Some(0.8));
}

/// A feed with nothing to say: the pool only needs to hand it on.
struct Quiet;

impl Feed for Quiet {
    fn read(&self, _out: &mut [[f32; 2]]) -> FeedRead {
        FeedRead {
            frames: 0,
            sample_rate: 0,
        }
    }
}

#[test]
fn a_feed_moves_over_when_the_device_opens() {
    let (backend, rec) = RecordingBackend::new();
    let (mut audio, go) = opened(backend);
    let radio = audio.start_feed(Arc::new(Quiet), None, Bus::MUSIC);
    go.send(()).unwrap();
    wait_for_open(&mut audio);

    assert_eq!(rec.played(), ["feed"], "the feed is replayed once");
    assert!(
        matches!(rec.calls().last(), Some(Call::Play { clip: None, .. })),
        "a feed is not a clip"
    );
    assert!(audio.is_playing(radio), "the handle survives the swap");
}
