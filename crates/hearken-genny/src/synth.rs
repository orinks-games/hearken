//! Segments, durations, parameter tracks, pitch and the Klatt-style
//! cascade, ported from genny's `speech.py`.

use std::f64::consts::PI;

use crate::phonemes::{
    burst_db, burst_spec, coart, fric_db, fric_spec, locus, ph, Kind, Place, Spectrum, F4, F5, FNP,
};
use crate::text::{Phrase, Word};
use crate::Voice;

// ---------------------------------------------------------------------------
// Segments
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct Seg {
    /// `None` is a pause.
    ph: Option<usize>,
    st: u8,
    phrase: usize,
    word: Option<(usize, usize)>,
    letter: bool,
    wstart: bool,
    wend: bool,
    fin: bool,
    dur: f64,
    head: f64,
    head_asp: f64,
    burst: f64,
    post: f64,
    burst_amp: f64,
    s: f64,
    e: f64,
}

impl Seg {
    fn pause(dur: f64) -> Seg {
        Seg {
            ph: None,
            dur,
            ..Seg::default()
        }
    }

    fn kind(&self) -> Option<Kind> {
        self.ph.map(|p| ph(p).kind)
    }

    fn vowelish(&self) -> bool {
        self.kind().is_some_and(Kind::vowelish)
    }
}

fn build_segments(phrases: &[Phrase], rate: f64) -> Vec<Seg> {
    let mut segs: Vec<Seg> = Vec::new();
    for (pi, phr) in phrases.iter().enumerate() {
        let start = segs.len();
        let real: Vec<usize> = phr
            .words
            .iter()
            .enumerate()
            .filter(|(_, Word::Spoken { ph, .. })| !ph.is_empty())
            .map(|(i, _)| i)
            .collect();
        for (wi, Word::Spoken { ph: phones, letter }) in phr.words.iter().enumerate() {
            if phones.is_empty() {
                continue;
            }
            for (k, &(p, st)) in phones.iter().enumerate() {
                segs.push(Seg {
                    ph: Some(p),
                    st,
                    phrase: pi,
                    word: Some((pi, wi)),
                    letter: *letter,
                    wstart: k == 0,
                    wend: k == phones.len() - 1,
                    ..Seg::default()
                });
            }
            let is_last = real.last().is_none_or(|&last| last == wi);
            if !is_last && *letter {
                segs.push(Seg::pause(0.07));
            }
        }
        let idx: Vec<usize> = (start..segs.len())
            .filter(|&j| segs[j].ph.is_some())
            .collect();
        if let Some(&last_v) = idx.iter().rev().find(|&&j| segs[j].vowelish()) {
            for &j in &idx {
                if j >= last_v {
                    segs[j].fin = true;
                }
            }
        }
        for &j in &idx {
            if segs[j].letter && segs[j].vowelish() && segs[j].st > 0 {
                segs[j].fin = true;
            }
        }
        if pi < phrases.len() - 1 {
            segs.push(Seg::pause(if phr.punct == ',' { 0.16 } else { 0.28 }));
        }
    }
    durations(&mut segs, rate);
    segs
}

fn durations(segs: &mut [Seg], rate: f64) {
    let r = rate.max(0.1);
    let stretch = 1.12 / r;
    let n = segs.len();
    let mut words: Vec<(usize, usize)> = segs.iter().filter_map(|s| s.ph.and(s.word)).collect();
    words.dedup();
    words.sort();
    words.dedup();
    let citation = if words.len() == 1 { 1.25 } else { 1.0 };
    for i in 0..n {
        let Some(p) = segs[i].ph else {
            segs[i].dur /= r;
            continue;
        };
        let info = ph(p);
        let (inh, mn) = info.dur;
        let k = info.kind;
        let mut pct = 1.0;
        let nxt = (i + 1 < n && segs[i + 1].ph.is_some()).then(|| segs[i + 1].clone());
        let prv = (i > 0 && segs[i - 1].ph.is_some()).then(|| segs[i - 1].clone());
        let sg = &segs[i];
        let same_word_next = nxt.as_ref().is_some_and(|x| x.word == sg.word);
        if k.vowelish() {
            if sg.st == 0 {
                pct *= 0.5;
            } else if sg.st == 2 {
                pct *= 0.8;
            }
            if sg.fin {
                pct *= 1.9;
            }
            if same_word_next {
                let nk = ph(nxt.as_ref().and_then(|x| x.ph).unwrap_or(0));
                if matches!(nk.kind, Kind::Stop | Kind::Affricate | Kind::Fricative) && !nk.voiced {
                    pct *= 0.82;
                } else if sg.fin && !nk.kind.sonorant() {
                    pct *= 1.12;
                }
            }
        } else {
            let cons_next = nxt.as_ref().is_some_and(|x| !x.vowelish()) && same_word_next;
            let cons_prev = prv
                .as_ref()
                .is_some_and(|x| !x.vowelish() && x.word == sg.word);
            if cons_next || cons_prev {
                pct *= 0.8;
            }
            if sg.fin {
                pct *= 1.25;
            }
        }
        let c = if k.vowelish() { citation } else { 1.0 };
        segs[i].dur = (mn + (inh - mn) * pct) / 1000.0 * stretch * c;
    }
    // stop release details / VOT
    for i in 0..n {
        let Some(p) = segs[i].ph else { continue };
        let info = ph(p);
        if info.kind != Kind::Stop {
            continue;
        }
        let has_next = i + 1 < n && segs[i + 1].ph.is_some();
        let prv_s = i > 0
            && segs[i - 1].ph.is_some_and(|q| ph(q).name == "S")
            && segs[i - 1].word == segs[i].word;
        segs[i].burst = match info.place {
            Place::Lab => 0.006,
            Place::Alv => 0.009,
            _ => 0.015,
        };
        segs[i].post = 0.0;
        segs[i].burst_amp = 1.0;
        if has_next {
            let nk = segs[i + 1].kind().unwrap_or(Kind::Vowel);
            if nk.sonorant() || matches!(nk, Kind::Tap | Kind::Trill) {
                let (vot, asp) = if info.voiced {
                    (0.008, 0.0)
                } else if prv_s {
                    (0.014, 0.5)
                } else if segs[i].wstart || segs[i + 1].st >= 1 {
                    (0.058, 1.0)
                } else {
                    (0.03, 0.8)
                };
                let head = vot / r.sqrt();
                segs[i + 1].head = head;
                segs[i + 1].head_asp = asp;
                segs[i + 1].dur += head * 0.7;
            } else {
                segs[i].burst_amp = 0.45;
            }
        } else {
            segs[i].post = (if info.voiced { 0.012 } else { 0.04 }) / r.sqrt();
            segs[i].burst_amp = 0.7;
            let post = segs[i].post;
            segs[i].dur += post;
        }
    }
    let mut t = 0.012;
    for sg in segs.iter_mut() {
        sg.s = t;
        t += sg.dur;
        sg.e = t;
    }
}

// ---------------------------------------------------------------------------
// Targets
// ---------------------------------------------------------------------------

const HM_LO: [f64; 6] = [
    165.327516,
    -6.73636734e-01,
    1.80874446e-03,
    -4.52201682e-06,
    7.49514000e-09,
    -4.70219241e-12,
];
const HM_HI: [f64; 6] = [
    15.8146139,
    8.10159009e-02,
    -9.79728215e-05,
    5.28725064e-08,
    -1.07099364e-11,
    7.91528509e-16,
];

/// Hawks & Miller 1995 formant bandwidth at F0 = 132 Hz.
fn formant_bandwidth(f: f64) -> f64 {
    let f = f.clamp(100.0, 5000.0);
    let c = if f < 500.0 { &HM_LO } else { &HM_HI };
    c.iter()
        .enumerate()
        .map(|(i, ci)| ci * f.powi(i as i32))
        .sum()
}

type Target = [f64; 10];

fn with_bw(f: [f64; 5], kind: Kind) -> Target {
    let bw: [f64; 5] = if kind.vowelish() {
        f.map(formant_bandwidth)
    } else if kind == Kind::Nasal {
        [100.0, 300.0, 300.0, 350.0, 400.0]
    } else if matches!(kind, Kind::Liquid | Kind::Glide) {
        [80.0, 120.0, 150.0, 250.0, 320.0]
    } else {
        [120.0, 150.0, 200.0, 300.0, 400.0]
    };
    let mut t = [0.0; 10];
    t[..5].copy_from_slice(&f);
    t[5..].copy_from_slice(&bw);
    t
}

fn five(f: [f64; 3]) -> [f64; 5] {
    [f[0], f[1], f[2], F4, F5]
}

fn vowel_target(p: usize) -> ([f64; 5], [f64; 5]) {
    let info = ph(p);
    (five(info.f), five(info.f2))
}

fn nearest_vowel(segs: &[Seg], i: usize) -> Option<[f64; 5]> {
    let n = segs.len();
    for seg in segs.iter().take(n.min(i + 4)).skip(i + 1) {
        let p = seg.ph?;
        if ph(p).kind.vowelish() {
            return Some(vowel_target(p).0);
        }
    }
    let lo = i.saturating_sub(3);
    for j in (lo..i).rev() {
        let p = segs[j].ph?;
        if ph(p).kind.vowelish() {
            return Some(vowel_target(p).1);
        }
    }
    None
}

fn targets(segs: &[Seg]) -> Vec<Option<(Target, Target)>> {
    let mut out = Vec::with_capacity(segs.len());
    for (i, sg) in segs.iter().enumerate() {
        let Some(p) = sg.ph else {
            out.push(None);
            continue;
        };
        let info = ph(p);
        let k = info.kind;
        if k.vowelish() {
            let (a, z) = vowel_target(p);
            out.push(Some((with_bw(a, k), with_bw(z, k))));
            continue;
        }
        if matches!(k, Kind::Liquid | Kind::Glide) {
            let mut f = five(info.f);
            if info.name == "L" && sg.wend && !sg.wstart {
                f[1] = 900.0; // dark final l
            }
            let t = with_bw(f, k);
            out.push(Some((t, t)));
            continue;
        }
        let vt = nearest_vowel(segs, i);
        if k == Kind::Aspirate {
            let f = vt.unwrap_or([500.0, 1500.0, 2500.0, F4, F5]);
            let mut t = [0.0; 10];
            t[..5].copy_from_slice(&f);
            t[5..].copy_from_slice(&[250.0, 200.0, 250.0, 300.0, 400.0]);
            out.push(Some((t, t)));
            continue;
        }
        let mut f: [f64; 5];
        if info.place == Place::Vel {
            let vf2 = vt.map_or(1500.0, |v| v[1]);
            let f2 = (vf2 + 150.0).clamp(1350.0, 2400.0);
            f = [250.0, f2, (f2 + 450.0).max(2200.0), F4, F5];
        } else {
            let loc = locus(info.place);
            f = [loc[0], loc[1], loc[2], F4, F5];
            if let Some(v) = vt {
                let kk = coart(k);
                f[1] = loc[1] + kk * (v[1] - loc[1]);
                f[2] = loc[2] + kk * 0.6 * (v[2] - loc[2]);
            }
        }
        if k == Kind::Nasal {
            f[0] = 280.0;
        }
        if matches!(k, Kind::Tap | Kind::Trill) {
            f[0] = 300.0;
            if let Some(v) = vt {
                f[1] = 1700.0 + 0.5 * (v[1] - 1700.0);
            }
        }
        let t = with_bw(f, k);
        out.push(Some((t, t)));
    }
    out
}

fn trans(ka: Kind, kb: Kind) -> (f64, f64) {
    let obs = |k: Kind| -> Option<f64> {
        Some(match k {
            Kind::Stop => 0.05,
            Kind::Affricate => 0.04,
            Kind::Fricative => 0.045,
            Kind::Aspirate => 0.02,
            Kind::Nasal => 0.035,
            Kind::Tap => 0.03,
            Kind::Trill => 0.035,
            _ => return None,
        })
    };
    match (obs(ka), obs(kb)) {
        (Some(_), Some(_)) => (0.015, 0.5),
        (Some(a), None) => (a, 0.05),
        (None, Some(b)) => (b + 0.005, 0.95),
        _ if matches!(ka, Kind::Liquid | Kind::Glide)
            || matches!(kb, Kind::Liquid | Kind::Glide) =>
        {
            (0.075, 0.5)
        }
        _ => (0.1, 0.5),
    }
}

// ---------------------------------------------------------------------------
// Tracks
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Tr {
    Fm(usize),
    Av,
    Ah,
    Af,
    Avb,
    B1add,
    Fnz,
    F0,
    /// Frication band j: 0 = centre, 1 = bandwidth, 2 = gain.
    Fr(usize, usize),
    FrByp,
}

impl Tr {
    fn index(self) -> usize {
        match self {
            Tr::Fm(j) => j,
            Tr::Av => 10,
            Tr::Ah => 11,
            Tr::Af => 12,
            Tr::Avb => 13,
            Tr::B1add => 14,
            Tr::Fnz => 15,
            Tr::F0 => 16,
            Tr::Fr(j, w) => 17 + j * 3 + w,
            Tr::FrByp => 26,
        }
    }
}

struct Tracks {
    p: Vec<Vec<(f64, f64)>>,
}

impl Tracks {
    fn new() -> Tracks {
        Tracks {
            p: vec![Vec::new(); 27],
        }
    }

    fn add(&mut self, tr: Tr, t: f64, v: f64) {
        self.p[tr.index()].push((t, v));
    }

    fn curve(&self, tr: Tr, times: &[f64], default: f64) -> Vec<f64> {
        let pts = &self.p[tr.index()];
        if pts.is_empty() {
            return vec![default; times.len()];
        }
        let mut pts = pts.clone();
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut xs: Vec<f64> = pts.iter().map(|q| q.0).collect();
        let ys: Vec<f64> = pts.iter().map(|q| q.1).collect();
        for j in 1..xs.len() {
            if xs[j] <= xs[j - 1] {
                xs[j] = xs[j - 1] + 1e-6;
            }
        }
        interp(times, &xs, &ys)
    }
}

/// `numpy.interp`: linear, held flat past both ends; `xs` increasing.
fn interp(at: &[f64], xs: &[f64], ys: &[f64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(at.len());
    let n = xs.len();
    let mut j = 0;
    let sorted = at.windows(2).all(|w| w[0] <= w[1]);
    for &t in at {
        if t <= xs[0] {
            out.push(ys[0]);
            continue;
        }
        if t >= xs[n - 1] {
            out.push(ys[n - 1]);
            continue;
        }
        if !sorted {
            j = 0;
        }
        while j + 1 < n && xs[j + 1] < t {
            j += 1;
        }
        let (x0, x1) = (xs[j], xs[j + 1]);
        let u = if x1 > x0 { (t - x0) / (x1 - x0) } else { 0.0 };
        out.push(ys[j] + u * (ys[j + 1] - ys[j]));
    }
    out
}

fn build_tracks(segs: &[Seg], v: &Voice) -> Tracks {
    let mut tr = Tracks::new();
    let n = segs.len();
    let tgt = targets(segs);
    let fshift = v.formant_shift;
    let kinds: Vec<Option<Kind>> = segs.iter().map(Seg::kind).collect();
    let mut tin = vec![0.0; n];
    let mut tout = vec![0.0; n];
    for i in 0..n.saturating_sub(1) {
        if let (Some(a), Some(b)) = (kinds[i], kinds[i + 1]) {
            let (tt, fl) = trans(a, b);
            tout[i] = tt * fl;
            tin[i + 1] = tt * (1.0 - fl);
        }
    }
    for (i, sg) in segs.iter().enumerate() {
        let Some(kind) = kinds[i] else { continue };
        let d = sg.e - sg.s;
        if sg.head > 0.0 {
            tin[i] = f64::max(tin[i], sg.head + 0.015);
        }
        let tot = tin[i] + tout[i];
        if tot > 0.85 * d {
            let sc = 0.85 * d / tot;
            tin[i] *= sc;
            tout[i] *= sc;
        }
        let (a, z) = tgt[i].expect("a target for every phoneme");
        let (t1, t2) = (sg.s + tin[i], sg.e - tout[i]);
        let mut pts: Vec<(f64, Target)> = vec![(t1, a)];
        if kind == Kind::Diphthong {
            pts.push((t1 + 0.3 * (t2 - t1), a));
        }
        pts.push((t2, z));
        if i == 0 || kinds[i - 1].is_none() {
            pts.insert(0, (sg.s, a));
        }
        if i == n - 1 || kinds[i + 1].is_none() {
            pts.push((sg.e, z));
        }
        for (t, vec) in pts {
            for (j, value) in vec.iter().enumerate() {
                tr.add(Tr::Fm(j), t, value * if j < 5 { fshift } else { 1.0 });
            }
        }
    }

    let breath = v.breath;
    for (i, sg) in segs.iter().enumerate() {
        let (s, e) = (sg.s, sg.e);
        let d = e - s;
        let prv_pause = i == 0 || kinds[i - 1].is_none();
        let nxt_pause = i == n - 1 || kinds[i + 1].is_none();
        let Some(p) = sg.ph else {
            let r = f64::min(0.005, d / 3.0);
            for nm in [Tr::Av, Tr::Ah, Tr::Af, Tr::Avb, Tr::B1add] {
                tr.add(nm, s + r, 0.0);
                tr.add(nm, e - r, 0.0);
            }
            continue;
        };
        let info = ph(p);
        let k = info.kind;
        let r = f64::min(0.006, d / 4.0);
        let fnz = if k == Kind::Nasal {
            info.fnz * fshift
        } else {
            FNP * fshift
        };
        let rz = f64::min(0.008, d / 3.0);
        tr.add(Tr::Fnz, s + rz, fnz);
        tr.add(Tr::Fnz, e - rz, fnz);
        if k.sonorant() || matches!(k, Kind::Nasal | Kind::Tap | Kind::Trill) {
            let av = if k.vowelish() {
                if sg.st > 0 {
                    1.0
                } else {
                    0.78
                }
            } else {
                match k {
                    Kind::Nasal => 0.8,
                    Kind::Liquid => 0.85,
                    Kind::Glide => 0.88,
                    _ => 0.85,
                }
            };
            let head = f64::min(sg.head, 0.6 * d);
            if head > 0.0 {
                let asp = sg.head_asp;
                tr.add(Tr::Av, s, 0.0);
                tr.add(Tr::Av, s + head, 0.0);
                tr.add(Tr::Av, s + head + 0.007, av);
                tr.add(Tr::Ah, s, asp);
                tr.add(Tr::Ah, s + head, asp * 0.6);
                tr.add(Tr::Ah, s + head + 0.012, 0.0);
                tr.add(Tr::B1add, s, 220.0);
                tr.add(Tr::B1add, s + head, 150.0);
                tr.add(Tr::B1add, s + head + 0.02, 0.0);
            } else {
                let on = if prv_pause {
                    f64::min(0.03, d / 3.0)
                } else {
                    r
                };
                if prv_pause {
                    tr.add(Tr::Av, s, 0.0);
                }
                tr.add(Tr::Av, s + on, av);
                tr.add(Tr::Ah, s + on, 0.0);
                tr.add(Tr::B1add, s + on, 0.0);
            }
            if nxt_pause {
                let off = f64::min(0.09, d * 0.45);
                tr.add(Tr::Av, e - off, av);
                tr.add(Tr::Av, e - off * 0.35, av * 0.35);
                tr.add(Tr::Av, e, 0.0);
                tr.add(Tr::Ah, e - off, 0.0);
                tr.add(Tr::Ah, e - off * 0.3, 0.05 + breath * 0.3);
                tr.add(Tr::Ah, e, 0.0);
                tr.add(Tr::B1add, e - off, 0.0);
                tr.add(Tr::B1add, e, 150.0);
            } else {
                tr.add(Tr::Av, e - r, av);
                tr.add(Tr::Ah, e - r, 0.0);
                tr.add(Tr::B1add, e - r, 0.0);
            }
            for nm in [Tr::Af, Tr::Avb] {
                tr.add(nm, s + r, 0.0);
                tr.add(nm, e - r, 0.0);
            }
            if k == Kind::Tap {
                tr.add(Tr::Av, s + d * 0.5, av * 0.25);
            }
            if k == Kind::Trill {
                for j in 1..6 {
                    tr.add(
                        Tr::Av,
                        s + d * j as f64 / 6.0,
                        av * if j % 2 == 1 { 0.25 } else { 1.0 },
                    );
                }
            }
            continue;
        }

        let mut fl: Option<Spectrum> = None;
        match k {
            Kind::Stop => {
                let burst = sg.burst;
                let rel = e - burst - sg.post;
                let voiced = info.voiced;
                let ba = 10f64.powf((burst_db(info.place) - if voiced { 3.0 } else { 0.0 }) / 20.0)
                    * sg.burst_amp;
                tr.add(Tr::Av, s + r, 0.0);
                tr.add(Tr::Av, e - 0.001, 0.0);
                tr.add(Tr::Ah, s + r, 0.0);
                tr.add(Tr::B1add, s + r, 0.0);
                if sg.post > 0.0 {
                    tr.add(Tr::Ah, rel + burst, 0.0);
                    tr.add(Tr::Ah, rel + burst + 0.004, if voiced { 0.2 } else { 0.55 });
                    tr.add(Tr::Ah, e - 0.002, 0.0);
                    tr.add(Tr::B1add, rel + burst, 200.0);
                } else {
                    tr.add(Tr::Ah, e - 0.001, 0.0);
                }
                if voiced {
                    let vb = if prv_pause { 0.5 } else { 1.0 };
                    tr.add(Tr::Avb, s + f64::min(0.012, d / 4.0), vb);
                    tr.add(Tr::Avb, rel - 0.002, vb);
                    tr.add(Tr::Avb, rel + 0.004, 0.0);
                } else {
                    tr.add(Tr::Avb, s + r, 0.0);
                }
                tr.add(Tr::Avb, e, 0.0);
                tr.add(Tr::Af, s + r, 0.0);
                tr.add(Tr::Af, rel - 0.0004, 0.0);
                tr.add(Tr::Af, rel, ba);
                tr.add(Tr::Af, rel + burst, ba * 0.35);
                tr.add(Tr::Af, rel + burst + 0.004, 0.0);
                fl = Some(if info.place == Place::Vel {
                    let vt = nearest_vowel(segs, i);
                    let fv = (vt.map_or(1600.0, |v| v[1]) * 1.05 + 100.0).clamp(1500.0, 3000.0);
                    (
                        [
                            (fv, 450.0, 1.0),
                            (fv * 1.45, 900.0, 0.35),
                            (5000.0, 3000.0, 0.1),
                        ],
                        0.05,
                    )
                } else {
                    burst_spec(info.place)
                });
            }
            Kind::Affricate => {
                let c = 0.42 * d;
                let a = 10f64.powf(fric_db(info.name) / 20.0);
                tr.add(Tr::Av, s + r, 0.0);
                tr.add(Tr::Ah, s + r, 0.0);
                tr.add(Tr::B1add, s + r, 0.0);
                tr.add(Tr::Af, s + r, 0.0);
                tr.add(Tr::Af, s + c - 0.0004, 0.0);
                tr.add(Tr::Af, s + c, a * 1.2);
                tr.add(Tr::Af, e - f64::min(0.015, d / 4.0), a);
                if info.voiced {
                    tr.add(Tr::Avb, s + f64::min(0.012, d / 4.0), 0.8);
                    tr.add(Tr::Avb, s + c, 0.6);
                    tr.add(Tr::Av, s + c, 0.0);
                    tr.add(Tr::Av, e - f64::min(0.02, d / 4.0), 0.35);
                    tr.add(Tr::Avb, e - r, 0.3);
                } else {
                    tr.add(Tr::Av, e - 0.002, 0.0);
                    tr.add(Tr::Avb, s + r, 0.0);
                    tr.add(Tr::Avb, e - r, 0.0);
                }
                if nxt_pause {
                    tr.add(Tr::Af, e, 0.0);
                }
                fl = fric_spec(info.name);
            }
            Kind::Aspirate => {
                let ro = f64::min(0.012, d / 3.0);
                tr.add(Tr::Av, s + ro, 0.0);
                tr.add(Tr::Av, e - 0.002, 0.0);
                tr.add(Tr::Ah, s + f64::min(0.02, d / 3.0), 1.0);
                tr.add(Tr::Ah, e - 0.002, 0.85);
                tr.add(Tr::B1add, s + ro, 150.0);
                tr.add(Tr::B1add, e, 120.0);
                for nm in [Tr::Af, Tr::Avb] {
                    tr.add(nm, s + r, 0.0);
                    tr.add(nm, e - r, 0.0);
                }
            }
            _ => {
                // fricatives
                let rise = f64::min(0.02, d / 3.0);
                let a = 10f64.powf(fric_db(info.name) / 20.0);
                let ro = f64::min(0.012, d / 3.0);
                if info.voiced {
                    tr.add(Tr::Av, s + rise, 0.45);
                    tr.add(Tr::Av, e - rise, 0.45);
                    tr.add(Tr::Avb, s + rise, 0.6);
                    tr.add(Tr::Avb, e - rise, 0.6);
                } else {
                    tr.add(Tr::Av, s + ro, 0.0);
                    tr.add(Tr::Av, e - ro, 0.0);
                    tr.add(Tr::Avb, s + r, 0.0);
                    tr.add(Tr::Avb, e - r, 0.0);
                }
                tr.add(Tr::Af, s + rise, a);
                tr.add(Tr::Af, e - rise, a);
                if prv_pause {
                    tr.add(Tr::Af, s, 0.0);
                }
                if nxt_pause {
                    tr.add(Tr::Af, e, 0.0);
                }
                tr.add(Tr::Ah, s + rise, 0.0);
                tr.add(Tr::Ah, e - rise, 0.0);
                tr.add(Tr::B1add, s + r, 0.0);
                tr.add(Tr::B1add, e - r, 0.0);
                fl = fric_spec(info.name);
            }
        }
        if let Some((spec, byp)) = fl {
            let fsh = fshift.powf(0.7);
            for t in [s + 0.002, e - 0.002] {
                for (j, (fc, bw, g)) in spec.iter().enumerate() {
                    tr.add(Tr::Fr(j, 0), t, fc * fsh);
                    tr.add(Tr::Fr(j, 1), t, bw * fsh);
                    tr.add(Tr::Fr(j, 2), t, *g);
                }
                tr.add(Tr::FrByp, t, byp);
            }
        }
    }
    tr
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Statement,
    Question,
    Exclaim,
    Continue,
}

fn pitch_points(segs: &[Seg], phrases: &[Phrase], v: &Voice) -> Vec<(f64, f64)> {
    let base = v.pitch;
    let emphasis = 1.0f64;
    if v.flat {
        return vec![(0.0, base), (segs.last().map_or(0.0, |s| s.e) + 1.0, base)];
    }
    let mut pts = Vec::new();
    let nph = phrases.len();
    for (pi, phr) in phrases.iter().enumerate() {
        let idx: Vec<usize> = (0..segs.len())
            .filter(|&j| segs[j].ph.is_some() && segs[j].phrase == pi)
            .collect();
        let (Some(&first), Some(&last)) = (idx.first(), idx.last()) else {
            continue;
        };
        let (ps, pe) = (segs[first].s, segs[last].e);
        let span = (pe - ps).max(1e-3);
        let mut mode = match phr.punct {
            '?' => Mode::Question,
            '!' => Mode::Exclaim,
            _ => Mode::Statement,
        };
        if pi < nph - 1 && phr.punct == ',' && mode == Mode::Statement {
            mode = Mode::Continue;
        }
        let top = if mode == Mode::Exclaim { 1.12 } else { 1.06 };
        let drop = if mode == Mode::Exclaim { 0.2 } else { 0.14 };
        let decl = |t: f64| base * (top - drop * (t - ps) / span);
        let acc_scale = emphasis * if mode == Mode::Exclaim { 1.8 } else { 1.0 };
        let vows: Vec<usize> = idx
            .iter()
            .copied()
            .filter(|&j| segs[j].vowelish())
            .collect();
        let mut stressed: Vec<usize> = vows.iter().copied().filter(|&j| segs[j].st >= 1).collect();
        if stressed.is_empty() {
            stressed = vows.last().copied().into_iter().collect();
        }
        let nuclear = stressed.last().copied();
        pts.push((ps, decl(ps) * 0.97));
        for &j in &vows {
            if Some(j) == nuclear {
                break;
            }
            let s = segs[j].s + segs[j].head;
            let e = segs[j].e;
            let d = e - s;
            let acc = match segs[j].st {
                1 => (if j == stressed[0] { 0.16 } else { 0.1 }) * acc_scale,
                2 => 0.06 * acc_scale,
                _ => -0.02,
            };
            pts.push((s + 0.3 * d, decl(s + 0.3 * d) * (1.0 + acc)));
            pts.push((e, decl(e) * (1.0 + acc * 0.6)));
        }
        if let Some(j) = nuclear {
            let s = segs[j].s + segs[j].head;
            let d = segs[j].e - s;
            match mode {
                Mode::Question => {
                    pts.push((s + 0.25 * d, decl(s) * 0.95));
                    pts.push((pe, base * (1.0 + 0.45 * emphasis.max(0.3))));
                }
                Mode::Continue => {
                    pts.push((s + 0.3 * d, decl(s) * (1.0 + 0.1 * acc_scale)));
                    pts.push((pe, decl(pe) * 1.04));
                }
                _ => {
                    let acc = 0.13 * acc_scale;
                    pts.push((s + 0.2 * d, decl(s) * (1.0 + acc)));
                    let end = if mode == Mode::Exclaim { 0.72 } else { 0.8 };
                    pts.push((
                        pe,
                        base * (end + (1.0 - end) * (1.0 - emphasis.min(1.5) / 1.5) * 0.5),
                    ));
                }
            }
        }
    }
    pts
}

// ---------------------------------------------------------------------------
// DSP
// ---------------------------------------------------------------------------

/// A small seeded generator (SplitMix64) for noise, jitter and shimmer.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn uniform(&mut self) -> f64 {
        self.unit() * 2.0 - 1.0
    }

    fn normal(&mut self) -> f64 {
        let u1 = self.unit().max(1e-300);
        let u2 = self.unit();
        (-2.0 * u1.ln()).sqrt() * (2.0 * PI * u2).cos()
    }
}

#[derive(Clone, Copy)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 3],
}

/// Klatt resonator, unity DC gain.
fn reson(f: f64, bw: f64, sr: f64) -> Biquad {
    let f = f.min(0.47 * sr);
    let bw = bw.max(10.0);
    let t = 1.0 / sr;
    let c = -(-2.0 * PI * bw * t).exp();
    let b = 2.0 * (-PI * bw * t).exp() * (2.0 * PI * f * t).cos();
    let a = 1.0 - b - c;
    Biquad {
        b: [a, 0.0, 0.0],
        a: [1.0, -b, -c],
    }
}

fn anti(f: f64, bw: f64, sr: f64) -> Biquad {
    let r = reson(f, bw, sr);
    let (a, b, c) = (r.b[0], -r.a[1], -r.a[2]);
    Biquad {
        b: [1.0 / a, -b / a, -c / a],
        a: [1.0, 0.0, 0.0],
    }
}

/// Constant 0 dB peak band-pass (RBJ).
fn bandpass(f: f64, bw: f64, sr: f64) -> Biquad {
    let f = f.clamp(20.0, 0.46 * sr);
    let q = (f / bw.max(20.0)).max(0.3);
    let w0 = 2.0 * PI * f / sr;
    let alpha = w0.sin() / (2.0 * q);
    let a0 = 1.0 + alpha;
    Biquad {
        b: [alpha / a0, 0.0, -alpha / a0],
        a: [1.0, -2.0 * w0.cos() / a0, (1.0 - alpha) / a0],
    }
}

#[derive(Clone, Copy)]
enum Rbj {
    Lowpass,
    Highpass,
    HighShelf(f64),
}

fn rbj(kind: Rbj, fc: f64, sr: f64) -> Biquad {
    let q = 0.707;
    let fc = fc.clamp(10.0, sr * 0.49);
    let w0 = 2.0 * PI * fc / sr;
    let (cw, sw) = (w0.cos(), w0.sin());
    let alpha = sw / (2.0 * q);
    let (b, a) = match kind {
        Rbj::Lowpass => (
            [(1.0 - cw) / 2.0, 1.0 - cw, (1.0 - cw) / 2.0],
            [1.0 + alpha, -2.0 * cw, 1.0 - alpha],
        ),
        Rbj::Highpass => (
            [(1.0 + cw) / 2.0, -(1.0 + cw), (1.0 + cw) / 2.0],
            [1.0 + alpha, -2.0 * cw, 1.0 - alpha],
        ),
        Rbj::HighShelf(gain_db) => {
            let a = 10f64.powf(gain_db / 40.0);
            let sq = 2.0 * a.sqrt() * alpha;
            (
                [
                    a * ((a + 1.0) + (a - 1.0) * cw + sq),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cw),
                    a * ((a + 1.0) + (a - 1.0) * cw - sq),
                ],
                [
                    (a + 1.0) - (a - 1.0) * cw + sq,
                    2.0 * ((a - 1.0) - (a + 1.0) * cw),
                    (a + 1.0) - (a - 1.0) * cw - sq,
                ],
            )
        }
    };
    Biquad {
        b: [b[0] / a[0], b[1] / a[0], b[2] / a[0]],
        a: [1.0, a[1] / a[0], a[2] / a[0]],
    }
}

/// A static biquad over the whole signal, `order` times.
fn filter(x: &mut [f64], q: Biquad, order: usize) {
    for _ in 0..order.max(1) {
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        for s in x.iter_mut() {
            let x0 = *s;
            let y0 = q.b[0] * x0 + q.b[1] * x1 + q.b[2] * x2 - q.a[1] * y1 - q.a[2] * y2;
            x2 = x1;
            x1 = x0;
            y2 = y1;
            y1 = y0;
            *s = y0;
        }
    }
}

/// A time-varying biquad, one coefficient set per block, direct form I so a
/// coefficient change never jumps.
fn tv_filter(x: &[f64], coefs: &[Biquad], blk: usize) -> Vec<f64> {
    let mut y = vec![0.0; x.len()];
    let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
    for (i, (xi, yi)) in x.iter().zip(y.iter_mut()).enumerate() {
        let q = &coefs[(i / blk).min(coefs.len() - 1)];
        let y0 = q.b[0] * xi + q.b[1] * x1 + q.b[2] * x2 - q.a[1] * y1 - q.a[2] * y2;
        x2 = x1;
        x1 = *xi;
        y2 = y1;
        y1 = y0;
        *yi = y0;
    }
    y
}

fn smooth(x: &[f64], width: f64) -> Vec<f64> {
    let width = width.max(1.0) as usize;
    if width <= 1 || x.is_empty() {
        return x.to_vec();
    }
    let pad = width / 2;
    let first = x[0];
    let last = x[x.len() - 1];
    let get = |i: isize| -> f64 {
        if i < 0 {
            first
        } else if i as usize >= x.len() {
            last
        } else {
            x[i as usize]
        }
    };
    let mut out = Vec::with_capacity(x.len());
    let mut sum: f64 = (0..width).map(|k| get(k as isize - pad as isize)).sum();
    for i in 0..x.len() {
        out.push(sum / width as f64);
        sum += get(i as isize + width as isize - pad as isize) - get(i as isize - pad as isize);
    }
    out
}

fn polyblep(t: f64, dt: f64) -> f64 {
    if t < dt {
        let tt = t / dt;
        tt + tt - tt * tt - 1.0
    } else if t > 1.0 - dt {
        let tt = (t - 1.0) / dt;
        tt * tt + tt + tt + 1.0
    } else {
        0.0
    }
}

/// Praat local jitter = sqrt(pi)/2 times the per-cycle standard deviation.
const PRAAT_LOCAL: f64 = 0.886_226_925_452_758;

/// Scale every glottal cycle of `f0` by its own factor 1 + d, d normal, so
/// the periods measure a Praat local jitter of `jitter`.
fn cycle_jitter(f0: &mut [f64], jitter: f64, rng: &mut Rng, sr: f64) {
    let sd = jitter * PRAAT_LOCAL;
    let mut ph = 0.0;
    let mut edge = 0.0;
    let mut d = (rng.normal() * sd).clamp(-3.0 * sd, 3.0 * sd);
    for f in f0.iter_mut() {
        let base = *f;
        ph += base / sr;
        while ph >= edge + 1.0 / (1.0 + d) {
            edge += 1.0 / (1.0 + d);
            d = (rng.normal() * sd).clamp(-3.0 * sd, 3.0 * sd);
        }
        *f = base * (1.0 + d);
    }
}

pub fn render(phrases: &[Phrase], v: &Voice, sr: f64, rate: f64) -> Vec<f32> {
    let mut segs = build_segments(phrases, rate);
    while segs.last().is_some_and(|s| s.ph.is_none()) {
        segs.pop();
    }
    if segs.is_empty() {
        return Vec::new();
    }
    let mut tr = build_tracks(&segs, v);
    let mut vs = v.clone();
    if phrases.last().is_some_and(|p| p.punct == '!') {
        vs.oq = f64::max(0.45, vs.oq - 0.08);
    }
    for (t, f) in pitch_points(&segs, phrases, &vs) {
        tr.add(Tr::F0, t, f);
    }
    synth(&segs, &tr, &vs, sr)
}

fn synth(segs: &[Seg], tr: &Tracks, vs: &Voice, sr: f64) -> Vec<f32> {
    let mut rng = Rng(1000);
    let total = segs.last().map_or(0.0, |s| s.e) + 0.12;
    let n = ((total * sr).round() as usize).max(1);
    let blk = ((sr / 1400.0).round() as usize).max(8);
    let nb = n.div_ceil(blk);
    let tc: Vec<f64> = (0..nb)
        .map(|k| (k as f64 + 0.5) * blk as f64 / sr)
        .collect();
    let ts: Vec<f64> = (0..n).map(|k| k as f64 / sr).collect();
    let up = |ctl: &[f64]| interp(&ts, &tc, ctl);

    // ---- source
    let f0c = smooth(&tr.curve(Tr::F0, &tc, vs.pitch), 0.045 * sr / blk as f64);
    let mut f0 = up(&f0c);
    if vs.jitter > 0.0 {
        cycle_jitter(&mut f0, vs.jitter, &mut rng, sr);
        for (f, t) in f0.iter_mut().zip(&ts) {
            *f *= 1.0 + 0.004 * ((2.0 * PI * 4.7 * t).sin() + (2.0 * PI * 7.1 * t + 1.0).sin());
        }
    }
    for f in f0.iter_mut() {
        *f = f.clamp(40.0, 0.2 * sr);
    }
    let oq = vs.oq;
    let tp = oq * 0.66;
    let tn = oq - tp;
    let jj = PI / (2.0 * tn);
    let mut src = vec![0.0; n];
    let mut flow = vec![0.0; n];
    let mut ph_acc = 0.0;
    let mut cycle = 0i64;
    let mut shimmer_gain = 1.0 + vs.shimmer * PRAAT_LOCAL * rng.normal();
    for i in 0..n {
        ph_acc += f0[i] / sr;
        let t = ph_acc.fract();
        let dt = (f0[i] / sr).max(1e-6);
        let mut dflow = 0.0;
        if t < tp {
            dflow = (PI / (2.0 * tp)) * (PI * t / tp).sin();
            flow[i] = 0.5 * (1.0 - (PI * t / tp).cos());
        } else if t < oq {
            dflow = -jj * (0.5 * PI * (t - tp) / tn).sin();
            flow[i] = (0.5 * PI * (t - tp) / tn).cos();
        }
        dflow += (jj / 2.0) * polyblep((t - oq).rem_euclid(1.0), dt);
        let mut s = dflow / jj;
        if vs.shimmer > 0.0 {
            let c = ph_acc.floor() as i64;
            while cycle < c {
                cycle += 1;
                shimmer_gain = 1.0 + vs.shimmer * PRAAT_LOCAL * rng.normal();
            }
            s *= shimmer_gain;
        }
        src[i] = s;
    }
    if vs.tilt > 0.0 {
        let a = (-2.0 * PI * vs.tilt / sr).exp();
        let mut y1 = 0.0;
        for s in src.iter_mut() {
            y1 = (1.0 - a) * *s + a * y1;
            *s = y1;
        }
        let rms = (src.iter().map(|s| s * s).sum::<f64>() / n as f64).sqrt() + 1e-9;
        for s in src.iter_mut() {
            *s = *s / rms * 0.35;
        }
    }

    let av = up(&tr.curve(Tr::Av, &tc, 0.0));
    let ah = up(&tr.curve(Tr::Ah, &tc, 0.0));
    let af = up(&tr.curve(Tr::Af, &tc, 0.0));
    let avb = up(&tr.curve(Tr::Avb, &tc, 0.0));

    let mut prev = 0.0;
    let k_asp = 0.16;
    let breath = vs.breath;
    let mut exc = vec![0.0; n];
    for i in 0..n {
        let raw = rng.uniform();
        let asp = 0.5 * raw + 0.5 * prev;
        prev = raw;
        exc[i] =
            av[i] * src[i] + (ah[i] * k_asp + breath * k_asp * 0.7 * av[i] * (0.3 + flow[i])) * asp;
    }

    // ---- cascade (nasal pole/zero + F5..F1)
    let defaults = [
        500.0, 1500.0, 2500.0, F4, F5, 60.0, 90.0, 150.0, 250.0, 320.0,
    ];
    let fw = 0.008 * sr / blk as f64;
    let fm: Vec<Vec<f64>> = (0..10)
        .map(|j| smooth(&tr.curve(Tr::Fm(j), &tc, defaults[j]), fw))
        .collect();
    let b1add = tr.curve(Tr::B1add, &tc, 0.0);
    let fnz = tr.curve(Tr::Fnz, &tc, FNP);
    let fshift = vs.formant_shift;
    let pole = vec![reson(FNP * fshift, 100.0, sr); nb];
    let mut y = tv_filter(&exc, &pole, blk);
    let zero: Vec<Biquad> = fnz.iter().map(|&f| anti(f, 100.0, sr)).collect();
    y = tv_filter(&y, &zero, blk);
    for j in (0..5).rev() {
        let coefs: Vec<Biquad> = (0..nb)
            .map(|b| {
                let bw = fm[5 + j][b] * vs.bw + if j == 0 { b1add[b] } else { 0.0 };
                reson(fm[j][b], bw, sr)
            })
            .collect();
        y = tv_filter(&y, &coefs, blk);
    }
    for (fh, bh) in [(4900.0, 600.0), (5900.0, 800.0), (7000.0, 1000.0)] {
        let fh = fh * fshift;
        if fh < 0.42 * sr {
            filter(&mut y, reson(fh, bh, sr), 1);
        }
    }
    filter(&mut y, rbj(Rbj::HighShelf(12.0), 3000.0 * fshift, sr), 1);

    // reference level: RMS inside stressed vowels
    let mut mask = vec![false; n];
    for sg in segs {
        if sg.vowelish() && sg.st >= 1 {
            let s0 = ((sg.s + 0.3 * (sg.e - sg.s)) * sr) as usize;
            let e0 = (((sg.e - 0.3 * (sg.e - sg.s)) * sr) as usize).min(n);
            for m in mask.iter_mut().take(e0).skip(s0) {
                *m = true;
            }
        }
    }
    if !mask.iter().any(|&m| m) {
        for (m, a) in mask.iter_mut().zip(&av) {
            *m = *a > 0.5;
        }
    }
    let rms_of = |sel: &dyn Fn(usize) -> bool| -> f64 {
        let (mut sum, mut cnt) = (0.0, 0usize);
        for (i, v) in y.iter().enumerate() {
            if sel(i) {
                sum += v * v;
                cnt += 1;
            }
        }
        if cnt == 0 {
            0.0
        } else {
            (sum / cnt as f64).sqrt()
        }
    };
    let reference = if mask.iter().any(|&m| m) {
        rms_of(&|i| mask[i])
    } else {
        rms_of(&|_| true)
    }
    .max(1e-6);

    let mut out = y;
    // ---- frication branch (parallel)
    if af.iter().any(|&a| a > 0.0) {
        let mut noise = vec![0.0; n];
        for i in 0..n {
            let vm = av[i].clamp(0.0, 1.0);
            noise[i] = rng.uniform() * 3f64.sqrt() * (1.0 - 0.6 * vm + 1.2 * vm * flow[i]);
        }
        let mut fr = vec![0.0; n];
        let mut power = vec![0.0; nb];
        for j in 0..3 {
            let fc = tr.curve(Tr::Fr(j, 0), &tc, 5000.0);
            let bw = tr.curve(Tr::Fr(j, 1), &tc, 2000.0);
            let g = tr.curve(Tr::Fr(j, 2), &tc, 0.0);
            let coefs: Vec<Biquad> = (0..nb).map(|b| bandpass(fc[b], bw[b], sr)).collect();
            let band = tv_filter(&noise, &coefs, blk);
            let gu = up(&g);
            for i in 0..n {
                fr[i] += band[i] * gu[i];
            }
            for b in 0..nb {
                power[b] += g[b] * g[b] * (PI / 2.0) * bw[b] / (sr / 2.0);
            }
        }
        let byp = tr.curve(Tr::FrByp, &tc, 0.0);
        let bypu = up(&byp);
        for b in 0..nb {
            power[b] += byp[b] * byp[b];
        }
        let norm = up(&power.iter().map(|p| p.max(1e-4).sqrt()).collect::<Vec<_>>());
        for i in 0..n {
            fr[i] = (fr[i] + noise[i] * bypu[i]) / norm[i];
        }
        filter(&mut fr, rbj(Rbj::Highpass, 700.0, sr), 1);
        filter(
            &mut fr,
            rbj(Rbj::Lowpass, f64::min(12000.0, 0.45 * sr), sr),
            1,
        );
        for i in 0..n {
            out[i] += fr[i] * af[i] * reference;
        }
    }

    // ---- voice bar
    if avb.iter().any(|&a| a > 0.0) {
        let mean = flow.iter().sum::<f64>() / n as f64;
        let mut vb: Vec<f64> = flow.iter().map(|f| f - mean).collect();
        filter(&mut vb, rbj(Rbj::Lowpass, 350.0 * fshift, sr), 2);
        let rms = (vb.iter().map(|v| v * v).sum::<f64>() / n as f64).sqrt() + 1e-9;
        let g = reference * 10f64.powf(-13.0 / 20.0) / rms;
        for i in 0..n {
            out[i] += vb[i] * avb[i] * g;
        }
    }

    if vs.ring > 0.0 {
        let m = vs.ring;
        for (o, t) in out.iter_mut().zip(&ts) {
            *o = *o * (1.0 - m) + *o * (2.0 * PI * vs.ring_hz * t).sin() * m * 1.5;
        }
    }
    filter(&mut out, rbj(Rbj::Highpass, 55.0, sr), 2);
    let peak = out.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let scale = if peak > 1e-9 { 0.89 / peak } else { 1.0 };
    out.iter().map(|v| (v * scale) as f32).collect()
}
