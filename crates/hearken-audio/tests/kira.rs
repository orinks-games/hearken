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

/// A rig whose channels fade in straight lines and pan as a balance, the
/// way BASS does.
fn bass_like_rig() -> Rig {
    let Rig { b, heard } = rig();
    Rig {
        b: b.with_gain_curve(GainCurve::Linear)
            .with_pan_law(PanLaw::Balance),
        heard,
    }
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

/// 0.1 s loud, 0.1 s quiet, 0.1 s loud: the quiet middle is the loop.
fn three_part() -> Vec<f32> {
    let n = (0.1 * RATE as f32) as usize;
    tone(440.0, 0.3)
        .into_iter()
        .enumerate()
        .map(|(i, s)| if (n..2 * n).contains(&i) { s * 0.1 } else { s })
        .collect()
}

/// `samples` as a WAV whose `smpl` chunk loops `start..=end`.
fn with_smpl(samples: &[f32], start: u32, end: u32) -> Source {
    let mut wav = hearken_synth::wav_bytes(samples, RATE);
    let mut smpl = vec![0u8; 36];
    smpl[28..32].copy_from_slice(&1u32.to_le_bytes());
    for field in [0, 0, start, end, 0, 0] {
        smpl.extend(field.to_le_bytes());
    }
    wav.extend(b"smpl");
    wav.extend((smpl.len() as u32).to_le_bytes());
    wav.extend(smpl);
    let riff = (wav.len() - 8) as u32;
    wav[4..8].copy_from_slice(&riff.to_le_bytes());
    Source::Clip(wav.into())
}

/// Plays the attack, then checks the loop stays in the quiet middle, then
/// releases and checks the loud tail plays and the sound ends by itself.
fn holds_the_middle_then_plays_out(r: &mut Rig, ch: Channel) {
    let tenth = RATE as usize / 10;
    assert!(rms(&r.render(tenth / 2)) > 0.2, "the attack plays");
    r.render(tenth);
    let held = r.render(5 * tenth);
    assert!(
        rms(&held) < 0.1,
        "the loop repeats only the quiet middle: {}",
        rms(&held)
    );
    r.b.release(ch, 100);
    let tail = r.render(2 * tenth);
    assert!(rms(&tail) > 0.2, "the release plays the loud tail");
    r.render(2 * tenth);
    r.b.update();
    assert!(!r.b.is_playing(ch), "a released sound ends by itself");
}

#[test]
fn a_wavs_own_loop_points_hold_its_middle_and_release_plays_its_tail() {
    let mut r = rig();
    let n = RATE / 10;
    let ch =
        r.b.play(
            "sfx/horn",
            with_smpl(&three_part(), n, 2 * n - 1),
            true,
            Mix::UNITY,
        )
        .expect("play");
    holds_the_middle_then_plays_out(&mut r, ch);
}

#[test]
fn a_loop_region_in_seconds_overrides_the_whole_clip() {
    let mut r = rig();
    let ch =
        r.b.play("sfx/horn", clip(&three_part()), true, Mix::UNITY)
            .expect("play");
    r.b.set_loop_region(ch, LoopRegion::new(0.1, 0.2));
    holds_the_middle_then_plays_out(&mut r, ch);
}

#[test]
fn an_opus_files_loop_tags_hold_its_middle() {
    const TAGGED: &[u8] = include_bytes!("fixtures/loop_tagged.opus");
    let mut r = rig();
    let ch =
        r.b.play("sfx/bell", Source::Clip(TAGGED.into()), true, Mix::UNITY)
            .expect("play");
    assert!(rms(&r.render(RATE as usize / 5)) > 0.2, "the attack plays");
    r.render(RATE as usize / 5);
    let held = r.render(RATE as usize);
    assert!(
        rms(&held) < 0.1,
        "the tagged region repeats: {}",
        rms(&held)
    );
    r.b.release(ch, 0);
    r.render(RATE as usize / 2);
    r.render(RATE as usize / 10);
    r.b.update();
    assert!(!r.b.is_playing(ch));
}

#[test]
fn releasing_a_one_shot_or_a_voice_is_harmless_and_a_voice_stops() {
    let mut r = rig();
    let voice =
        r.b.play("voice", sine_voice(220.0), true, Mix::UNITY)
            .expect("play");
    r.b.release(voice, 0);
    r.b.update();
    assert!(!r.b.is_playing(voice), "a voice has no tail: it stops");
    r.b.release(9_999, 0);
}

#[test]
fn a_long_clips_loop_points_hold_while_it_streams() {
    let mut r = rig();
    // 3 s of 16-bit mono is about 265 KB: over the streaming cut.
    let second = RATE as usize;
    let samples: Vec<f32> = tone(440.0, 3.0)
        .into_iter()
        .enumerate()
        .map(|(i, s)| {
            if (second..2 * second).contains(&i) {
                s * 0.1
            } else {
                s
            }
        })
        .collect();
    let ch =
        r.b.play(
            "sfx/long_horn",
            with_smpl(&samples, RATE, 2 * RATE - 1),
            true,
            Mix::UNITY,
        )
        .expect("play");
    // Through the attack and two passes of the loop, letting the decoder
    // thread keep up; the last pass must be the quiet middle.
    let mut rendered = Vec::new();
    while rendered.len() < 3 * second + second / 2 {
        rendered.extend(r.render(1_024));
        std::thread::sleep(Duration::from_millis(1));
    }
    let late = &rendered[rendered.len() - second / 2..];
    assert!(rms(late) < 0.1, "still in the loop region: {}", rms(late));
    assert!(r.b.is_playing(ch));
}

/// How long a stream started `from_s` seconds into `bytes` takes to make
/// sound, rendering as fast as the mock allows.
fn time_to_sound(bytes: &[u8], from_s: f64) -> Duration {
    let mut r = rig();
    let started = std::time::Instant::now();
    r.b.play_from(
        "music/long",
        Source::Clip(bytes.into()),
        false,
        Mix::UNITY,
        from_s,
    )
    .expect("play");
    while rms(&r.render(1_024)) < 0.05 {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "no sound a minute after a seek to {from_s} s"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    started.elapsed()
}

/// The H5 measurement: a stream started 10 minutes into a 15-minute Opus
/// programme, the way a station rotation tunes in mid-show. Needs a long
/// file, which the repository does not carry; make one with
///   ffmpeg -f lavfi -i sine=frequency=440:duration=900:sample_rate=48000 -ac 2 -c:a libopus -b:a 96k prog15.opus
/// and run with `HEARKEN_LONG_OPUS=prog15.opus cargo test -p hearken-audio
/// --test kira -- --ignored --nocapture`.
#[test]
#[ignore = "needs HEARKEN_LONG_OPUS, a 15-minute Opus file"]
fn a_ten_minute_seek_into_a_long_opus_is_measured() {
    let path = std::env::var("HEARKEN_LONG_OPUS").expect("HEARKEN_LONG_OPUS");
    let bytes = std::fs::read(path).expect("read HEARKEN_LONG_OPUS");
    let top = time_to_sound(&bytes, 0.0);
    let ten = time_to_sound(&bytes, 600.0);
    eprintln!("time to sound: from the top {top:?}, 10 minutes in {ten:?}");
    assert!(ten < Duration::from_millis(500), "{ten:?}");
}

/// `seconds` of mono silence and then `seconds` of a constant 0.5, as a WAV.
fn silence_then_dc(seconds: f32) -> Vec<f32> {
    let n = (seconds * RATE as f32) as usize;
    let mut samples = vec![0.0; n];
    samples.extend(std::iter::repeat_n(0.5, n));
    samples
}

#[test]
fn a_balance_pan_keeps_the_centre_at_unity_and_only_lowers_the_far_ear() {
    let mut r = bass_like_rig();
    let ch =
        r.b.play("sfx/dc", clip(&[0.5; 44_100]), false, Mix::UNITY)
            .expect("play");
    let f = r.render(1_000)[500];
    assert!(
        (f.left - 0.5).abs() < 1e-6 && (f.right - 0.5).abs() < 1e-6,
        "a centred mono clip is at unity in both ears: {f:?}"
    );
    // BASS's balance law: left = min(1, 1 - pan), right = min(1, 1 + pan).
    for (pan, left, right) in [
        (1.0, 0.0, 1.0),
        (-1.0, 1.0, 0.0),
        (0.5, 0.5, 1.0),
        (-0.25, 1.0, 0.75),
    ] {
        r.b.set_mix(ch, mix(1.0, pan, 1.0, None), 0);
        let f = r.render(1_000)[500];
        assert!(
            (f.left - 0.5 * left).abs() < 1e-5 && (f.right - 0.5 * right).abs() < 1e-5,
            "pan {pan}: {f:?}"
        );
    }
}

#[test]
fn a_pan_given_at_the_start_is_there_from_the_first_frame() {
    let mut r = bass_like_rig();
    r.b.play(
        "sfx/dc",
        clip(&[0.5; 4_410]),
        false,
        mix(1.0, -1.0, 1.0, None),
    )
    .expect("play");
    let frames = r.render(512);
    assert!(
        frames[..256].iter().all(|f| f.right == 0.0),
        "no swing in from the centre"
    );
}

#[test]
fn a_linear_fade_out_is_at_half_volume_half_way() {
    for (curve, expect) in [(GainCurve::Linear, 0.25), (GainCurve::Decibels, 0.0158)] {
        let Rig { b, heard } = rig();
        let mut r = Rig {
            b: b.with_gain_curve(curve),
            heard,
        };
        let ch =
            r.b.play("sfx/dc", clip(&[0.5; 88_200]), false, Mix::UNITY)
                .expect("play");
        r.render(512);
        r.b.stop(ch, 1_000);
        let faded = r.render(RATE as usize / 2);
        let half_way = faded[RATE as usize / 2 - 1].left;
        // Linear: 0.5 x 0.5. Kira's decibels: -30 dB of 0.5 half way down
        // a 0 -> -60 dB tween.
        assert!(
            (half_way - expect).abs() < 0.01,
            "{curve:?}: {half_way} half way through the fade"
        );
    }
}

#[test]
fn a_linear_volume_ramp_moves_in_a_straight_line() {
    let mut r = bass_like_rig();
    let ch =
        r.b.play(
            "sfx/dc",
            clip(&[0.5; 88_200]),
            true,
            mix(0.0, 0.0, 1.0, None),
        )
        .expect("play");
    r.render(512);
    r.b.set_mix_ramped(
        ch,
        mix(1.0, 0.0, 1.0, None),
        Ramps {
            gain_ms: 1_000,
            ..Ramps::default()
        },
    );
    let ramp = r.render(RATE as usize);
    for (at, expect) in [(0.25, 0.125), (0.5, 0.25), (0.75, 0.375)] {
        let f = ramp[(at * RATE as f32) as usize];
        assert!((f.left - expect).abs() < 0.005, "{at}: {f:?}");
    }
    r.b.set_mix_ramped(ch, mix(0.2, 0.0, 1.0, None), Ramps::default());
    let f = r.render(256)[200];
    assert!((f.left - 0.1).abs() < 1e-5, "a zero ramp is at once: {f:?}");
}

#[test]
fn a_ramped_change_to_the_volume_leaves_a_pitch_slide_under_way() {
    let mut r = rig();
    let ch =
        r.b.play("sfx/tone", clip(&tone(441.0, 4.0)), true, Mix::UNITY)
            .expect("play");
    r.render(4_410);
    r.b.set_mix_ramped(
        ch,
        mix(1.0, 0.0, 2.0, None),
        Ramps {
            pitch_ms: 2_000,
            ..Ramps::default()
        },
    );
    // A volume change at once, mid slide: it must not snap the pitch.
    r.render(RATE as usize / 2);
    r.b.set_mix_ramped(ch, mix(0.8, 0.0, 2.0, None), Ramps::default());
    let frames = r.render(4_410);
    let n = crossings(&frames[..4_410]);
    // A quarter of the way into a 1 -> 2 slide is about 1.25x 441 Hz, ~110
    // crossings in 0.1 s; snapped to 2x it would be ~176.
    assert!((100..=135).contains(&n), "{n} crossings");
}

#[test]
fn a_clip_starts_from_an_offset_decoded_whole_or_streamed() {
    // 1 s of silence then 1 s of DC decodes whole; 6 s and 6 s streams.
    for seconds in [1.0, 6.0] {
        let mut r = rig();
        let samples = silence_then_dc(seconds);
        let bytes = hearken_synth::wav_bytes(&samples, RATE);
        let streams = bytes.len() > 256 * 1024;
        assert_eq!(streams, seconds > 3.0);
        let ch =
            r.b.play_from(
                "music/c3k",
                Source::Clip(bytes.into()),
                false,
                Mix::UNITY,
                f64::from(seconds) + 0.5,
            )
            .expect("play");
        let mut heard = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while heard.len() < 2_048 && std::time::Instant::now() < deadline {
            heard.extend(r.render(1_024));
            std::thread::sleep(Duration::from_millis(2));
        }
        // A stream may hand over a few frames of its prefill first.
        let sounding = heard.iter().skip(1_024).take(512);
        assert!(
            sounding.clone().all(|f| (f.left - 0.5).abs() < 1e-3),
            "{seconds} s clip, started in its second half, is sounding"
        );
        assert_eq!(
            r.b.length_s(ch).map(|s| s.round()),
            Some(f64::from(seconds) * 2.0)
        );
    }
}

#[test]
fn a_live_voice_has_no_length() {
    let mut r = rig();
    let ch =
        r.b.play("voice", sine_voice(441.0), true, Mix::UNITY)
            .expect("play");
    assert_eq!(r.b.length_s(ch), None);
}

/// A 0.2 s, 440 Hz stereo tone in each container a player's own music might
/// come in, made with:
///   ffmpeg -f lavfi -i sine=frequency=440:duration=0.2 -af volume=12dB -ac 2 -c:a aac -b:a 64k tone.m4a
///   ffmpeg -f lavfi -i sine=frequency=440:duration=0.2 -af volume=12dB -ac 2 tone.aiff
#[test]
fn aac_in_mp4_and_aiff_decode() {
    for (name, bytes) in [
        ("m4a", &include_bytes!("fixtures/tone.m4a")[..]),
        ("aiff", &include_bytes!("fixtures/tone.aiff")[..]),
    ] {
        let mut r = rig();
        r.b.play("music/file", Source::Clip(bytes.into()), false, Mix::UNITY)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let frames = r.render(RATE as usize / 10);
        assert!(rms(&frames[2_000..]) > 0.1, "{name} makes sound");
    }
}

#[test]
fn a_long_opus_started_part_way_plays_only_what_is_left() {
    let mut r = rig();
    let ch =
        r.b.play_from(
            "music/opus",
            Source::Clip(OPUS_LONG.into()),
            false,
            Mix::UNITY,
            3.0,
        )
        .expect("play");
    let mut rendered = 0;
    while rendered < RATE as usize {
        rendered += r.render(1_024).len();
        std::thread::sleep(Duration::from_millis(1));
    }
    r.b.update();
    assert!(r.b.is_playing(ch), "1 s in, 0.5 s of the clip is left");
    while rendered < 2 * RATE as usize {
        rendered += r.render(1_024).len();
        std::thread::sleep(Duration::from_millis(1));
    }
    r.b.update();
    assert!(
        !r.b.is_playing(ch),
        "3 s + 2 s is past the 4.5 s clip's end"
    );
}
