//! Attack round 2: a still source changes pitch when its path changes.
//! Run: cargo test --release -p hearken-audio --test attack_r2_pitch -- --nocapture

#![cfg(feature = "kira")]

use std::sync::{Arc, Mutex};

use hearken_audio::acoustics::{Builtin, SceneBuilder, Shape};
use hearken_audio::{AcousticSettings, Audio, Bus, Feed, FeedRead, KiraBackend, LooseClips, Vec3};

const RATE: u32 = 48_000;

struct Sine {
    hz: f32,
    at: Mutex<usize>,
}

impl Feed for Sine {
    fn read(&self, out: &mut [[f32; 2]]) -> FeedRead {
        let mut at = self.at.lock().unwrap();
        for f in out.iter_mut() {
            let v = (std::f32::consts::TAU * self.hz * *at as f32 / RATE as f32).sin() * 0.5;
            *f = [v, v];
            *at += 1;
        }
        FeedRead {
            frames: out.len(),
            sample_rate: RATE,
        }
    }
}

/// Frequency from rising zero crossings over a window.
fn freq(x: &[f32]) -> Option<f64> {
    let mut crossings = Vec::new();
    for i in 1..x.len() {
        if x[i - 1] < 0.0 && x[i] >= 0.0 {
            // Sub-sample crossing.
            crossings.push(i as f64 - 1.0 + f64::from(-x[i - 1] / (x[i] - x[i - 1])));
        }
    }
    if crossings.len() < 2 {
        return None;
    }
    let span = crossings[crossings.len() - 1] - crossings[0];
    Some((crossings.len() - 1) as f64 / (span / f64::from(RATE)))
}

/// Source and listener never move. Halfway through, the scene gains a
/// wall between them, so the main arrival moves to the path round its end,
/// 1.7 m (5 ms) longer. With Doppler on, that jump is under the 20 ms
/// glide limit, so the delay glides and the tone bends like a moving
/// source. Nothing moved, so the pitch should stay at 1 kHz (a crossfade,
/// as for jumps over 20 ms, would not bend it).
/// Currently fails: the tone dips to about 900 Hz.
#[test]
fn still_source_keeps_its_pitch_when_a_wall_appears() {
    let open = Arc::new(SceneBuilder::new().build());
    let mut b = SceneBuilder::new();
    b.add(
        Shape::wall_between(5.0, -3.0, 5.0, 3.0, 0.0, 3.0),
        Builtin::Concrete.id(),
    )
    .unwrap();
    let walled = Arc::new(b.build());
    let mut audio = Audio::new(
        Box::new(KiraBackend::offline(RATE).unwrap()),
        Box::new(LooseClips::none()),
    );
    audio.set_acoustics(Some(AcousticSettings {
        room: false,
        reflections: false,
        ..AcousticSettings::default()
    }));
    audio.set_listener(Vec3::new(0.0, 0.0, 1.5), 0.0);
    audio.update_acoustic(&open);
    audio.start_feed(
        Arc::new(Sine {
            hz: 1000.0,
            at: Mutex::new(0),
        }),
        Some(Vec3::new(10.0, 0.0, 1.5)),
        Bus::EFFECTS,
    );
    let mut out: Vec<f32> = Vec::new();
    let mut frame = 0;
    while out.len() < RATE as usize {
        let scene = if frame < 30 { &open } else { &walled };
        audio.update_acoustic(scene);
        out.extend(audio.render(800).unwrap().iter().map(|f| f[0]));
        frame += 1;
    }
    let switch = 30 * 800;
    let win = 240;
    let mut worst: f64 = 1000.0;
    let mut at = switch;
    while at + win < out.len() {
        if let Some(f) = freq(&out[at..at + win]) {
            if (f - 1000.0).abs() > (worst - 1000.0).abs() {
                worst = f;
            }
        }
        at += win;
    }
    let before = freq(&out[switch - 4800..switch]).unwrap();
    eprintln!("before {before:.1} Hz, furthest after the wall {worst:.1} Hz");
    assert!((before - 1000.0).abs() < 2.0);
    assert!(
        (worst - 1000.0).abs() < 20.0,
        "still source bent to {worst:.1} Hz"
    );
}
