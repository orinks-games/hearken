//! Attack round 1 on the rendering side, through Kira's offline mixer.
//! Run: cargo test --release -p hearken-audio --test attack_r1 -- --nocapture
//! Tests marked FAILS fail on the build they were written against.

#![cfg(feature = "kira")]

use std::sync::{Arc, Mutex};

use hearken_audio::acoustics::{Bands, Material, SceneBuilder, Shape};
use hearken_audio::{AcousticSettings, Audio, Bus, Feed, FeedRead, KiraBackend, LooseClips, Vec3};

const RATE: u32 = 48_000;

struct Signal {
    samples: Vec<f32>,
    at: Mutex<usize>,
}

impl Feed for Signal {
    fn read(&self, out: &mut [[f32; 2]]) -> FeedRead {
        let mut at = self.at.lock().unwrap();
        for f in out.iter_mut() {
            let v = self.samples.get(*at).copied().unwrap_or(0.0);
            *f = [v, v];
            *at += 1;
        }
        FeedRead {
            frames: out.len(),
            sample_rate: RATE,
        }
    }
}

fn sine(hz: f32, seconds: f32) -> Arc<Signal> {
    let samples = (0..(seconds * RATE as f32) as usize)
        .map(|n| (std::f32::consts::TAU * hz * n as f32 / RATE as f32).sin() * 0.5)
        .collect();
    Arc::new(Signal {
        samples,
        at: Mutex::new(0),
    })
}

fn bare() -> AcousticSettings {
    AcousticSettings {
        room: false,
        reflections: false,
        ..AcousticSettings::default()
    }
}

fn rms(x: &[[f32; 2]]) -> f64 {
    (x.iter().map(|f| f64::from(f[0]).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

/// Renders a 2 kHz tone through two wide walls of `tl` transmission loss.
fn through_two_walls(tl: [f64; 7]) -> (f64, f32) {
    let mut b = SceneBuilder::new();
    let id = b
        .add_material(&Material {
            name: "panel".into(),
            absorption: Bands::splat(0.1),
            scattering: Bands::splat(0.05),
            transmission_loss_db: Bands(tl),
        })
        .unwrap();
    for y in [2.0, 4.0] {
        b.add(
            Shape::wall_between(-5000.0, y, 5000.0, y, -100.0, 500.0),
            id,
        )
        .unwrap();
    }
    let scene = Arc::new(b.build());
    let mut audio = Audio::new(
        Box::new(KiraBackend::offline(RATE).unwrap()),
        Box::new(LooseClips::none()),
    );
    audio.set_acoustics(Some(bare()));
    audio.set_listener(Vec3::new(0.0, 0.0, 1.5), 0.0);
    audio.update_acoustic(&scene);
    let src = Vec3::new(0.0, 6.0, 1.5);
    audio.start_feed(sine(2000.0, 1.0), Some(src), Bus::EFFECTS);
    let mut out = Vec::new();
    while out.len() < RATE as usize / 2 {
        audio.update_acoustic(&scene);
        out.extend(audio.render(800).unwrap());
    }
    let g = hearken_audio::acoustics::propagate(
        &scene,
        hearken_audio::acoustics::Vec3::new(0.0, 6.0, 1.5),
        hearken_audio::acoustics::Vec3::new(0.0, 0.0, 1.5),
        &Default::default(),
    )
    .render(0.0, None)
    .gains;
    (rms(&out[RATE as usize / 4..]), g[4])
}

/// The direct path's level is anchored on its lowest band: the shelf
/// cascade only holds 80 dB per step and floors bands at 1e-6, then the
/// whole path is scaled by band 0. A material that blocks only the lowest
/// band (two crossings of 60 dB at 125 Hz, 0 dB elsewhere) then also mutes
/// the 2 kHz band the model says passes untouched.
/// FAILS: the tone comes out tens of dB quieter than its band gain says.
#[test]
fn a_quiet_lowest_band_mutes_every_band() {
    let (open, g_open) = through_two_walls([0.0; 7]);
    let (low_cut, g_cut) = through_two_walls([60.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    eprintln!("2 kHz band gain: open {g_open} low-cut {g_cut}");
    eprintln!("2 kHz tone rms: open {open:.5} low-cut {low_cut:.5}");
    // The model gives the 2 kHz band the same gain in both scenes.
    assert_eq!(g_open, g_cut);
    let db = 20.0 * (low_cut / open).log10();
    assert!(db > -3.0, "2 kHz tone {db:.1} dB down");
}

/// A source past the delay line's reach arrives early: the line holds
/// `max_delay_s * 96 kHz` samples, so at 48 kHz anything beyond about
/// 2 * 0.5 s (340 m) is clamped, and the audible delay stops growing.
#[test]
fn far_source_delay_is_clamped() {
    let scene = Arc::new(SceneBuilder::new().build());
    let mut audio = Audio::new(
        Box::new(KiraBackend::offline(RATE).unwrap()),
        Box::new(LooseClips::none()),
    );
    audio.set_acoustics(Some(bare()));
    audio.set_listener(Vec3::new(0.0, 0.0, 1.5), 0.0);
    audio.update_acoustic(&scene);
    let mut s = vec![0.0f32; RATE as usize * 3];
    s[0] = 1.0;
    let click = Arc::new(Signal {
        samples: s,
        at: Mutex::new(0),
    });
    let d = 600.0f32;
    audio.start_feed(click, Some(Vec3::new(0.0, d, 1.5)), Bus::EFFECTS);
    let mut out = Vec::new();
    while out.len() < RATE as usize * 2 {
        audio.update_acoustic(&scene);
        out.extend(audio.render(800).unwrap());
    }
    let at = out
        .iter()
        .position(|f| f[0].abs() > 1e-5)
        .map(|i| i as f64 / f64::from(RATE));
    let want = f64::from(d) / scene.speed_of_sound();
    eprintln!("click at {at:?} s, travel time {want:.3} s");
    // Documented behaviour, not a crash; printed for the record.
    assert!(at.is_some());
}

/// A sealed room of a material that absorbs nothing: RT60 hits the 30 s
/// cap in every band. The shared reverb must stay bounded and decay, and
/// obstacle sense at full strength must stay finite, while the listener
/// stands a hair from a wall.
#[test]
fn lossless_room_reverb_stays_bounded() {
    let mut b = SceneBuilder::new();
    let id = b
        .add_material(&Material {
            name: "mirror".into(),
            absorption: Bands::ZERO,
            scattering: Bands::ZERO,
            transmission_loss_db: Bands::splat(60.0),
        })
        .unwrap();
    let (w, d, h) = (6.0, 5.0, 3.0);
    for (x0, y0, x1, y1) in [
        (0.0, 0.0, w, 0.0),
        (w, 0.0, w, d),
        (w, d, 0.0, d),
        (0.0, d, 0.0, 0.0),
    ] {
        b.add(Shape::wall_between(x0, y0, x1, y1, 0.0, h), id)
            .unwrap();
    }
    b.add(Shape::Ground { z: 0.0 }, id).unwrap();
    b.add(Shape::Ceiling { z: h }, id).unwrap();
    let scene = Arc::new(b.build());
    let mut audio = Audio::new(
        Box::new(KiraBackend::offline(RATE).unwrap()),
        Box::new(LooseClips::none()),
    );
    audio.set_acoustics(Some(AcousticSettings {
        obstacle_sense: 1.0,
        ..AcousticSettings::default()
    }));
    audio.set_listener(Vec3::new(0.001, 2.5, 1.5), 0.0);
    audio.update_acoustic(&scene);
    let mut x = 0x9e37_79b9u32;
    let noise: Vec<f32> = (0..RATE as usize / 5)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x as f32 / u32::MAX as f32 * 2.0 - 1.0) * 0.9
        })
        .collect();
    audio.start_feed(
        Arc::new(Signal {
            samples: noise,
            at: Mutex::new(0),
        }),
        Some(Vec3::new(4.0, 2.5, 1.5)),
        Bus::EFFECTS,
    );
    let mut out = Vec::new();
    while out.len() < RATE as usize * 6 {
        audio.update_acoustic(&scene);
        out.extend(audio.render(800).unwrap());
    }
    let peak = out
        .iter()
        .map(|f| f[0].abs().max(f[1].abs()))
        .fold(0.0f32, f32::max);
    let r = RATE as usize;
    let early = rms(&out[r / 2..r]);
    let late = rms(&out[5 * r..6 * r]);
    eprintln!("peak {peak} early rms {early} late rms {late}");
    assert!(out.iter().all(|f| f[0].is_finite() && f[1].is_finite()));
    assert!(peak < 8.0, "{peak}");
    assert!(late < early, "{early} {late}");
}
