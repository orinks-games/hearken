#![cfg(feature = "kira")]
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hearken_audio::*;
use kira::backend::mock::{MockBackend, MockBackendSettings};
use kira::effect::Effect;
use kira::info::Info;
use kira::track::MainTrackBuilder;
use kira::{AudioManagerSettings, Capacities, Frame};

const RATE: u32 = 44_100;

/// Copies everything reaching the main track, so a test hears what the
/// backend made.
struct Tap(Arc<Mutex<Vec<Frame>>>);

impl Effect for Tap {
    fn process(&mut self, input: &mut [Frame], _dt: f64, _info: &Info) {
        self.0.lock().unwrap().extend_from_slice(input);
    }
}

struct Rig {
    b: KiraBackend<MockBackend>,
    heard: Arc<Mutex<Vec<Frame>>>,
}

fn rig_with(capacities: Capacities) -> Rig {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let settings = AudioManagerSettings {
        capacities,
        main_track_builder: MainTrackBuilder::new().with_built_effect(Box::new(Tap(heard.clone()))),
        backend_settings: MockBackendSettings { sample_rate: RATE },
        ..AudioManagerSettings::default()
    };
    let b = KiraBackend::with_settings(settings).expect("the mock output always opens");
    Rig { b, heard }
}

fn rig() -> Rig {
    rig_with(Capacities::default())
}

impl Rig {
    /// Renders at least `frames` frames of output and returns them.
    fn render(&mut self, frames: usize) -> Vec<Frame> {
        self.heard.lock().unwrap().clear();
        let mock = self.b.manager_mut().expect("open").backend_mut();
        while self.heard.lock().unwrap().len() < frames {
            mock.on_start_processing();
            mock.process();
        }
        std::mem::take(&mut *self.heard.lock().unwrap())
    }

    fn sub_tracks(&mut self) -> usize {
        self.b.manager_mut().expect("open").num_sub_tracks()
    }
}

fn clip(samples: &[f32]) -> Source {
    Source::Clip(hearken_synth::wav_bytes(samples, RATE).into())
}

fn tone(hz: f32, seconds: f32) -> Vec<f32> {
    let n = (seconds * RATE as f32) as usize;
    (0..n)
        .map(|i| (i as f32 * hz * TAU / RATE as f32).sin() * 0.5)
        .collect()
}

fn rms(frames: &[Frame]) -> f32 {
    (frames.iter().map(|f| f.left * f.left).sum::<f32>() / frames.len() as f32).sqrt()
}

fn mix(gain: f32, pan: f32, pitch: f32, lowpass_hz: Option<f32>) -> Mix {
    Mix {
        gain,
        pan,
        pitch,
        lowpass_hz,
        relative: None,
    }
}

#[test]
fn a_one_shot_plays_and_is_reaped_after_it_ends() {
    let mut r = rig();
    let ch =
        r.b.play("sfx/test", clip(&tone(440.0, 0.05)), false, Mix::UNITY)
            .expect("play");
    assert!(r.b.is_playing(ch));
    assert!(rms(&r.render(1_000)) > 0.1, "it makes sound");
    r.render(4_410);
    r.b.update();
    assert!(!r.b.is_playing(ch));
    r.render(128);
    assert_eq!(r.sub_tracks(), 0, "its track was removed");
}

#[test]
fn a_loop_outlasts_its_clip_until_stopped_and_a_fade_ends_it_after_the_fade() {
    let mut r = rig();
    let ch =
        r.b.play("amb/test", clip(&tone(440.0, 0.05)), true, Mix::UNITY)
            .expect("play");
    r.render(8_820);
    r.b.update();
    assert!(r.b.is_playing(ch), "a loop outlasts its clip");
    r.b.stop(ch, 100);
    r.b.update();
    assert!(r.b.is_playing(ch), "still fading");
    let faded = r.render(8_820);
    assert!(
        rms(&faded[faded.len() - 1_000..]) < 1e-4,
        "the fade reached silence"
    );
    std::thread::sleep(Duration::from_millis(150));
    r.b.update();
    assert!(!r.b.is_playing(ch));
}

#[test]
fn gain_and_pan_reach_the_output() {
    let mut r = rig();
    let ch =
        r.b.play(
            "sfx/dc",
            clip(&[0.5; 44_100]),
            false,
            mix(0.5, 0.0, 1.0, None),
        )
        .expect("play");
    let f = r.render(1_000)[500];
    assert!(
        (f.left - 0.25).abs() < 1e-3 && (f.right - 0.25).abs() < 1e-3,
        "{f:?}"
    );
    r.b.set_mix(ch, mix(0.5, 1.0, 1.0, None), 0);
    let f = r.render(1_000)[500];
    assert!(
        f.left.abs() < 1e-3,
        "hard right leaves the left ear silent: {f:?}"
    );
    assert!(
        (f.right - 0.25 * std::f32::consts::SQRT_2).abs() < 1e-3,
        "{f:?}"
    );
}

#[test]
fn a_gain_of_zero_is_silence() {
    let mut r = rig();
    r.b.play(
        "sfx/dc",
        clip(&[0.5; 4_410]),
        false,
        mix(0.0, 0.0, 1.0, None),
    )
    .expect("play");
    assert!(r
        .render(2_000)
        .iter()
        .all(|f| f.left == 0.0 && f.right == 0.0));
}

#[test]
fn pitch_two_plays_a_clip_in_half_the_time() {
    let mut r = rig();
    let fast =
        r.b.play(
            "sfx/a",
            clip(&tone(440.0, 0.1)),
            false,
            mix(1.0, 0.0, 2.0, None),
        )
        .expect("play");
    let slow =
        r.b.play("sfx/b", clip(&tone(440.0, 0.1)), false, Mix::UNITY)
            .expect("play");
    r.render(3_000);
    r.b.update();
    assert!(
        !r.b.is_playing(fast),
        "0.1 s at double speed is over by 0.068 s"
    );
    assert!(r.b.is_playing(slow));
}

#[test]
fn a_low_pass_muffles_high_frequencies_and_lifts_cleanly() {
    let mut r = rig();
    let ch =
        r.b.play("amb/hiss", clip(&tone(8_000.0, 0.5)), true, Mix::UNITY)
            .expect("play");
    let open = rms(&r.render(4_410)[500..]);
    r.b.set_mix(ch, mix(1.0, 0.0, 1.0, Some(800.0)), 0);
    let muffled = rms(&r.render(4_410)[500..]);
    r.b.set_mix(ch, Mix::UNITY, 0);
    let lifted = rms(&r.render(4_410)[500..]);
    assert!(muffled < open * 0.1, "open {open}, muffled {muffled}");
    assert!(lifted > open * 0.9, "open {open}, lifted {lifted}");
}

#[test]
fn pause_silences_a_sound_without_ending_it() {
    let mut r = rig();
    let ch =
        r.b.play("amb/test", clip(&tone(440.0, 0.5)), true, Mix::UNITY)
            .expect("play");
    r.render(1_000);
    r.b.pause(ch, true);
    let paused = r.render(4_410);
    assert!(rms(&paused[1_000..]) < 1e-4);
    r.b.update();
    assert!(r.b.is_playing(ch), "a paused sound is still alive");
    r.b.pause(ch, false);
    assert!(
        rms(&r.render(4_410)[1_000..]) > 0.1,
        "resume brings it back"
    );
}

#[test]
fn a_long_clip_streams_and_is_heard() {
    let mut r = rig();
    // 5 s of 16-bit mono is about 441 KB: over the streaming cut.
    let ch =
        r.b.play("music/long", clip(&tone(440.0, 5.0)), false, Mix::UNITY)
            .expect("play");
    assert!(r.b.is_playing(ch));
    let mut heard = false;
    for _ in 0..200 {
        if rms(&r.render(1_024)) > 0.1 {
            heard = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(heard, "the decoder thread fed the stream");
}

#[test]
fn bytes_that_are_not_sound_are_an_error_not_a_panic() {
    let mut r = rig();
    let err = r.b.play(
        "sfx/bad",
        Source::Clip((&b"not audio"[..]).into()),
        false,
        Mix::UNITY,
    );
    assert!(err.is_err());
    // Two audio cycles: Kira adds the track in one and removes it in the next.
    r.render(256);
    assert_eq!(r.sub_tracks(), 0, "the failed sound's track did not leak");
}

#[test]
fn more_sounds_than_capacity_is_an_error_not_a_panic() {
    let mut r = rig_with(Capacities {
        sub_track_capacity: 2,
        ..Capacities::default()
    });
    for _ in 0..2 {
        r.b.play("sfx/test", clip(&tone(440.0, 0.1)), false, Mix::UNITY)
            .expect("play");
    }
    assert!(r
        .b
        .play("sfx/test", clip(&tone(440.0, 0.1)), false, Mix::UNITY)
        .is_err());
}

#[test]
fn shutdown_is_idempotent_and_later_calls_are_harmless() {
    let mut r = rig();
    let ch =
        r.b.play("amb/test", clip(&tone(440.0, 0.1)), true, Mix::UNITY)
            .expect("play");
    r.b.shutdown();
    r.b.shutdown();
    assert!(r.b.manager_mut().is_none());
    assert!(!r.b.is_playing(ch));
    r.b.set_mix(ch, Mix::UNITY, 0);
    r.b.pause(ch, true);
    r.b.stop(ch, 100);
    r.b.update();
    assert!(r
        .b
        .play("sfx/test", clip(&tone(440.0, 0.1)), false, Mix::UNITY)
        .is_err());
}

#[test]
fn the_default_device_opens_and_plays_or_the_test_says_why_not() {
    let mut b = match KiraBackend::open() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("hearken-audio: no sound device here ({e}); skipping");
            return;
        }
    };
    // Silent, so a test run makes no noise; this checks the device path.
    let ch = b
        .play(
            "sfx/test",
            clip(&tone(440.0, 0.05)),
            false,
            mix(0.0, 0.0, 1.0, None),
        )
        .expect("play");
    assert!(b.is_playing(ch));
    b.shutdown();
}

fn sine_voice(hz: f32) -> hearken_audio::Source {
    use hearken_synth::fundsp::prelude32::*;
    fn tone(p: &hearken_synth::Params) -> Box<dyn AudioUnit> {
        Box::new(var(&p.get("hz")) >> sine())
    }
    // Fully qualified: fundsp's prelude brings its own `Source`.
    hearken_audio::Source::Voice(hearken_synth::Voice::new(tone, &[("hz", hz)]))
}

fn crossings(frames: &[Frame]) -> usize {
    frames
        .windows(2)
        .filter(|w| (w[0].left < 0.0) != (w[1].left < 0.0))
        .count()
}

#[test]
fn a_live_voice_sounds_follows_its_pitch_and_stops() {
    let mut r = rig();
    let ch =
        r.b.play("voice", sine_voice(441.0), true, Mix::UNITY)
            .expect("play");
    let at_one = crossings(&r.render(4_410)[..4_410]);
    assert!(
        (80..=96).contains(&at_one),
        "441 Hz for 0.1 s crosses zero ~88 times: {at_one}"
    );
    r.b.set_mix(ch, mix(1.0, 0.0, 2.0, None), 0);
    r.render(512);
    let at_two = crossings(&r.render(4_410)[..4_410]);
    assert!(
        (168..=184).contains(&at_two),
        "pitch 2 doubles it: {at_two}"
    );
    r.b.stop(ch, 0);
    assert!(!r.b.is_playing(ch));
    assert!(rms(&r.render(1_000)[500..]) < 1e-4);
}

#[test]
fn a_nonsense_pitch_on_a_voice_still_renders_sound() {
    let mut r = rig();
    let ch =
        r.b.play("voice", sine_voice(441.0), true, Mix::UNITY)
            .expect("play");
    r.b.set_mix(ch, mix(1.0, 0.0, f32::NAN, None), 0);
    let frames = r.render(4_410);
    assert!(frames.iter().all(|f| f.left.is_finite()));
    assert!(rms(&frames[1_000..]) > 0.1);
}

// The Opus fixtures are 440 Hz tones at half scale (ffmpeg's sine is 1/8
// scale; +12 dB), made with:
//   ffmpeg -f lavfi -i sine=frequency=440:duration=1 -af volume=12dB -ac 1 -c:a libopus -b:a 48k tone.opus
//   ffmpeg -f lavfi -i sine=frequency=440:duration=4.5 -af volume=12dB -ac 2 -c:a libopus -b:a 512k -vbr off tone_long.opus
const OPUS: &[u8] = include_bytes!("fixtures/tone.opus");
/// 4.5 s at a constant 512 kbps: over the streaming cut.
const OPUS_LONG: &[u8] = include_bytes!("fixtures/tone_long.opus");

#[test]
fn an_opus_clip_is_heard_at_its_own_pitch() {
    let mut r = rig();
    r.b.play("sfx/opus", Source::Clip(OPUS.into()), false, Mix::UNITY)
        .expect("play");
    let frames = r.render(RATE as usize / 2);
    assert!(rms(&frames[4_410..]) > 0.2, "it makes sound");
    // 48 kHz Opus played at the output's 44.1 kHz: still 440 Hz.
    let n = crossings(&frames[4_410..8_820]);
    assert!(
        (84..=92).contains(&n),
        "440 Hz for 0.1 s crosses ~88 times: {n}"
    );
}

#[test]
fn a_long_opus_clip_streams_and_loops() {
    let mut r = rig();
    assert!(OPUS_LONG.len() > 256 * 1024);
    let ch =
        r.b.play(
            "music/opus",
            Source::Clip(OPUS_LONG.into()),
            true,
            Mix::UNITY,
        )
        .expect("play");
    // Past the clip's 4.5 s end, giving the decoder thread time to keep up.
    let mut rendered = 0;
    while rendered < 5 * RATE as usize {
        rendered += r.render(1_024).len();
        std::thread::sleep(Duration::from_millis(1));
    }
    r.b.update();
    assert!(r.b.is_playing(ch), "a loop outlasts its clip");
    let mut heard = false;
    for _ in 0..200 {
        if rms(&r.render(1_024)) > 0.2 {
            heard = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(heard, "the stream looped back to sound");
}

#[test]
fn a_truncated_opus_clip_is_an_error_not_a_panic() {
    let mut r = rig();
    // Cut inside the Opus header page.
    let err = r.b.play(
        "sfx/bad",
        Source::Clip(OPUS[..40].into()),
        false,
        Mix::UNITY,
    );
    assert!(err.is_err());
    r.render(256);
    assert_eq!(r.sub_tracks(), 0, "the failed sound's track did not leak");
}

/// A 441 Hz sine at an adjustable sample rate, there whenever it is pulled
/// unless `dry` is set, counting the frames it hands over.
struct TestFeed {
    rate: AtomicU32,
    dry: AtomicBool,
    phase: Mutex<f64>,
    pulled: AtomicUsize,
}

impl TestFeed {
    fn new(rate: u32) -> Arc<TestFeed> {
        Arc::new(TestFeed {
            rate: AtomicU32::new(rate),
            dry: AtomicBool::new(false),
            phase: Mutex::new(0.0),
            pulled: AtomicUsize::new(0),
        })
    }
}

impl Feed for TestFeed {
    fn read(&self, out: &mut [[f32; 2]]) -> FeedRead {
        if self.dry.load(Ordering::Relaxed) {
            return FeedRead {
                frames: 0,
                sample_rate: 0,
            };
        }
        let rate = self.rate.load(Ordering::Relaxed);
        let mut phase = self.phase.lock().unwrap();
        for frame in out.iter_mut() {
            let sample = (*phase * std::f64::consts::TAU).sin() as f32 * 0.5;
            *frame = [sample; 2];
            // A rate of 0 is the feed's mistake under test, not this sine's.
            *phase = (*phase + 441.0 / f64::from(rate.max(1))).fract();
        }
        self.pulled.fetch_add(out.len(), Ordering::Relaxed);
        FeedRead {
            frames: out.len(),
            sample_rate: rate,
        }
    }
}

#[test]
fn a_feed_is_heard_at_its_own_pitch_across_a_rate_change() {
    let mut r = rig();
    let feed = TestFeed::new(22_050);
    let ch =
        r.b.play("feed", Source::Feed(feed.clone()), true, Mix::UNITY)
            .expect("play");
    let before = crossings(&r.render(4_410)[..4_410]);
    assert!(
        (80..=96).contains(&before),
        "441 Hz for 0.1 s crosses zero ~88 times: {before}"
    );
    feed.rate.store(48_000, Ordering::Relaxed);
    r.render(1_024);
    let after = crossings(&r.render(4_410)[..4_410]);
    assert!(
        (80..=96).contains(&after),
        "the new rate leaves the pitch alone: {after}"
    );
    assert!(r.b.is_playing(ch));
}

#[test]
fn a_feed_that_starts_at_rate_zero_is_heard_once_it_has_a_rate() {
    let mut r = rig();
    let feed = TestFeed::new(0);
    r.b.play("feed", Source::Feed(feed.clone()), true, Mix::UNITY)
        .expect("play");
    assert!(rms(&r.render(2_000)[500..]) < 1e-4, "no rate is silence");
    feed.rate.store(44_100, Ordering::Relaxed);
    assert!(
        rms(&r.render(2_000)[500..]) > 0.1,
        "it sounds once the rate is real"
    );
}

#[test]
fn a_feed_at_an_absurd_rate_is_pulled_no_faster_than_a_real_one() {
    let mut r = rig();
    let feed = TestFeed::new(u32::MAX);
    r.b.play("feed", Source::Feed(feed.clone()), true, Mix::UNITY)
        .expect("play");
    let rendered = r.render(512).len();
    let pulled = feed.pulled.load(Ordering::Relaxed);
    // The most a 768 kHz feed could be asked for, and the block in hand.
    let most = rendered * 768_000 / RATE as usize + 1_024;
    assert!(pulled <= most, "{pulled} frames pulled for {rendered}");
}

#[test]
fn a_feed_that_runs_dry_is_silence_not_a_stop() {
    let mut r = rig();
    let feed = TestFeed::new(44_100);
    feed.dry.store(true, Ordering::Relaxed);
    let ch =
        r.b.play("feed", Source::Feed(feed.clone()), true, Mix::UNITY)
            .expect("play");
    assert!(rms(&r.render(2_000)[500..]) < 1e-4, "dry is silence");
    r.b.update();
    assert!(r.b.is_playing(ch), "and the feed is still alive");
    feed.dry.store(false, Ordering::Relaxed);
    assert!(
        rms(&r.render(2_000)[500..]) > 0.1,
        "it sounds again once there is audio"
    );
}

#[test]
fn pause_silences_a_feed() {
    let mut r = rig();
    let ch =
        r.b.play(
            "feed",
            Source::Feed(TestFeed::new(44_100)),
            true,
            Mix::UNITY,
        )
        .expect("play");
    assert!(rms(&r.render(2_000)[500..]) > 0.1, "it makes sound");
    r.b.pause(ch, true);
    assert!(rms(&r.render(2_000)[500..]) < 1e-4);
}
