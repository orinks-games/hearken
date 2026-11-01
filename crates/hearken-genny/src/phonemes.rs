//! The phoneme inventory and the tables the synth reads, as in genny's
//! `speech.py`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Vowel,
    Diphthong,
    Stop,
    Fricative,
    Affricate,
    Nasal,
    Liquid,
    Glide,
    Aspirate,
    Tap,
    Trill,
}

impl Kind {
    pub fn vowelish(self) -> bool {
        matches!(self, Kind::Vowel | Kind::Diphthong)
    }

    pub fn sonorant(self) -> bool {
        matches!(
            self,
            Kind::Vowel | Kind::Diphthong | Kind::Liquid | Kind::Glide
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    None,
    Lab,
    Labd,
    Dent,
    Alv,
    Post,
    Vel,
    Glot,
}

#[derive(Clone, Copy, Debug)]
pub struct Phoneme {
    pub name: &'static str,
    pub kind: Kind,
    /// F1-F3 at the start (vowels, liquids, glides).
    pub f: [f64; 3],
    /// F1-F3 at the end; the same as `f` when the source gives none.
    pub f2: [f64; 3],
    /// Inherent and minimum duration, ms.
    pub dur: (f64, f64),
    pub voiced: bool,
    pub place: Place,
    /// Nasal zero during the murmur (nasals only; 0 elsewhere).
    pub fnz: f64,
}

pub const F4: f64 = 3400.0;
pub const F5: f64 = 4200.0;
/// Nasal pole, cancelled by the nasal zero outside nasals.
pub const FNP: f64 = 270.0;

const Z3: [f64; 3] = [0.0, 0.0, 0.0];

const fn vowel(
    name: &'static str,
    kind: Kind,
    f: [f64; 3],
    f2: [f64; 3],
    dur: (f64, f64),
) -> Phoneme {
    Phoneme {
        name,
        kind,
        f,
        f2,
        dur,
        voiced: true,
        place: Place::None,
        fnz: 0.0,
    }
}

const fn cons(
    name: &'static str,
    kind: Kind,
    voiced: bool,
    place: Place,
    dur: (f64, f64),
) -> Phoneme {
    Phoneme {
        name,
        kind,
        f: Z3,
        f2: Z3,
        dur,
        voiced,
        place,
        fnz: 0.0,
    }
}

const fn nasal(name: &'static str, place: Place, dur: (f64, f64), fnz: f64) -> Phoneme {
    Phoneme {
        name,
        kind: Kind::Nasal,
        f: Z3,
        f2: Z3,
        dur,
        voiced: true,
        place,
        fnz,
    }
}

const fn sonor(name: &'static str, kind: Kind, f: [f64; 3], dur: (f64, f64)) -> Phoneme {
    vowel(name, kind, f, f, dur)
}

use Kind::*;
use Place as P;

/// English vowels: men's means of Hillenbrand et al. 1995 at 20 % (`f`) and
/// 50 % (`f2`) of the vowel; AX, IX, AXR, AY, OY and AW keep Peterson &
/// Barney / Klatt values.
pub static PH: &[Phoneme] = &[
    vowel(
        "IY",
        Vowel,
        [346.0, 2312.0, 3010.0],
        [340.0, 2338.0, 2994.0],
        (155.0, 55.0),
    ),
    vowel(
        "IH",
        Vowel,
        [434.0, 2032.0, 2683.0],
        [459.0, 1941.0, 2641.0],
        (135.0, 40.0),
    ),
    vowel(
        "EH",
        Vowel,
        [584.0, 1817.0, 2603.0],
        [592.0, 1774.0, 2602.0],
        (150.0, 70.0),
    ),
    vowel(
        "AE",
        Vowel,
        [592.0, 1923.0, 2597.0],
        [613.0, 1863.0, 2566.0],
        (230.0, 80.0),
    ),
    vowel(
        "AA",
        Vowel,
        [754.0, 1304.0, 2528.0],
        [757.0, 1326.0, 2523.0],
        (240.0, 100.0),
    ),
    vowel(
        "AO",
        Vowel,
        [654.0, 1020.0, 2516.0],
        [670.0, 1046.0, 2509.0],
        (240.0, 130.0),
    ),
    vowel(
        "UH",
        Vowel,
        [468.0, 1122.0, 2440.0],
        [483.0, 1208.0, 2438.0],
        (160.0, 60.0),
    ),
    vowel(
        "UW",
        Diphthong,
        [385.0, 1015.0, 2346.0],
        [375.0, 971.0, 2359.0],
        (210.0, 70.0),
    ),
    vowel(
        "AH",
        Vowel,
        [622.0, 1181.0, 2548.0],
        [618.0, 1243.0, 2549.0],
        (140.0, 60.0),
    ),
    sonor("AX", Vowel, [500.0, 1400.0, 2400.0], (120.0, 55.0)),
    sonor("IX", Vowel, [420.0, 1800.0, 2500.0], (110.0, 40.0)),
    vowel(
        "ER",
        Vowel,
        [485.0, 1371.0, 1720.0],
        [460.0, 1406.0, 1704.0],
        (180.0, 80.0),
    ),
    sonor("AXR", Vowel, [490.0, 1350.0, 1690.0], (120.0, 60.0)),
    vowel(
        "EY",
        Diphthong,
        [479.0, 2089.0, 2699.0],
        [400.0, 2229.0, 2742.0],
        (190.0, 100.0),
    ),
    vowel(
        "AY",
        Diphthong,
        [700.0, 1220.0, 2600.0],
        [400.0, 1950.0, 2600.0],
        (250.0, 150.0),
    ),
    vowel(
        "OY",
        Diphthong,
        [550.0, 960.0, 2400.0],
        [380.0, 1850.0, 2500.0],
        (280.0, 150.0),
    ),
    vowel(
        "AW",
        Diphthong,
        [720.0, 1240.0, 2500.0],
        [450.0, 1000.0, 2350.0],
        (260.0, 100.0),
    ),
    vowel(
        "OW",
        Diphthong,
        [511.0, 936.0, 2455.0],
        [435.0, 898.0, 2448.0],
        (220.0, 80.0),
    ),
    cons("P", Stop, false, P::Lab, (90.0, 50.0)),
    cons("B", Stop, true, P::Lab, (85.0, 60.0)),
    cons("T", Stop, false, P::Alv, (75.0, 50.0)),
    cons("D", Stop, true, P::Alv, (75.0, 50.0)),
    cons("K", Stop, false, P::Vel, (80.0, 60.0)),
    cons("G", Stop, true, P::Vel, (80.0, 60.0)),
    cons("F", Fricative, false, P::Labd, (100.0, 80.0)),
    cons("V", Fricative, true, P::Labd, (60.0, 40.0)),
    cons("TH", Fricative, false, P::Dent, (90.0, 60.0)),
    cons("DH", Fricative, true, P::Dent, (50.0, 30.0)),
    cons("S", Fricative, false, P::Alv, (105.0, 60.0)),
    cons("Z", Fricative, true, P::Alv, (75.0, 40.0)),
    cons("SH", Fricative, false, P::Post, (105.0, 80.0)),
    cons("ZH", Fricative, true, P::Post, (70.0, 40.0)),
    cons("HH", Aspirate, false, P::Glot, (80.0, 30.0)),
    cons("CH", Affricate, false, P::Post, (120.0, 80.0)),
    cons("JH", Affricate, true, P::Post, (100.0, 60.0)),
    nasal("M", P::Lab, (70.0, 60.0), 1000.0),
    nasal("N", P::Alv, (60.0, 50.0), 1500.0),
    nasal("NG", P::Vel, (95.0, 60.0), 2200.0),
    sonor("L", Liquid, [330.0, 1050.0, 2800.0], (80.0, 40.0)),
    sonor("R", Liquid, [330.0, 1060.0, 1400.0], (80.0, 30.0)),
    sonor("W", Glide, [290.0, 650.0, 2150.0], (80.0, 60.0)),
    sonor("Y", Glide, [260.0, 2070.0, 3020.0], (80.0, 40.0)),
    cons("DX", Tap, true, P::Alv, (28.0, 22.0)),
    // British (not genny's): rounded LOT, NURSE without r-colouring, and
    // the GOAT glide from a central start. Approximate Southern British
    // means after Deterding 1997.
    vowel(
        "OH",
        Vowel,
        [615.0, 1065.0, 2500.0],
        [620.0, 1080.0, 2500.0],
        (150.0, 70.0),
    ),
    vowel(
        "ERB",
        Vowel,
        [580.0, 1380.0, 2500.0],
        [570.0, 1400.0, 2500.0],
        (190.0, 90.0),
    ),
    vowel(
        "OWB",
        Diphthong,
        [512.0, 1290.0, 2450.0],
        [430.0, 980.0, 2350.0],
        (220.0, 80.0),
    ),
];

/// The index of a phoneme by its ARPAbet name.
pub fn id(name: &str) -> Option<usize> {
    PH.iter().position(|p| p.name == name)
}

pub fn ph(index: usize) -> &'static Phoneme {
    &PH[index]
}

/// Consonant loci (F1, F2, F3).
pub fn locus(place: Place) -> [f64; 3] {
    match place {
        P::Lab => [220.0, 850.0, 2200.0],
        P::Labd => [250.0, 1100.0, 2300.0],
        P::Dent => [260.0, 1400.0, 2650.0],
        P::Alv => [240.0, 1700.0, 2650.0],
        P::Post => [270.0, 1900.0, 2500.0],
        P::Glot => [500.0, 1500.0, 2500.0],
        P::Vel | P::None => [240.0, 1700.0, 2650.0],
    }
}

/// How far F2/F3 lean toward the neighbouring vowel.
pub fn coart(kind: Kind) -> f64 {
    match kind {
        Stop => 0.45,
        Fricative => 0.4,
        Affricate => 0.3,
        Nasal => 0.35,
        Tap => 0.5,
        Trill => 0.4,
        _ => 0.4,
    }
}

/// A noise spectrum: (centre Hz, bandwidth Hz, gain) bands and a flat bypass gain.
pub type Spectrum = ([(f64, f64, f64); 3], f64);

const SIBILANT: Spectrum = (
    [
        (4800.0, 1500.0, 0.55),
        (6500.0, 2000.0, 1.0),
        (8500.0, 3000.0, 0.6),
    ],
    0.0,
);
const SHUSH: Spectrum = (
    [
        (2600.0, 800.0, 1.0),
        (3800.0, 1500.0, 0.6),
        (6000.0, 2500.0, 0.35),
    ],
    0.0,
);
const LABIODENTAL: Spectrum = (
    [
        (2000.0, 3000.0, 0.3),
        (5500.0, 4000.0, 0.5),
        (8500.0, 4000.0, 0.4),
    ],
    0.25,
);
const DENTAL: Spectrum = (
    [
        (2600.0, 3000.0, 0.3),
        (5500.0, 4000.0, 0.5),
        (8000.0, 4000.0, 0.45),
    ],
    0.3,
);

pub fn fric_spec(name: &str) -> Option<Spectrum> {
    Some(match name {
        "S" | "Z" => SIBILANT,
        "SH" | "ZH" | "CH" | "JH" => SHUSH,
        "F" | "V" => LABIODENTAL,
        "TH" | "DH" => DENTAL,
        _ => return None,
    })
}

pub fn burst_spec(place: Place) -> Spectrum {
    match place {
        P::Lab => (
            [
                (1000.0, 1500.0, 0.5),
                (2500.0, 2000.0, 0.35),
                (5000.0, 4000.0, 0.2),
            ],
            0.3,
        ),
        _ => (
            [
                (3800.0, 1500.0, 0.6),
                (5000.0, 2000.0, 0.8),
                (7000.0, 3000.0, 0.4),
            ],
            0.1,
        ),
    }
}

/// Frication level in dB relative to the RMS of a stressed vowel.
pub fn fric_db(name: &str) -> f64 {
    match name {
        "S" => -14.0,
        "Z" => -21.0,
        "SH" => -12.0,
        "ZH" => -19.0,
        "F" => -25.0,
        "V" => -30.0,
        "TH" => -27.0,
        "DH" => -31.0,
        "CH" => -11.0,
        "JH" => -16.0,
        _ => -20.0,
    }
}

pub fn burst_db(place: Place) -> f64 {
    match place {
        P::Lab => -17.0,
        _ => -9.0,
    }
}
