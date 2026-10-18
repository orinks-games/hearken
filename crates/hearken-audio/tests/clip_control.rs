//! Playing clips the game found itself, the no-default switch, per-part
//! ramps, pan without a position, start offsets and clip lengths: what a
//! game with its own asset packs and its own mixer (Freight Fate) needs
//! from the pool.

use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use hearken_audio::*;

/// A clip source the test can add to while the pool holds it, counting
/// every lookup: a game registering a generated sound mid-run.
#[derive(Clone, Default)]
struct Registry {
    clips: Arc<Mutex<HashMap<String, Arc<[u8]>>>>,
    asked: Arc<Mutex<Vec<String>>>,
}

impl Registry {
    fn register(&self, key: &str, bytes: &[u8]) {
        self.clips
            .lock()
            .unwrap()
            .insert(key.to_string(), bytes.into());
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl ClipSource for Registry {
    fn load(&mut self, key: &str) -> Option<Arc<[u8]>> {
        self.asked.lock().unwrap().push(key.to_string());
        self.clips.lock().unwrap().get(key).cloned()
    }
}

fn pool() -> (Audio, Recorder, Registry) {
    let (backend, rec) = RecordingBackend::new();
    let registry = Registry::default();
    registry.register("sfx/door", b"door");
    let audio = Audio::new(Box::new(backend), Box::new(registry.clone()));
    (audio, rec, registry)
}

fn silent_pool() -> (Audio, Recorder, Registry) {
    let (mut audio, rec, registry) = pool();
    audio.set_default_fallback(false);
    (audio, rec, registry)
}

fn last_ramped(rec: &Recorder) -> (Mix, Ramps) {
    rec.calls()
        .into_iter()
        .rev()
        .find_map(|call| match call {
            Call::SetMixRamped { mix, ramps, .. } => Some((mix, ramps)),
            _ => None,
        })
        .expect("a ramped mix change")
}

#[test]
fn with_the_default_off_a_missing_clip_plays_nothing_and_is_listed() {
    let (mut audio, rec, _registry) = silent_pool();
    let sound = audio.play("sfx/step", Bus::EFFECTS);
    assert_eq!(sound, Sound::NONE);
    assert!(rec.calls().is_empty(), "{:?}", rec.calls());
    assert_eq!(audio.missing_clips(), ["sfx/step"]);
    let sound = audio.start_with("sfx/step", Bus::EFFECTS, Start::LOOP);
    assert_eq!(sound, Sound::NONE);
    assert!(rec.calls().is_empty());
}

#[test]
fn with_the_default_off_a_clip_registered_after_a_miss_is_heard() {
    let (mut audio, rec, registry) = silent_pool();
    audio.play("lane/guide", Bus::EFFECTS);
    audio.play("lane/guide", Bus::EFFECTS);
    assert!(rec.played().is_empty());
    assert_eq!(
        registry.asked(),
        ["lane/guide", "lane/guide"],
        "a miss is not remembered: the source is asked again"
    );
    registry.register("lane/guide", b"tone");
    let sound = audio.play("lane/guide", Bus::EFFECTS);
    assert!(audio.is_playing(sound));
    match rec.calls().last() {
        Some(Call::Play { key, clip, .. }) => {
            assert_eq!(key, "lane/guide");
            assert_eq!(clip.as_deref(), Some(&b"tone"[..]));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn with_the_default_on_a_miss_is_remembered_as_before() {
    let (mut audio, rec, registry) = pool();
    audio.play("lane/guide", Bus::EFFECTS);
    registry.register("lane/guide", b"tone");
    audio.play("lane/guide", Bus::EFFECTS);
    let default = hearken_synth::builtin_wav("lane/guide");
    assert!(rec
        .calls()
        .iter()
        .all(|c| matches!(c, Call::Play { clip: Some(b), .. } if **b == *default)));
    assert_eq!(registry.asked(), ["lane/guide"]);
}

#[test]
fn with_the_default_off_a_clip_that_will_not_decode_plays_nothing_and_is_logged_once() {
    let (mut audio, rec, _registry) = silent_pool();
    rec.refuse("sfx/door");
    for _ in 0..3 {
        assert_eq!(audio.play("sfx/door", Bus::EFFECTS), Sound::NONE);
    }
    assert!(rec.calls().is_empty(), "no default stood in");
    assert_eq!(audio.missing_clips(), ["sfx/door"]);
}

#[test]
fn play_clip_plays_the_games_bytes_without_asking_the_clip_source() {
    let (mut audio, rec, registry) = silent_pool();
    let bytes: Arc<[u8]> = Arc::from(&b"engine"[..]);
    let start = Start::LOOP.volume(0.25).pan(-0.5).pitch(1.5);
    let sound = audio.play_clip("engine/idle", bytes.clone(), Bus::EFFECTS, start);
    assert!(audio.is_playing(sound));
    assert!(registry.asked().is_empty());
    match &rec.calls()[0] {
        Call::Play {
            key,
            looping,
            clip,
            mix,
            from_s,
            ..
        } => {
            assert_eq!(key, "engine/idle");
            assert!(*looping);
            assert_eq!(clip.as_ref(), Some(&bytes));
            assert_eq!((mix.gain, mix.pan, mix.pitch), (0.25, -0.5, 1.5));
            assert_eq!(*from_s, 0.0);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_start_clamps_what_it_is_given() {
    let (mut audio, rec, _registry) = silent_pool();
    let start = Start::ONCE.volume(1.7).pan(-3.0).pitch(9.0).from_s(-2.0);
    audio.play_clip("sfx/x", Arc::from(&b"x"[..]), Bus::EFFECTS, start);
    match &rec.calls()[0] {
        Call::Play { mix, from_s, .. } => {
            assert_eq!((mix.gain, mix.pan, mix.pitch), (1.0, -1.0, 4.0));
            assert_eq!(*from_s, 0.0);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_start_offset_and_a_loop_region_reach_the_backend() {
    let (mut audio, rec, _registry) = silent_pool();
    audio.play_clip(
        "music/show",
        Arc::from(&b"show"[..]),
        Bus::MUSIC,
        Start::ONCE.from_s(600.0),
    );
    let horn = audio.start_with(
        "sfx/door",
        Bus::EFFECTS,
        Start::LOOP.region(LoopRegion::new(0.25, 0.5)),
    );
    let calls = rec.calls();
    assert!(matches!(&calls[0], Call::Play { from_s, .. } if *from_s == 600.0));
    assert!(matches!(
        &calls[2],
        Call::SetLoopRegion { region, .. } if *region == LoopRegion::new(0.25, 0.5)
    ));
    audio.release(horn, 0);
    assert!(matches!(rec.calls().last(), Some(Call::Release { .. })));
}

#[test]
fn try_play_clip_reports_a_refusal_instead_of_playing_a_default() {
    let (mut audio, rec, _registry) = pool();
    rec.refuse("file:/music/bad.wav");
    let err = audio
        .try_play_clip(
            "file:/music/bad.wav",
            Arc::from(&b"junk"[..]),
            Bus::MUSIC,
            Start::ONCE,
        )
        .unwrap_err();
    assert!(err.0.contains("refused"), "{err}");
    assert!(rec.calls().is_empty(), "the fallback is on, but not here");
    let ok = audio.try_play_clip(
        "file:/music/good.wav",
        Arc::from(&b"good"[..]),
        Bus::MUSIC,
        Start::ONCE,
    );
    assert!(audio.is_playing(ok.unwrap()));
}

#[test]
fn try_play_file_reads_the_file_and_tells_a_read_failure_from_a_refusal() {
    let (mut audio, rec, _registry) = pool();
    let dir = tempfile::tempdir().unwrap();
    let song = dir.path().join("song.wav");
    std::fs::write(&song, b"riff").unwrap();
    let sound = audio.try_play_file(&song, Bus::MUSIC, Start::ONCE).unwrap();
    assert!(audio.is_playing(sound));
    assert!(matches!(
        rec.calls().last(),
        Some(Call::Play { clip: Some(b), .. }) if **b == b"riff"[..]
    ));
    let missing = dir.path().join("missing.wav");
    assert!(matches!(
        audio.try_play_file(&missing, Bus::MUSIC, Start::ONCE),
        Err(PlayError::Read(_))
    ));
    rec.refuse(&song.to_string_lossy());
    assert!(matches!(
        audio.try_play_file(&song, Bus::MUSIC, Start::ONCE),
        Err(PlayError::Refused(_))
    ));
}

#[test]
fn volume_pitch_and_pan_each_ramp_over_their_own_time() {
    let (mut audio, rec, _registry) = silent_pool();
    let engine = audio.start_with("sfx/door", Bus::EFFECTS, Start::LOOP.volume(0.0));
    audio.set_pitch_over(engine, 1.2, 120);
    let (mix, ramps) = last_ramped(&rec);
    assert_eq!(mix.pitch, 1.2);
    assert_eq!(ramps.pitch_ms, 120);
    audio.set_volume_over(engine, 0.6, 0);
    let (mix, ramps) = last_ramped(&rec);
    assert_eq!((mix.gain, mix.pitch), (0.6, 1.2));
    assert_eq!(ramps.gain_ms, 0, "a volume set at once");
    audio.set_pan_over(engine, 0.7, 0);
    let (mix, ramps) = last_ramped(&rec);
    assert_eq!(mix.pan, 0.7);
    assert_eq!(ramps.pan_ms, 0);
    audio.set_pan(engine, -0.25);
    let (mix, ramps) = last_ramped(&rec);
    assert_eq!(mix.pan, -0.25);
    assert_eq!(ramps.pan_ms, audio.fades.move_ms);
    // An unchanged value sends nothing.
    rec.clear();
    audio.set_volume_over(engine, 0.6, 500);
    assert!(rec.calls().is_empty());
}

#[test]
fn a_positioned_sound_takes_its_pan_from_where_it_is() {
    let (mut audio, rec, _registry) = pool();
    audio.set_listener(Vec3::ZERO, 0.0);
    let east = audio.play_at("sfx/door", Vec3::new(10.0, 0.0, 0.0), Bus::EFFECTS);
    audio.set_pan(east, -1.0);
    let channel = rec.channel_of("sfx/door").unwrap();
    assert!(
        rec.last_mix(channel).unwrap().pan > 0.0,
        "still to the east"
    );
}

#[test]
fn the_length_of_a_clip_comes_from_the_backend() {
    let (mut audio, rec, _registry) = silent_pool();
    rec.set_length("music/show", 912.5);
    let show = audio.play_clip("music/show", Arc::from(&b"s"[..]), Bus::MUSIC, Start::ONCE);
    assert_eq!(audio.length_s(show), Some(912.5));
    let door = audio.play("sfx/door", Bus::EFFECTS);
    assert_eq!(audio.length_s(door), None, "this backend does not know");
    audio.stop(show, 0);
    assert_eq!(audio.length_s(show), None);
}

#[test]
fn a_deferred_open_replays_a_game_clip_loop_with_its_own_bytes() {
    let (backend, rec) = RecordingBackend::new();
    let (go, wait) = mpsc::channel::<()>();
    let registry = Registry::default();
    let mut audio = Audio::deferred(
        move || {
            wait.recv().ok();
            Ok(Box::new(backend) as Box<dyn Backend>)
        },
        Box::new(registry.clone()),
    );
    audio.set_default_fallback(false);
    let bed = audio.play_clip(
        "ambient/bed",
        Arc::from(&b"bed"[..]),
        Bus::EFFECTS,
        Start::LOOP.volume(0.5),
    );
    go.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while audio.opening() && Instant::now() < deadline {
        audio.update(&|_| false);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(audio.is_playing(bed));
    assert!(registry.asked().is_empty());
    match rec.calls().first() {
        Some(Call::Play { key, clip, mix, .. }) => {
            assert_eq!(key, "ambient/bed");
            assert_eq!(clip.as_deref(), Some(&b"bed"[..]));
            assert_eq!(mix.gain, 0.5);
        }
        other => panic!("{other:?}"),
    }
}

/// A bus insert reaches every sound started on that bus, and only those,
/// until it is taken off.
#[test]
fn a_bus_insert_rides_only_the_sounds_started_on_its_bus() {
    let (mut audio, rec, _registry) = pool();
    let radio = Bus("radio");
    let cab = CabSpeakers::new(CabSpeakersParams::default());
    audio.set_bus_insert(radio, Some(Insert::CabSpeakers(cab)));
    assert!(audio.bus_insert(radio).is_some());

    audio.play("radio/static", radio);
    audio.play("sfx/door", Bus::EFFECTS);
    let channel = |key: &str| rec.channel_of(key).expect("played");
    assert!(rec.inserted(channel("radio/static")));
    assert!(!rec.inserted(channel("sfx/door")));

    audio.set_bus_insert(radio, None);
    audio.play("radio/later", radio);
    assert!(!rec.inserted(channel("radio/later")));
}
