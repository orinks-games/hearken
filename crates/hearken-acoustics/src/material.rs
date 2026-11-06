//! Surface materials: how much of a sound they absorb, scatter and let
//! through, per band.

use crate::bands::{Bands, BAND_COUNT, BAND_EXACT_HZ};
use serde::{Deserialize, Serialize};

/// Longest material name kept; longer names are cut.
pub const MAX_MATERIAL_NAME: usize = 64;

/// A material name as a scene keeps it: trimmed and cut to
/// [`MAX_MATERIAL_NAME`] characters. Lookups by name use the same form.
pub(crate) fn material_name(name: &str) -> &str {
    let name = name.trim();
    match name.char_indices().nth(MAX_MATERIAL_NAME) {
        Some((i, _)) => &name[..i],
        None => name,
    }
}

/// Highest transmission loss used, dB. Real partitions rarely do better than
/// this because sound flanks round them.
pub const MAX_TRANSMISSION_LOSS_DB: f64 = 60.0;

/// How a surface treats sound, per band.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub name: String,
    /// Energy absorption coefficient, 0 (reflects all) to 1 (absorbs all).
    pub absorption: Bands,
    /// Scattering coefficient, 0 (mirror) to 1 (fully diffuse): the share of
    /// reflected energy that leaves in random directions (ISO 17497-1).
    pub scattering: Bands,
    /// Transmission loss through the surface in dB: how much quieter a
    /// sound is on the far side.
    pub transmission_loss_db: Bands,
}

/// A material as written in a file. Give `transmission_loss_db`, or
/// `surface_density_kg_m2` to get it from the mass law, or neither for a
/// surface sound cannot pass through.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaterialDesc {
    pub name: String,
    pub absorption: Bands,
    #[serde(default)]
    pub scattering: Option<Bands>,
    #[serde(default)]
    pub transmission_loss_db: Option<Bands>,
    #[serde(default)]
    pub surface_density_kg_m2: Option<f64>,
}

/// Why a material was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaterialError {
    EmptyName,
    NotFinite,
}

impl core::fmt::Display for MaterialError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MaterialError::EmptyName => write!(f, "material has no name"),
            MaterialError::NotFinite => write!(f, "material has a value that is not a number"),
        }
    }
}

impl Material {
    /// This material with every value clamped into range and the name cut
    /// to [`MAX_MATERIAL_NAME`] characters. Refuses non-finite values.
    pub fn sanitized(&self) -> Result<Material, MaterialError> {
        let name = material_name(&self.name).to_string();
        if name.is_empty() {
            return Err(MaterialError::EmptyName);
        }
        if !(self.absorption.is_finite()
            && self.scattering.is_finite()
            && self.transmission_loss_db.is_finite())
        {
            return Err(MaterialError::NotFinite);
        }
        Ok(Material {
            name,
            absorption: self.absorption.map(|v| v.max(0.0).min(1.0)),
            scattering: self.scattering.map(|v| v.max(0.0).min(1.0)),
            transmission_loss_db: self
                .transmission_loss_db
                .map(|v| v.max(0.0).min(MAX_TRANSMISSION_LOSS_DB)),
        })
    }

    /// Amplitude kept by a specular reflection, per band:
    /// sqrt((1 - absorption) * (1 - scattering)).
    pub fn specular_gain(&self) -> Bands {
        self.absorption.zip(self.scattering, |a, s| {
            ((1.0 - a) * (1.0 - s)).max(0.0).sqrt()
        })
    }

    /// Amplitude that passes through, per band.
    pub fn transmission_gain(&self) -> Bands {
        self.transmission_loss_db.loss_db_to_gain()
    }
}

impl MaterialDesc {
    /// The material this describes, validated.
    pub fn to_material(&self) -> Result<Material, MaterialError> {
        let tl = match (self.transmission_loss_db, self.surface_density_kg_m2) {
            (Some(tl), _) => tl,
            (None, Some(m)) if m.is_finite() => mass_law_tl(m),
            (None, Some(_)) => return Err(MaterialError::NotFinite),
            (None, None) => Bands::splat(MAX_TRANSMISSION_LOSS_DB),
        };
        Material {
            name: self.name.clone(),
            absorption: self.absorption,
            scattering: self.scattering.unwrap_or(SMOOTH),
            transmission_loss_db: tl,
        }
        .sanitized()
    }
}

/// Field-incidence mass law, TL = 20 log10(m f) - 47 dB, with m the surface
/// density in kg/m2 (L. L. Beranek and I. L. Ver, "Noise and Vibration
/// Control Engineering", 1992, ch. 9). It ignores the coincidence dip, so
/// it overstates thin stiff panels a little at high frequencies; the result
/// is clamped to 0..60 dB.
pub fn mass_law_tl(surface_density_kg_m2: f64) -> Bands {
    let m = if surface_density_kg_m2.is_finite() {
        surface_density_kg_m2.max(0.0).min(1.0e5)
    } else {
        0.0
    };
    let mut out = [0.0; BAND_COUNT];
    for (o, f) in out.iter_mut().zip(BAND_EXACT_HZ) {
        let mf = m * f;
        *o = if mf > 0.0 {
            (20.0 * libm::log10(mf) - 47.0)
                .max(0.0)
                .min(MAX_TRANSMISSION_LOSS_DB)
        } else {
            0.0
        };
    }
    Bands(out)
}

// Scattering coefficients are rarely published per material; these are
// estimates in line with the guidance in M. Vorlander, "Auralization"
// (2008), sec. 11.4: about 0.05 for smooth flat surfaces, rising with
// frequency for rough ones.
const SMOOTH: Bands = Bands([0.05, 0.05, 0.05, 0.05, 0.05, 0.05, 0.05]);
const TEXTURED: Bands = Bands([0.05, 0.07, 0.10, 0.15, 0.20, 0.25, 0.30]);
const ROUGH: Bands = Bands([0.10, 0.15, 0.20, 0.30, 0.40, 0.50, 0.60]);
const SOFT: Bands = Bands([0.10, 0.10, 0.15, 0.20, 0.25, 0.30, 0.35]);
const VEGETATION: Bands = Bands([0.20, 0.30, 0.40, 0.50, 0.60, 0.70, 0.70]);

/// Materials that ship with the engine.
///
/// Absorption coefficients for 125 Hz to 4 kHz come from published tables
/// (cited per entry). The tables stop at 4 kHz, so the 8 kHz value repeats
/// the 4 kHz one. Transmission loss is the mass law for a typical thickness
/// (stated per entry) unless noted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Builtin {
    Concrete,
    ConcreteBlock,
    Brick,
    Plaster,
    Drywall,
    Wood,
    Plywood,
    Glass,
    Metal,
    Tile,
    Carpet,
    Curtain,
    AcousticTile,
    Grass,
    Soil,
    Gravel,
    Snow,
    Water,
    Foliage,
    Rock,
}

impl Builtin {
    pub const ALL: [Builtin; 20] = [
        Builtin::Concrete,
        Builtin::ConcreteBlock,
        Builtin::Brick,
        Builtin::Plaster,
        Builtin::Drywall,
        Builtin::Wood,
        Builtin::Plywood,
        Builtin::Glass,
        Builtin::Metal,
        Builtin::Tile,
        Builtin::Carpet,
        Builtin::Curtain,
        Builtin::AcousticTile,
        Builtin::Grass,
        Builtin::Soil,
        Builtin::Gravel,
        Builtin::Snow,
        Builtin::Water,
        Builtin::Foliage,
        Builtin::Rock,
    ];

    /// The name scenes use for it, for example `"concrete_block"`.
    pub fn name(self) -> &'static str {
        match self {
            Builtin::Concrete => "concrete",
            Builtin::ConcreteBlock => "concrete_block",
            Builtin::Brick => "brick",
            Builtin::Plaster => "plaster",
            Builtin::Drywall => "drywall",
            Builtin::Wood => "wood",
            Builtin::Plywood => "plywood",
            Builtin::Glass => "glass",
            Builtin::Metal => "metal",
            Builtin::Tile => "tile",
            Builtin::Carpet => "carpet",
            Builtin::Curtain => "curtain",
            Builtin::AcousticTile => "acoustic_tile",
            Builtin::Grass => "grass",
            Builtin::Soil => "soil",
            Builtin::Gravel => "gravel",
            Builtin::Snow => "snow",
            Builtin::Water => "water",
            Builtin::Foliage => "foliage",
            Builtin::Rock => "rock",
        }
    }

    /// The material's values.
    pub fn material(self) -> Material {
        let (absorption, scattering, tl) = self.values();
        Material {
            name: self.name().to_string(),
            absorption: oct(absorption),
            scattering,
            transmission_loss_db: tl,
        }
    }

    // Sources:
    // [E] F. A. Everest and K. C. Pohlmann, "Master Handbook of Acoustics",
    //     table of sound absorption coefficients of general building
    //     materials, as reproduced by R. Nave, HyperPhysics, "Sound
    //     absorption coefficients" (Georgia State University).
    // [I] ISO 9613-2:1996, Annex A, Table A.1 (attenuation through foliage).
    fn values(self) -> ([f64; 6], Bands, Bands) {
        match self {
            // [E] concrete or terrazzo floor. TL: 150 mm at 2300 kg/m3.
            Builtin::Concrete => (
                [0.01, 0.01, 0.015, 0.02, 0.02, 0.02],
                SMOOTH,
                mass_law_tl(345.0),
            ),
            // [E] concrete block, coarse. TL: 190 mm hollow block, ~200 kg/m2.
            Builtin::ConcreteBlock => (
                [0.36, 0.44, 0.31, 0.29, 0.39, 0.25],
                ROUGH,
                mass_law_tl(200.0),
            ),
            // [E] brick, unglazed. TL: 110 mm single leaf at 1900 kg/m3.
            Builtin::Brick => (
                [0.03, 0.03, 0.03, 0.04, 0.05, 0.07],
                TEXTURED,
                mass_law_tl(209.0),
            ),
            // [E] plaster, gypsum or lime, smooth finish on tile or brick.
            // TL: plastered 110 mm brick, ~230 kg/m2.
            Builtin::Plaster => (
                [0.013, 0.015, 0.02, 0.03, 0.04, 0.05],
                SMOOTH,
                mass_law_tl(230.0),
            ),
            // [E] gypsum board, 1/2 in. nailed to 2x4s 16 in. o.c. TL: one
            // board each side of a stud, ~20 kg/m2.
            Builtin::Drywall => (
                [0.29, 0.10, 0.05, 0.04, 0.07, 0.09],
                SMOOTH,
                mass_law_tl(20.0),
            ),
            // [E] wood floor. TL: 40 mm solid timber at 600 kg/m3.
            Builtin::Wood => (
                [0.15, 0.11, 0.10, 0.07, 0.06, 0.07],
                TEXTURED,
                mass_law_tl(24.0),
            ),
            // [E] plywood panelling, 3/8 in. TL: 10 mm at 600 kg/m3.
            Builtin::Plywood => (
                [0.28, 0.22, 0.17, 0.09, 0.10, 0.11],
                SMOOTH,
                mass_law_tl(6.0),
            ),
            // [E] glass, ordinary window. TL: 6 mm at 2500 kg/m3.
            Builtin::Glass => (
                [0.35, 0.25, 0.18, 0.12, 0.07, 0.04],
                SMOOTH,
                mass_law_tl(15.0),
            ),
            // No entry in [E]; uses [E] marble or glazed tile as the stand-in
            // for a hard non-porous surface. TL: 2 mm steel at 7850 kg/m3.
            Builtin::Metal => (
                [0.01, 0.01, 0.01, 0.01, 0.02, 0.02],
                SMOOTH,
                mass_law_tl(15.7),
            ),
            // [E] marble or glazed tile. TL: tiled 150 mm concrete.
            Builtin::Tile => (
                [0.01, 0.01, 0.01, 0.01, 0.02, 0.02],
                SMOOTH,
                mass_law_tl(345.0),
            ),
            // [E] carpet, heavy, on concrete. TL: the slab beneath.
            Builtin::Carpet => (
                [0.02, 0.06, 0.14, 0.37, 0.60, 0.65],
                SOFT,
                mass_law_tl(345.0),
            ),
            // [E] medium velour, 14 oz/yd2, draped to half area. TL: an
            // estimate for heavy porous fabric (mass law gives almost
            // nothing at 0.5 kg/m2).
            Builtin::Curtain => (
                [0.07, 0.31, 0.49, 0.75, 0.70, 0.60],
                SOFT,
                Bands([1.0, 1.5, 2.0, 3.0, 4.0, 5.0, 6.0]),
            ),
            // [E] acoustic ceiling tile (suspended mineral fibre).
            // TL: ~4 kg/m2.
            Builtin::AcousticTile => ([0.70, 0.66, 0.72, 0.92, 0.88, 0.75], SOFT, mass_law_tl(4.0)),
            // [E] grass. TL: earth, effectively opaque.
            Builtin::Grass => (
                [0.11, 0.26, 0.60, 0.69, 0.92, 0.99],
                VEGETATION,
                Bands::splat(MAX_TRANSMISSION_LOSS_DB),
            ),
            // [E] soil, rough. TL: earth.
            Builtin::Soil => (
                [0.15, 0.25, 0.40, 0.55, 0.60, 0.60],
                ROUGH,
                Bands::splat(MAX_TRANSMISSION_LOSS_DB),
            ),
            // [E] gravel, loose and moist. TL: earth.
            Builtin::Gravel => (
                [0.25, 0.60, 0.65, 0.70, 0.75, 0.80],
                ROUGH,
                Bands::splat(MAX_TRANSMISSION_LOSS_DB),
            ),
            // [E] snow, freshly fallen, 4 in. (100 mm). TL: the ground beneath.
            Builtin::Snow => (
                [0.45, 0.75, 0.90, 0.95, 0.95, 0.95],
                ROUGH,
                Bands::splat(MAX_TRANSMISSION_LOSS_DB),
            ),
            // [E] water surface, as in a swimming pool. TL: deep water.
            Builtin::Water => (
                [0.008, 0.008, 0.013, 0.015, 0.020, 0.025],
                SMOOTH,
                Bands::splat(MAX_TRANSMISSION_LOSS_DB),
            ),
            // [E] trees, balsam firs, 20 sq ft (1.9 m2) ground area per tree,
            // 11 ft (3.4 m) high.
            // TL per crossing: [I] for 10 to 20 m of dense foliage
            // (0, 1, 1, 1, 1, 2, 3 dB at 125 Hz to 8 kHz).
            Builtin::Foliage => (
                [0.03, 0.06, 0.11, 0.17, 0.27, 0.31],
                VEGETATION,
                Bands([0.0, 1.0, 1.0, 1.0, 1.0, 2.0, 3.0]),
            ),
            // No entry in [E]; uses [E] unglazed brick as the stand-in for
            // bare rock. TL: solid rock, opaque.
            Builtin::Rock => (
                [0.03, 0.03, 0.03, 0.04, 0.05, 0.07],
                ROUGH,
                Bands::splat(MAX_TRANSMISSION_LOSS_DB),
            ),
        }
    }

    /// The built-in material with this name.
    pub fn from_name(name: &str) -> Option<Builtin> {
        Builtin::ALL.into_iter().find(|b| b.name() == name)
    }
}

/// 125 Hz to 4 kHz values, with 8 kHz repeating 4 kHz.
fn oct(v: [f64; 6]) -> Bands {
    Bands([v[0], v[1], v[2], v[3], v[4], v[5], v[5]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mass_law_known_values() {
        // 100 kg/m2 at 1 kHz: 20 log10(1e5) - 47 = 53 dB.
        assert!((mass_law_tl(100.0).0[3] - 53.0).abs() < 1e-9);
        // Doubling mass adds 6 dB.
        let a = mass_law_tl(10.0).0[2];
        let b = mass_law_tl(20.0).0[2];
        assert!((b - a - 6.0206).abs() < 1e-3);
        assert_eq!(mass_law_tl(0.0), Bands::ZERO);
        assert_eq!(mass_law_tl(f64::NAN), Bands::ZERO);
        assert_eq!(mass_law_tl(1e12).0[6], MAX_TRANSMISSION_LOSS_DB);
    }

    #[test]
    fn builtins_are_valid_and_named_uniquely() {
        for (i, b) in Builtin::ALL.iter().enumerate() {
            let m = b.material();
            assert_eq!(m.sanitized().unwrap(), m, "{}", b.name());
            assert_eq!(Builtin::from_name(b.name()), Some(*b));
            for other in &Builtin::ALL[i + 1..] {
                assert_ne!(b.name(), other.name());
            }
        }
    }

    #[test]
    fn soft_materials_absorb_more_than_hard_ones() {
        let carpet = Builtin::Carpet.material().absorption.mean();
        let concrete = Builtin::Concrete.material().absorption.mean();
        assert!(carpet > 5.0 * concrete);
        assert!(
            Builtin::Glass.material().transmission_loss_db.0[3]
                < Builtin::Concrete.material().transmission_loss_db.0[3]
        );
    }

    #[test]
    fn desc_validation() {
        let mut d = MaterialDesc {
            name: "  moss  ".into(),
            absorption: Bands::splat(2.0),
            scattering: None,
            transmission_loss_db: None,
            surface_density_kg_m2: Some(10.0),
        };
        let m = d.to_material().unwrap();
        assert_eq!(m.name, "moss");
        assert_eq!(m.absorption, Bands::ONE);
        assert_eq!(m.transmission_loss_db, mass_law_tl(10.0));
        d.absorption.0[3] = f64::NAN;
        assert_eq!(d.to_material(), Err(MaterialError::NotFinite));
        d.absorption = Bands::ZERO;
        d.name = "   ".into();
        assert_eq!(d.to_material(), Err(MaterialError::EmptyName));
        d.name = "x".repeat(10_000);
        assert_eq!(d.to_material().unwrap().name.len(), MAX_MATERIAL_NAME);
    }

    #[test]
    fn loads_from_json() {
        let d: MaterialDesc = serde_json::from_str(
            r#"{"name":"felt","absorption":[0.1,0.2,0.3,0.4,0.5,0.6,0.6],
                "transmission_loss_db":[1,2,3,4,5,6,7]}"#,
        )
        .unwrap();
        let m = d.to_material().unwrap();
        assert_eq!(m.transmission_loss_db.0[6], 7.0);
        assert_eq!(m.scattering, SMOOTH);
    }

    #[test]
    fn specular_gain() {
        let m = Material {
            name: "x".into(),
            absorption: Bands::splat(0.36),
            scattering: Bands::ZERO,
            transmission_loss_db: Bands::ZERO,
        };
        assert!((m.specular_gain().0[0] - 0.8).abs() < 1e-12);
    }
}
