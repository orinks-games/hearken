//! The acoustic path heard through Kira's offline mixer: real audio out,
//! checked for delay, reflections, transmission, reverb, Doppler, HRTF and
//! obstacle sense; and the plain mix a backend without the insert gets.

#![cfg(feature = "kira")]

use std::sync::{Arc, Mutex};

use hearken_audio::acoustics::{Builtin, Scene, SceneBuilder, Shape};
use hearken_audio::{
    AcousticSettings, Audio, Backend, Bus, Call, Feed, FeedRead, KiraBackend, LooseClips,
    RecordingBackend, Vec3,
};

const RATE: u32 = 48_000;

/// A feed that plays a fixed signal, then silence.
struct Signal {
    samples: Vec<f32>,
    at: Mutex<usize>,
}

impl Signal {
    fn new(samples: Vec<f32>) -> Arc<Signal> {
        Arc::new(Signal {
            samples,
            at: Mutex::new(0),
        })
    }

    fn click() -> Arc<Signal> {
        let mut s = vec![0.0; RATE as usize * 2];
        s[0] = 1.0;
        Signal::new(s)
    }

    fn noise(seconds: f32) -> Arc<Signal> {
        let mut x = 0x1234_5678u32;
        let s = (0..(seconds * RATE as f32) as usize)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x as f32 / u32::MAX as f32 * 2.0 - 1.0) * 0.5
            })
            .collect();
        Signal::new(s)
    }

    fn sine(hz: f32, seconds: f32) -> Arc<Signal> {
        let s = (0..(seconds * RATE as f32) as usize)
            .map(|n| (std::f32::consts::TAU * hz * n as f32 / RATE as f32).sin() * 0.5)
            .collect();
        Signal::new(s)
    }
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

fn offline() -> Audio {
    let backend = KiraBackend::offline(RATE).expect("offline mixer");
    Audio::new(Box::new(backend), Box::new(LooseClips::none()))
}

/// Settings for measuring one effect at a time.
fn bare() -> AcousticSettings {
    AcousticSettings {
        room: false,
        reflections: false,
        ..AcousticSettings::default()
    }
}

/// Renders `seconds`, running an acoustic update about every 800 frames (60
/// a second), with `step` called before each with the time so far.
fn run(
    audio: &mut Audio,
    scene: &Arc<Scene>,
    seconds: f32,
    mut step: impl FnMut(&mut Audio, f32),
) -> Vec<[f32; 2]> {
    let total = (seconds * RATE as f32) as usize;
    let mut out = Vec::with_capacity(total + 1024);
    while out.len() < total {
        // The time rendered so far: a render may hand back a few frames
        // more than asked.
        step(audio, out.len() as f32 / RATE as f32);
        audio.update_acoustic(scene);
        out.extend(audio.render(800).expect("offline render"));
    }
    out.truncate(total);
    out
}

fn energy(x: &[[f32; 2]], side: usize) -> f64 {
    x.iter().map(|f| f64::from(f[side]).powi(2)).sum()
}

fn ms(t: f32) -> usize {
    (t / 1000.0 * RATE as f32) as usize
}

/// The first frame louder than `threshold` in either ear.
fn onset(x: &[[f32; 2]], threshold: f32) -> Option<usize> {
    x.iter()
        .position(|f| f[0].abs() > threshold || f[1].abs() > threshold)
}

fn open_air() -> Arc<Scene> {
    Arc::new(SceneBuilder::new().build())
}

#[test]
fn sound_arrives_after_its_travel_time() {
    let mut audio = offline();
    audio.set_acoustics(Some(bare()));
    let scene = open_air();
    audio.set_listener(Vec3::new(0.0, 0.0, 1.5), 0.0);
    audio.update_acoustic(&scene);
    audio.start_feed(
        Signal::click(),
        Some(Vec3::new(0.0, 17.0, 1.5)),
        Bus::EFFECTS,
    );
    let out = run(&mut audio, &scene, 0.2, |_, _| {});
    let at = onset(&out, 0.005).expect("the click is heard");
    let expected = 17.0 / scene.speed_of_sound() * f64::from(RATE);
    let off = (at as f64 - expected).abs();
    // Within a block of Kira's start and the feed's own resampling.
    assert!(off < 160.0, "heard at {at}, expected about {expected:.0}");
}

#[test]
fn a_wall_on_the_left_is_heard_on_the_left() {
    // A concrete wall 2 m to the listener's left, running north; the
    // sound is 5 m ahead. Its reflection arrives after the direct sound,
    // from the left.
    let mut b = SceneBuilder::new();
    b.add(
        Shape::wall_between(-2.0, -20.0, -2.0, 20.0, 0.0, 6.0),
        Builtin::Concrete.id(),
    )
    .unwrap();
    let scene = Arc::new(b.build());
    let mut audio = offline();
    audio.set_acoustics(Some(AcousticSettings {
        room: false,
        ..AcousticSettings::default()
    }));
    audio.set_listener(Vec3::new(0.0, 0.0, 1.5), 0.0);
    audio.update_acoustic(&scene);
    audio.start_feed(
        Signal::click(),
        Some(Vec3::new(0.0, 5.0, 1.5)),
        Bus::EFFECTS,
    );
    let out = run(&mut audio, &scene, 0.1, |_, _| {});

    let c = scene.speed_of_sound() as f32;
    let direct = onset(&out, 0.01).expect("the direct sound");
    let image = (16.0f32 + 25.0).sqrt() / c * 1000.0;
    let lag = ms(image) as isize - ms(5.0 / c * 1000.0) as isize;
    let start = (direct as isize + lag - ms(1.0) as isize) as usize;
    let window = &out[start..start + ms(3.0)];
    let (l, r) = (energy(window, 0), energy(window, 1));
    assert!(l > r * 4.0, "reflection window: left {l} right {r}");
    // The direct sound, straight ahead, is centred.
    let front = &out[direct..direct + ms(1.0)];
    let (fl, fr) = (energy(front, 0), energy(front, 1));
    assert!((fl / fr - 1.0).abs() < 0.05, "direct: left {fl} right {fr}");
}

/// Energy below 300 Hz and above 3 kHz, by one-pole filters.
fn lows_and_highs(x: &[[f32; 2]]) -> (f64, f64) {
    let a_lo = (-std::f32::consts::TAU * 300.0 / RATE as f32).exp();
    let a_hi = (-std::f32::consts::TAU * 3000.0 / RATE as f32).exp();
    let (mut lo, mut lp_hi) = (0.0f32, 0.0f32);
    let (mut e_lo, mut e_hi) = (0.0f64, 0.0f64);
    for f in x {
        let v = 0.5 * (f[0] + f[1]);
        lo = lo * a_lo + v * (1.0 - a_lo);
        lp_hi = lp_hi * a_hi + v * (1.0 - a_hi);
        let hi = v - lp_hi;
        e_lo += f64::from(lo * lo);
        e_hi += f64::from(hi * hi);
    }
    (e_lo, e_hi)
}

fn through_wall(material: Builtin) -> (f64, f64) {
    // A wall far wider than the path search reaches, so only what passes
    // through it is heard.
    let mut b = SceneBuilder::new();
    b.add(
        Shape::wall_between(-5000.0, 3.0, 5000.0, 3.0, -100.0, 500.0),
        material.id(),
    )
    .unwrap();
    let scene = Arc::new(b.build());
    let mut audio = offline();
    audio.set_acoustics(Some(bare()));
    audio.set_listener(Vec3::new(0.0, 0.0, 1.5), 0.0);
    audio.update_acoustic(&scene);
    audio.start_feed(
        Signal::noise(1.0),
        Some(Vec3::new(0.0, 6.0, 1.5)),
        Bus::EFFECTS,
    );
    let out = run(&mut audio, &scene, 0.8, |_, _| {});
    lows_and_highs(&out[ms(200.0)..])
}

#[test]
fn concrete_cuts_highs_more_than_a_curtain() {
    let (c_lo, c_hi) = through_wall(Builtin::Concrete);
    let (k_lo, k_hi) = through_wall(Builtin::Curtain);
    assert!(c_lo > 0.0 && k_lo > 0.0);
    // Concrete is far quieter overall...
    assert!(
        c_lo + c_hi < (k_lo + k_hi) * 0.01,
        "concrete {c_lo} {c_hi}, curtain {k_lo} {k_hi}"
    );
    // ...and darker: its highs fall further than its lows.
    let tilt_concrete = 10.0 * (c_hi / c_lo).log10();
    let tilt_curtain = 10.0 * (k_hi / k_lo).log10();
    assert!(
        tilt_concrete < tilt_curtain - 6.0,
        "high/low: concrete {tilt_concrete:.1} dB, curtain {tilt_curtain:.1} dB"
    );
}

fn closed_room() -> Arc<Scene> {
    let mut b = SceneBuilder::new();
    let m = Builtin::Plaster.id();
    let (w, d, h) = (8.0, 6.0, 3.0);
    b.add(Shape::wall_between(0.0, 0.0, w, 0.0, 0.0, h), m)
        .unwrap();
    b.add(Shape::wall_between(w, 0.0, w, d, 0.0, h), m).unwrap();
    b.add(Shape::wall_between(w, d, 0.0, d, 0.0, h), m).unwrap();
    b.add(Shape::wall_between(0.0, d, 0.0, 0.0, 0.0, h), m)
        .unwrap();
    b.add(Shape::Ground { z: 0.0 }, Builtin::Concrete.id())
        .unwrap();
    b.add(Shape::Ceiling { z: h }, m).unwrap();
    Arc::new(b.build())
}

fn tail_energy(room: bool) -> f64 {
    let scene = closed_room();
    let mut audio = offline();
    audio.set_acoustics(Some(AcousticSettings {
        room,
        ..AcousticSettings::default()
    }));
    audio.set_listener(Vec3::new(3.0, 2.0, 1.5), 0.0);
    audio.update_acoustic(&scene);
    audio.start_feed(
        Signal::click(),
        Some(Vec3::new(5.0, 4.0, 1.5)),
        Bus::EFFECTS,
    );
    let out = run(&mut audio, &scene, 0.6, |_, _| {});
    energy(&out[ms(120.0)..], 0) + energy(&out[ms(120.0)..], 1)
}

#[test]
fn a_closed_room_rings_on_after_the_early_sound() {
    let with = tail_energy(true);
    let without = tail_energy(false);
    assert!(with > 0.0);
    assert!(
        with > without * 100.0,
        "tail with room {with}, without {without}"
    );
}

/// Mean frequency from zero crossings of the left ear.
fn frequency(x: &[[f32; 2]]) -> f32 {
    let mut crossings = 0;
    for w in x.windows(2) {
        if w[0][0] <= 0.0 && w[1][0] > 0.0 {
            crossings += 1;
        }
    }
    crossings as f32 / (x.len() as f32 / RATE as f32)
}

/// Power at `hz` (Goertzel), left ear.
fn power_at(x: &[[f32; 2]], hz: f32) -> f64 {
    let w = std::f64::consts::TAU * f64::from(hz) / f64::from(RATE);
    let k = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for f in x {
        let s = f64::from(f[0]) + k * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    s1 * s1 + s2 * s2 - k * s1 * s2
}

fn approaching(doppler: bool) -> (f32, f64, f64) {
    let scene = open_air();
    let mut audio = offline();
    audio.set_acoustics(Some(AcousticSettings { doppler, ..bare() }));
    audio.set_listener(Vec3::new(0.0, 0.0, 1.5), 0.0);
    audio.update_acoustic(&scene);
    let speed = 30.0f32;
    let start = 60.0f32;
    let sound = audio.start_feed(
        Signal::sine(1000.0, 2.0),
        Some(Vec3::new(0.0, start, 1.5)),
        Bus::EFFECTS,
    );
    let out = run(&mut audio, &scene, 1.2, |audio, t| {
        audio.set_position(sound, Some(Vec3::new(0.0, start - speed * t, 1.5)));
    });
    let part = &out[ms(500.0)..ms(1100.0)];
    (
        frequency(part),
        power_at(part, 1000.0),
        power_at(part, 1087.0),
    )
}

#[test]
fn an_approaching_sound_rises_in_pitch_with_doppler_only() {
    // The delay follows the distance now, so the shift is f (1 + v / c):
    // about 1087 Hz at 30 m/s (the exact moving-source value is 1096).
    let (with, at_1000, at_shifted) = approaching(true);
    assert!(
        (with / 1000.0 - 1.0874).abs() < 0.006,
        "with Doppler {with} Hz"
    );
    assert!(at_shifted > at_1000 * 10.0, "{at_1000} {at_shifted}");
    // Without, the tone stays put (a new delay crossfades in).
    let (_, at_1000, at_shifted) = approaching(false);
    assert!(at_1000 > at_shifted * 10.0, "{at_1000} {at_shifted}");
}

#[test]
fn sounds_on_other_buses_are_untouched() {
    let render = |acoustics: bool| {
        let scene = closed_room();
        let mut audio = offline();
        if acoustics {
            audio.set_acoustics(Some(AcousticSettings::default()));
        }
        audio.set_listener(Vec3::new(3.0, 2.0, 1.5), 0.0);
        audio.start_feed(Signal::noise(0.3), Some(Vec3::new(5.0, 2.0, 1.5)), Bus::UI);
        let mut out = Vec::new();
        for _ in 0..12 {
            if acoustics {
                audio.update_acoustic(&scene);
            } else {
                audio.update(&|_| false);
            }
            out.extend(audio.render(800).unwrap());
        }
        out
    };
    assert_eq!(render(true), render(false));
}

#[test]
fn the_old_update_still_works_on_an_acoustic_sound() {
    let scene = open_air();
    let mut audio = offline();
    audio.set_acoustics(Some(bare()));
    audio.set_listener(Vec3::new(0.0, 0.0, 1.5), 0.0);
    audio.update_acoustic(&scene);
    audio.start_feed(
        Signal::noise(1.0),
        Some(Vec3::new(10.0, 0.0, 1.5)),
        Bus::EFFECTS,
    );
    for _ in 0..10 {
        audio.update(&|_| false);
        audio.render(800).unwrap();
    }
    let out = audio.render(4800).unwrap();
    // Back on the plain model: panned hard right, by Kira's own panner.
    let (l, r) = (energy(&out, 0), energy(&out, 1));
    assert!(r > 0.0 && l < r * 0.01, "left {l} right {r}");
}

#[test]
fn a_backend_without_the_insert_gets_a_plain_mix() {
    let (backend, calls) = RecordingBackend::new();
    assert!(!backend.renders_acoustics());
    let mut audio = Audio::new(Box::new(backend), Box::new(LooseClips::none()));
    audio.set_acoustics(Some(bare()));
    let mut b = SceneBuilder::new();
    b.add(
        Shape::wall_between(-5000.0, 3.0, 5000.0, 3.0, -100.0, 500.0),
        Builtin::Brick.id(),
    )
    .unwrap();
    let scene = Arc::new(b.build());
    audio.set_listener(Vec3::new(0.0, 0.0, 1.5), 0.0);
    audio.update_acoustic(&scene);
    let near = audio.start_feed(
        Signal::noise(1.0),
        Some(Vec3::new(2.0, 0.0, 1.5)),
        Bus::EFFECTS,
    );
    let behind_wall = audio.start_feed(
        Signal::noise(1.0),
        Some(Vec3::new(0.0, 6.0, 1.5)),
        Bus::EFFECTS,
    );
    audio.update_acoustic(&scene);
    let mixes: Vec<_> = calls
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            Call::Play { channel, mix, .. } => Some((channel, mix)),
            _ => None,
        })
        .collect();
    assert_eq!(mixes.len(), 2);
    let (_, m) = mixes[0];
    // 2 m to the right: half the level at the 1 m reference, panned right,
    // nothing muffled.
    assert!((m.gain - 0.5).abs() < 0.02, "{m:?}");
    assert!(m.pan > 0.99 && m.lowpass_hz.is_none(), "{m:?}");
    let (_, m) = mixes[1];
    assert!(m.gain < 0.01, "{m:?}");
    assert!(m.lowpass_hz.is_some_and(|hz| hz < 4000.0), "{m:?}");
    assert!(audio.is_playing(near) && audio.is_playing(behind_wall));
}

#[test]
fn obstacle_sense_hears_a_near_wall() {
    let wall_at = |x: f64| {
        let mut b = SceneBuilder::new();
        b.add(
            Shape::wall_between(x, -20.0, x, 20.0, 0.0, 6.0),
            Builtin::Concrete.id(),
        )
        .unwrap();
        Arc::new(b.build())
    };
    let wall = wall_at(-0.6);
    let render = |scene: &Arc<Scene>, strength: f32| {
        let mut audio = offline();
        audio.set_acoustics(Some(AcousticSettings {
            obstacle_sense: strength,
            ..AcousticSettings::default()
        }));
        audio.set_listener(Vec3::new(0.0, 0.0, 1.5), 0.0);
        let out = run(&mut audio, scene, 1.0, |_, _| {});
        out[ms(300.0)..].to_vec()
    };
    let off = render(&wall, 0.0);
    assert_eq!(energy(&off, 0) + energy(&off, 1), 0.0);
    let clear = render(&open_air(), 1.0);
    let near = render(&wall, 1.0);
    let balance = |x: &[[f32; 2]]| 10.0 * (energy(x, 0) / energy(x, 1)).log10();
    let (lo_c, hi_c) = lows_and_highs(&clear);
    assert!(
        balance(&clear).abs() < 1.0,
        "open air is even: {}",
        balance(&clear)
    );
    // A wall 0.6 m to the left: that side's own noise is blocked and the
    // other side's comes back off the wall, so the ears no longer match.
    let shift = balance(&near) - balance(&clear);
    assert!(shift.abs() > 1.0, "balance moved {shift:.2} dB");
    // At 0.2 m the reflection is so close it lifts the bass.
    let close = render(&wall_at(-0.2), 1.0);
    let (lo_x, hi_x) = lows_and_highs(&close);
    let tilt = |lo: f64, hi: f64| 10.0 * (lo / hi).log10();
    let lift = tilt(lo_x, hi_x) - tilt(lo_c, hi_c);
    assert!(lift > 0.5, "bass lift {lift:.2} dB");
}

#[cfg(feature = "hrtf")]
#[test]
fn headphones_put_a_sound_on_the_right_in_the_right_ear() {
    let scene = open_air();
    let mut audio = offline();
    audio.set_acoustics(Some(AcousticSettings {
        hrtf: true,
        ..bare()
    }));
    audio.set_listener(Vec3::new(0.0, 0.0, 1.5), 0.0);
    // Two updates: one learns the mixer's rate and loads the set.
    audio.update_acoustic(&scene);
    audio.render(800);
    audio.update_acoustic(&scene);
    audio.start_feed(
        Signal::click(),
        Some(Vec3::new(3.0, 0.0, 1.5)),
        Bus::EFFECTS,
    );
    let out = run(&mut audio, &scene, 0.1, |_, _| {});
    let left = onset(&out.iter().map(|f| [f[0], 0.0]).collect::<Vec<_>>(), 0.002);
    let right = onset(&out.iter().map(|f| [0.0, f[1]]).collect::<Vec<_>>(), 0.002);
    let (l, r) = (energy(&out, 0), energy(&out, 1));
    assert!(r > l * 3.0, "left {l} right {r}");
    // And sooner: the right ear is nearer.
    let (left, right) = (left.unwrap(), right.unwrap());
    assert!(right + 10 < left, "onsets: left {left} right {right}");
}
