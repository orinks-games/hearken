//! Speaks through the real model. Point `HEARKEN_VOICES` at a folder holding
//! `kokoro-en-q8` (`cargo run -p hearken-voice --example install --features
//! download -- <folder>`); without it the test is skipped, unless
//! `HEARKEN_VOICE_REQUIRED=1` (as CI sets) makes that a failure.
#![cfg(feature = "voice")]

use hearken_voice::{find_voices, package_dir, Kokoro};
use std::sync::Arc;
use std::time::Instant;

/// Commentary of the kind Fastbreak's broadcast crew speaks.
const LINES: [(&str, &str); 10] = [
    (
        "bf_emma",
        "Tip-off at the Garden, and the Harbor Hawks have it first.",
    ),
    (
        "am_michael",
        "That's a deep three from the corner. Count it!",
    ),
    ("bf_emma", "Hawks lead 54-49 at the half."),
    (
        "am_michael",
        "He got a hand in his face and it still went down.",
    ),
    ("bf_emma", "Timeout on the floor, 2:14 left in the fourth."),
    (
        "am_michael",
        "Off the rim. Long rebound, and they're running.",
    ),
    ("bf_emma", "That's three straight buckets for the visitors."),
    ("am_michael", "You cannot leave a shooter that open, Dana."),
    (
        "bf_emma",
        "Final score, 112-108. The Hawks move to 9 and 5.",
    ),
    ("am_michael", "What a night. We'll see you next time."),
];

fn model() -> Option<Kokoro> {
    let required = std::env::var("HEARKEN_VOICE_REQUIRED").as_deref() == Ok("1");
    let Some(dir) = find_voices("HEARKEN").and_then(|v| package_dir(&v)) else {
        assert!(
            !required,
            "HEARKEN_VOICE_REQUIRED=1 but no voices/kokoro-en-q8 was found"
        );
        eprintln!("skipped: no voices/kokoro-en-q8 (set HEARKEN_VOICES)");
        return None;
    };
    let start = Instant::now();
    let model = Kokoro::load(&dir).expect("the model loads");
    eprintln!("load: {:.0} ms", start.elapsed().as_secs_f64() * 1000.0);
    Some(model)
}

#[test]
fn a_line_becomes_wav_that_hearken_audio_decodes() {
    let Some(mut model) = model() else { return };
    let speakers = model.speakers();
    assert!(speakers.iter().any(|s| s == "bf_emma"), "{speakers:?}");
    assert!(speakers.iter().any(|s| s == "am_michael"), "{speakers:?}");

    let speech = model
        .synthesize("Hawks lead 54-49 at the half.", "bf_emma", 1.0)
        .expect("synthesis");
    assert!(speech.duration_s() > 0.5, "{} s", speech.duration_s());
    assert!(speech.samples.iter().any(|s| s.abs() > 0.01), "silent");

    let info = hearken_audio::probe_clip(Arc::from(speech.to_wav())).expect("decodes");
    assert_eq!(info.sample_rate, speech.sample_rate);
    let length = info.length_s.expect("WAV knows its length");
    assert!((length - speech.duration_s()).abs() < 0.01);
}

/// Prints the synthesis time of ten commentary lines, for the spike note.
#[test]
fn ten_lines_latency() {
    let Some(mut model) = model() else { return };
    // The first run warms ONNX Runtime; it is reported but not counted.
    let warm = Instant::now();
    model
        .synthesize("Warm up.", "bf_emma", 1.0)
        .expect("warm-up");
    eprintln!("warm-up: {:.0} ms", warm.elapsed().as_secs_f64() * 1000.0);

    let mut total_ms = 0.0;
    let mut audio_s = 0.0;
    let mut worst_ms: f64 = 0.0;
    for (speaker, text) in LINES {
        let start = Instant::now();
        let speech = model.synthesize(text, speaker, 1.0).expect(text);
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        total_ms += ms;
        audio_s += speech.duration_s();
        worst_ms = worst_ms.max(ms);
        eprintln!(
            "{ms:7.0} ms  {:5.2} s audio  {speaker}: {text}",
            speech.duration_s()
        );
    }
    eprintln!(
        "VOICE-LATENCY mean {:.0} ms, worst {:.0} ms, real-time factor {:.3} ({:.1} s of audio)",
        total_ms / LINES.len() as f64,
        worst_ms,
        total_ms / 1000.0 / audio_s,
        audio_s
    );
}
