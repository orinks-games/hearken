#![cfg(feature = "bass")]
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use bass_sys::safe::channel_get_attribute;
use bass_sys::{BASS_ATTRIB_FREQ, BASS_ATTRIB_PAN, BASS_ATTRIB_VOL};
use hearken_audio::*;

static LOCK: Mutex<()> = Mutex::new(());

fn bass() -> Option<(MutexGuard<'static, ()>, BassBackend)> {
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    match BassBackend::open_device("HEARKEN_AUDIO_TEST", 0) {
        Ok(b) => Some((guard, b)),
        Err(e) => {
            eprintln!("hearken-audio: BASS unavailable here ({e}); skipping");
            None
        }
    }
}

fn sine(seconds: f32) -> Source {
    let n = (seconds * 44_100.0) as usize;
    let samples: Vec<f32> = (0..n).map(|i| (i as f32 * 0.06).sin() * 0.3).collect();
    Source::Clip(hearken_synth::wav_bytes(&samples, 44_100).into())
}

#[test]
fn a_one_shot_plays_and_is_reaped_after_it_ends() {
    let Some((_g, mut b)) = bass() else { return };
    let ch = b
        .play("sfx/test", sine(0.05), false, Mix::UNITY)
        .expect("play");
    assert!(b.is_playing(ch));
    std::thread::sleep(Duration::from_millis(400));
    b.update();
    assert!(!b.is_playing(ch));
    b.shutdown();
}

#[test]
fn a_loop_plays_until_stopped_and_a_fade_ends_it_after_the_fade() {
    let Some((_g, mut b)) = bass() else { return };
    let ch = b
        .play("amb/test", sine(0.05), true, Mix::UNITY)
        .expect("play");
    std::thread::sleep(Duration::from_millis(150));
    assert!(b.is_playing(ch), "a loop outlasts its clip");
    b.stop(ch, 100);
    b.update();
    assert!(b.is_playing(ch), "still fading");
    std::thread::sleep(Duration::from_millis(250));
    b.update();
    assert!(!b.is_playing(ch));
    b.shutdown();
}

#[test]
fn the_mix_becomes_basss_volume_pan_and_frequency() {
    let Some((_g, mut b)) = bass() else { return };
    let mix = Mix {
        gain: 0.5,
        pan: -0.5,
        pitch: 2.0,
        lowpass_hz: None,
        relative: None,
    };
    let ch = b.play("sfx/test", sine(1.0), false, mix).expect("play");
    let h = ch as u32;
    assert!((channel_get_attribute(h, BASS_ATTRIB_VOL).unwrap() - 0.5).abs() < 1e-3);
    assert!((channel_get_attribute(h, BASS_ATTRIB_PAN).unwrap() + 0.5).abs() < 1e-3);
    assert!((channel_get_attribute(h, BASS_ATTRIB_FREQ).unwrap() - 88_200.0).abs() < 1.0);
    b.shutdown();
}

#[test]
fn a_low_pass_comes_and_goes_with_the_mix() {
    let Some((_g, mut b)) = bass() else { return };
    let ch = b
        .play("sfx/test", sine(1.0), true, Mix::UNITY)
        .expect("play");
    let muffled = Mix {
        lowpass_hz: Some(800.0),
        ..Mix::UNITY
    };
    b.set_mix(ch, muffled, 0);
    if !bass_sys::fx::bass_fx_loaded() {
        eprintln!("bass_fx not loaded; the filter half of this test is skipped");
        b.shutdown();
        return;
    }
    assert!(b.has_filter(ch));
    b.set_mix(ch, Mix::UNITY, 0);
    assert!(!b.has_filter(ch));
    b.shutdown();
}

#[test]
fn bytes_that_are_not_sound_are_an_error_not_a_panic() {
    let Some((_g, mut b)) = bass() else { return };
    let err = b.play(
        "sfx/bad",
        Source::Clip((&b"not audio"[..]).into()),
        false,
        Mix::UNITY,
    );
    assert!(err.is_err());
    b.shutdown();
}

#[test]
fn a_live_voice_plays_and_stops() {
    use hearken_synth::fundsp::prelude32::*;
    fn tone(p: &hearken_synth::Params) -> Box<dyn AudioUnit> {
        Box::new(var(&p.get("hz")) >> sine())
    }
    let Some((_g, mut b)) = bass() else { return };
    let voice = hearken_synth::Voice::new(tone, &[("hz", 220.0)]);
    // Fully qualified: `fundsp::prelude32::*` (just imported above, for
    // `AudioUnit`/`var`/`sine`) also brings in its own `net::Source`, which
    // shadows `hearken_audio::Source` for the rest of this function.
    let ch = b
        .play(
            "voice",
            hearken_audio::Source::Voice(voice),
            true,
            Mix::UNITY,
        )
        .expect("play");
    assert!(b.is_playing(ch));
    b.stop(ch, 0);
    assert!(!b.is_playing(ch));
    b.shutdown();
}
