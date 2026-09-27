//! Synthesis spike (throwaway): three recipes rendered offline to 48 kHz WAV.
//!
//! - chip: Faust modal model (compiled to Rust) with modes measured from a
//!   CC0 recording of a real chip strike.
//! - jetski: fundsp, driven by a throttle curve the way a game would drive it
//!   live -- fixed exhaust resonators, a moving firing rate, water on top.
//! - crowd: fundsp babble -- band-limited glottal sources through moving
//!   formants, consonant noise between syllables, a laugh, a room.
#![allow(clippy::excessive_precision)]

mod faust_prelude;
#[allow(non_snake_case, non_camel_case_types, non_upper_case_globals, unused)]
mod chip {
    use crate::faust_prelude::*;
    include!("../../faust/chip.rs");
}

use faust_prelude::Params;
use fundsp::prelude32::*;

const SR: f32 = 48_000.0;
const NONE: [&[f32]; 0] = [];

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn f(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f()
    }
    fn bi(&mut self) -> f32 {
        self.f() * 2.0 - 1.0
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[(self.next() % xs.len() as u64) as usize]
    }
}

fn at48<X: AudioNode>(mut n: An<X>) -> An<X> {
    n.set_sample_rate(SR as f64);
    n
}

fn save(name: &str, x: &[f32]) {
    let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-9);
    let g = 0.89 / peak; // -1 dBFS; loudness matching happens in measure.py
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SR as u32,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let path = format!("../out/{name}.wav");
    let mut w = hound::WavWriter::create(&path, spec).expect("create wav");
    for v in x {
        w.write_sample((v * g * 32767.0) as i16).expect("write");
    }
    w.finalize().expect("finalize");
    println!("wrote {path}");
}

// ---------------------------------------------------------------- chip (Faust)

/// Mode clusters read off CC0 recordings (Freesound 201808, 633997): a dense
/// group of plate modes between ~4.6 and 6.7 kHz carries the click, with weak
/// partials near 1.1, 3.9 and 9 kHz. Decay to -30 dB was ~55 ms.
fn chip_hit(rng: &mut Rng, out: &mut [f32], at_s: f32, gain: f32, pitch: f32) {
    let mut dsp = chip::ChipDsp::new();
    dsp.init(SR as i32);
    let mut p = Params::default();
    dsp.build_user_interface(&mut p);
    let idx = |n: &str| p.0[n];
    let mut modes: Vec<(f32, f32, f32)> = (0..8)
        .map(|_| (rng.range(5000.0, 7000.0), rng.range(0.07, 0.13), rng.range(0.35, 1.0)))
        .collect();
    modes.push((rng.range(1050.0, 1200.0), 0.05, 0.12));
    modes.push((rng.range(3800.0, 4200.0), 0.05, 0.1));
    modes.push((rng.range(8500.0, 9500.0), 0.05, 0.35));
    modes.push((rng.range(12_500.0, 15_000.0), 0.02, 0.45));
    for (i, (f, t, g)) in modes.iter().enumerate() {
        dsp.set_param(idx(&format!("f{i}")), f * pitch);
        dsp.set_param(idx(&format!("t{i}")), *t);
        dsp.set_param(idx(&format!("g{i}")), *g);
    }
    dsp.set_param(idx("bright"), 19_000.0);
    dsp.set_param(idx("contact"), 0.8);
    dsp.set_param(idx("width"), 0.0005);

    let n = (0.3 * SR) as usize;
    let mut buf = vec![0.0f32; n];
    let on = 24;
    dsp.set_param(idx("gate"), 1.0);
    dsp.compute(on, &NONE, &mut [&mut buf[..on]]);
    dsp.set_param(idx("gate"), 0.0);
    dsp.compute(n - on, &NONE, &mut [&mut buf[on..]]);
    let start = (at_s * SR) as usize;
    for (i, v) in buf.iter().enumerate() {
        if let Some(o) = out.get_mut(start + i) {
            *o += v * gain;
        }
    }
}

/// "two casino chips clacked onto felt" (0.5 s): the clack, the second chip a
/// beat later, a faint settle, and the dull landing on cloth.
fn chip_25() -> Vec<f32> {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut out = vec![0.0f32; (0.5 * SR) as usize];
    chip_hit(&mut rng, &mut out, 0.012, 1.0, 1.0);
    chip_hit(&mut rng, &mut out, 0.098, 0.75, 0.97);
    chip_hit(&mut rng, &mut out, 0.131, 0.18, 1.03);
    // felt: a short low burst under each landing
    let mut lp = at48(lowpass_hz(420.0, 0.7));
    let mut env = 0.0f32;
    for (i, o) in out.iter_mut().enumerate() {
        let t = i as f32 / SR;
        if (t - 0.012).abs() < 0.5 / SR || (t - 0.098).abs() < 0.5 / SR {
            env = 1.0;
        }
        env *= (-1.0 / (0.012 * SR)).exp();
        *o += lp.filter_mono(rng.bi()) * env * 0.35;
    }
    out
}

// ---------------------------------------------------------------- jet ski (fundsp)

/// Throttle over an 8 s ride: idle, open up, bounce over chop, back off.
fn throttle(t: f32) -> f32 {
    match t {
        t if t < 1.0 => 0.08,
        t if t < 2.5 => 0.08 + 0.8 * ((t - 1.0) / 1.5).powf(0.7),
        t if t < 6.0 => 0.88 + 0.05 * (std::f32::consts::TAU * 1.3 * t).sin(),
        t if t < 7.0 => 0.88 - 0.55 * (t - 6.0),
        _ => 0.33,
    }
}

fn jetski() -> Vec<f32> {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let n = (8.0 * SR) as usize;
    let mut out = vec![0.0f32; n];
    // Exhaust and hull: fixed resonances; only the firing rate moves.
    let mut res: Vec<(An<Resonator<f32, U1>>, f32)> = [
        (46.0, 18.0, 0.5),
        (92.0, 35.0, 1.0),
        (185.0, 55.0, 0.8),
        (370.0, 110.0, 0.1),
        (740.0, 260.0, 0.05),
    ]
    .iter()
    .map(|&(f, bw, g)| (at48(resonator_hz(f, bw)), g))
    .collect();
    let mut direct = at48(lowpass_hz(600.0, 0.7));
    let mut mech = at48(bandpass_hz(2200.0, 1.2));
    let mut muffle = at48(lowpass_hz(3000.0, 0.7));
    let mut spray_hp = at48(highpass_hz(1800.0, 0.7));
    let mut spray_pink = at48(pink());
    let mut fade_lp = at48(lowpole_hz(3.0));
    let mut gurgle_bp = at48(bandpass_hz(650.0, 1.4));
    let mut gurgle_am = at48(lowpole_hz(14.0));
    let mut slap_lp = at48(lowpass_hz(380.0, 0.9));

    let (mut rpm, mut speed) = (0.08f32, 0.0f32);
    let (mut phase, mut puff) = (0.0f32, 0.0f32);
    let amps = [0.95f32, 1.0, 0.72];
    let mut next_event = 0usize;
    let mut slap_env = 0.0f32;
    let puff_decay = (-1.0 / (0.0014 * SR)).exp();
    let slap_decay = (-1.0 / (0.045 * SR)).exp();
    for (i, o) in out.iter_mut().enumerate() {
        let t = i as f32 / SR;
        let thr = throttle(t);
        rpm += (thr - rpm) * (1.0 / (0.18 * SR)); // the engine answers fast
        speed += (thr - speed) * (1.0 / (1.4 * SR)); // the hull slowly
        let f0 = 42.0 + 88.0 * rpm; // firing rate: ~50 Hz idle, ~115 Hz cruise
        // three cylinders per cycle; uneven pulses give the sub-orders
        phase += f0 / 3.0 / SR;
        if phase >= 1.0 {
            phase -= 1.0;
            next_event = 0;
        }
        while next_event < 3 && phase >= next_event as f32 / 3.0 {
            puff += amps[next_event] * (1.0 + 0.08 * rng.bi()) * (0.35 + 0.65 * rpm);
            next_event += 1;
        }
        puff *= puff_decay;
        let exc = puff * (1.0 + 0.35 * rng.bi());
        let mut engine = direct.filter_mono(exc) * 0.25;
        for (r, g) in res.iter_mut() {
            engine += r.filter_mono(exc) * *g;
        }
        engine += mech.filter_mono(rng.bi()) * puff * 0.04 * (0.2 + rpm);
        muffle.set(Setting::center(1800.0 + 2600.0 * rpm));
        let engine = muffle.filter_mono(engine);

        // water: spray hiss with a slow Rayleigh-like fade, not a tremolo
        let fade = (fade_lp.filter_mono(rng.bi()) * 9.0).abs().min(1.5);
        let spray = spray_hp.filter_mono(spray_pink.get_mono()) * speed * speed * (0.5 + fade);
        let am = (gurgle_am.filter_mono(rng.bi()) * 6.0).abs().min(1.5);
        let gurgle = gurgle_bp.filter_mono(rng.bi()) * am * speed;
        if rng.f() < (1.0 + 5.0 * speed) / SR {
            slap_env += rng.range(0.4, 1.0) * speed;
        }
        slap_env *= slap_decay;
        let slap = slap_lp.filter_mono(rng.bi()) * slap_env;

        *o = engine * 0.9 + spray * 0.22 + gurgle * 0.12 + slap * 0.35;
    }
    out
}

// ---------------------------------------------------------------- crowd (fundsp)

const VOWELS: [[f32; 3]; 6] = [
    [730.0, 1090.0, 2440.0], // a
    [530.0, 1840.0, 2480.0], // e
    [270.0, 2290.0, 3010.0], // i
    [570.0, 840.0, 2410.0],  // o
    [300.0, 870.0, 2240.0],  // u
    [500.0, 1500.0, 2500.0], // schwa
];

struct Syllable {
    start: f32,
    voiced: f32,
    vowel: [f32; 3],
    consonant: u8, // 0 none, 1 fricative, 2 plosive, 3 the laugh's "h"
}

fn schedule(rng: &mut Rng, from: f32, to: f32, laugh: bool) -> Vec<Syllable> {
    let mut v = Vec::new();
    let mut t = from;
    while t < to {
        let voiced = if laugh { 0.085 } else { rng.range(0.11, 0.24) };
        let gap = if laugh { 0.07 } else { rng.range(0.03, 0.09) };
        let vowel = if laugh { VOWELS[0] } else { *rng.pick(&VOWELS) };
        let consonant = if laugh { 3 } else { (rng.next() % 3) as u8 };
        v.push(Syllable { start: t, voiced, vowel, consonant });
        let pause = if !laugh && rng.f() < 0.12 { rng.range(0.15, 0.4) } else { 0.0 };
        t += voiced + gap + pause;
    }
    v
}

/// One talker (or the laugh) rendered into `out`.
#[allow(clippy::too_many_arguments)]
fn talker(rng: &mut Rng, out: &mut [f32], syl: &[Syllable], pitch: f32, female: bool,
          gain: f32, dist_lp: f32, glide_down: bool) {
    let fscale = if female { 1.17 } else { 1.0 };
    let mut osc = at48(poly_saw());
    let mut tilt = at48(lowpole_hz(if female { 2200.0 } else { 1700.0 }));
    let mut fs: Vec<An<FixedSvf<f32, BandpassMode<f32>>>> =
        (0..3).map(|_| at48(bandpass_hz(1000.0, 4.0))).collect();
    let mut fric = at48(highpass_hz(3800.0, 0.7));
    let mut dist = at48(lowpass_hz(dist_lp, 0.7));
    let mut cur = syl.first().map(|s| s.vowel).unwrap_or(VOWELS[5]);
    let glide = 1.0 / (0.03 * SR);
    let phi = rng.range(0.0, 6.28);
    let vib = rng.range(4.5, 6.0);
    for s in syl {
        let a = (s.start.max(0.0) * SR) as usize;
        let voiced_n = (s.voiced * SR) as usize;
        let cons_n = match s.consonant {
            1 => (rng.range(0.04, 0.07) * SR) as usize,
            2 => (0.012 * SR) as usize,
            3 => (0.05 * SR) as usize,
            _ => 0,
        };
        for k in 0..cons_n {
            let Some(o) = out.get_mut(a + k) else { break };
            let x = k as f32 / cons_n as f32;
            let e = (std::f32::consts::PI * x).sin();
            let lvl = match s.consonant {
                1 => 0.32,
                2 => 0.45 * (1.0 - x),
                _ => 0.08,
            };
            let nz = if s.consonant == 3 {
                fs[1].filter_mono(rng.bi())
            } else {
                fric.filter_mono(rng.bi())
            };
            *o += dist.filter_mono(nz * e * lvl) * gain;
        }
        let base = a + cons_n;
        for k in 0..voiced_n {
            let Some(o) = out.get_mut(base + k) else { break };
            let x = k as f32 / voiced_n as f32;
            let t = (base + k) as f32 / SR;
            for (j, c) in cur.iter_mut().enumerate() {
                *c += (s.vowel[j] * fscale - *c) * glide;
            }
            let fall = if glide_down { 1.0 - 0.25 * x } else { 1.0 - 0.06 * x };
            let f = pitch * fall * (1.0 + 0.012 * (std::f32::consts::TAU * vib * t + phi).sin());
            let src = tilt.filter_mono(osc.filter_mono(f)) + rng.bi() * 0.10;
            let mut y = 0.0;
            for (j, bp) in fs.iter_mut().enumerate() {
                bp.set(Setting::center_q(cur[j], [5.0, 8.0, 10.0][j]));
                y += bp.filter_mono(src) * [1.0, 0.8, 0.6][j];
            }
            let e = (std::f32::consts::PI * x).sin().powf(0.6);
            *o += dist.filter_mono(y * e) * gain;
        }
    }
}

fn crowd_murmur() -> Vec<f32> {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let n = (3.0 * SR) as usize;
    let mut dry = vec![0.0f32; n];
    for v in 0..16 {
        let female = v % 2 == 1;
        let pitch = if female { rng.range(180.0, 240.0) } else { rng.range(95.0, 140.0) };
        let near = rng.f();
        let from = rng.range(-0.3, 0.6);
        let syl = schedule(&mut rng, from, 3.2, false);
        talker(&mut rng, &mut dry, &syl, pitch, female, 0.25 + 0.75 * near,
               5000.0 + 8000.0 * near, false);
    }
    let laugh = schedule(&mut rng, 1.55, 2.3, true);
    talker(&mut rng, &mut dry, &laugh, 265.0, true, 1.6, 9000.0, true);
    for (i, o) in dry.iter_mut().enumerate() {
        let t = i as f32 / SR;
        let e = (t / 0.7).min(1.0) * ((3.0 - t) / 0.8).clamp(0.0, 1.0);
        *o *= 0.35 + 0.65 * e;
    }
    let mut room = at48(reverb_stereo(14.0, 1.1, 0.55));
    let mut hp = at48(highpass_hz(140.0, 0.7));
    dry.iter()
        .map(|&x| {
            let (l, r) = room.filter_stereo(x, x);
            hp.filter_mono(x * 0.8 + (l + r) * 0.5 * 0.35)
        })
        .collect()
}

fn main() {
    save("chip_25_new", &chip_25());
    save("jetski_ride_new", &jetski());
    save("crowd_murmur_new_synth", &crowd_murmur());
}
