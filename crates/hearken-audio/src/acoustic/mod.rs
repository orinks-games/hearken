//! Rendering the acoustic model (`hearken-acoustics`) for positioned
//! sounds: an opt-in path next to [`Audio::update`](crate::Audio::update).
//!
//! With [`Audio::set_acoustics`](crate::Audio::set_acoustics) on, every
//! positioned sound started on one of the chosen buses carries an
//! [`AcousticDsp`] insert, and [`Audio::update_acoustic`](crate::Audio::update_acoustic)
//! works out, each frame, how the sound reaches the listener through the
//! game's [`Scene`]: its delay, per-band gains, early reflections and
//! reverb send. A backend that cannot run the insert gets the nearest plain
//! mix instead (a gain, a pan and a low-pass).

// DSP over parallel fixed arrays reads better with indices.
#![allow(clippy::needless_range_loop)]

pub(crate) mod dsp;
mod filters;
#[cfg(feature = "hrtf")]
pub(crate) mod hrtf;
mod mailbox;
pub(crate) mod room;
pub(crate) mod sense;
pub(crate) mod state;
pub(crate) mod voice;

pub use dsp::{AcousticDsp, AcousticGain};
#[cfg(feature = "hrtf")]
pub use hrtf::{HrtfSet, HrtfSource, BUILTIN_SOFA};
pub use room::{ReverbSend, RoomReverbDsp};
pub use sense::ObstacleSense;
pub use voice::AcousticVoice;

use hearken_acoustics::{
    nearby_surfaces, to_listener_frame, NearbyOptions, QueryOptions, ReverbOptions, Scene,
    ShapeKind, Vec3 as AVec3,
};

use crate::types::{Bus, Vec3};
use sense::{Sector, SECTORS};

/// How the acoustic path renders. Every field is the game's to change; the
/// player-facing ones are `hrtf` (headphones) and `obstacle_sense`.
#[derive(Clone, Debug, PartialEq)]
pub struct AcousticSettings {
    /// How much work each sound's propagation query does.
    pub query: QueryOptions,
    /// How the room round the listener is estimated for the shared reverb.
    pub reverb: ReverbOptions,
    /// Positioned sounds started on these buses get the acoustic path; the
    /// others keep the plain spatial model.
    pub buses: Vec<Bus>,
    /// Glide the delay as distances change, which gives Doppler. Off, a new
    /// delay crossfades in without a pitch change.
    pub doppler: bool,
    /// Keep the propagation delay (sound takes time to arrive). Off, the
    /// main arrival is at once and reflections keep their lag behind it.
    pub delay: bool,
    /// Early reflections as taps.
    pub reflections: bool,
    /// The shared reverb.
    pub room: bool,
    /// Render the main arrival through an HRTF, for headphones. Speakers
    /// keep the pan law. Needs the `hrtf` feature; otherwise ignored.
    pub hrtf: bool,
    /// How long parameter changes ramp, ms: about one or two game frames.
    pub ramp_ms: f32,
    /// Longest delay a sound's line holds, seconds (0.5 s is about 170 m).
    pub max_delay_s: f32,
    /// The obstacle-sense noise floor, 0 (off) to 1.
    pub obstacle_sense: f32,
    /// How far obstacle sense reaches, metres.
    pub sense_radius_m: f64,
    /// The room is estimated again when the listener moves this far,
    /// metres, or the scene changes.
    pub room_move_m: f64,
}

impl Default for AcousticSettings {
    fn default() -> AcousticSettings {
        AcousticSettings {
            query: QueryOptions::default(),
            reverb: ReverbOptions::default(),
            buses: vec![Bus::EFFECTS, Bus::AMBIENCE, Bus::VOICE],
            doppler: true,
            delay: true,
            reflections: true,
            room: true,
            hrtf: false,
            ramp_ms: 50.0,
            max_delay_s: 0.5,
            obstacle_sense: 0.0,
            sense_radius_m: 4.0,
            room_move_m: 0.5,
        }
    }
}

/// A position from the pool's `f32` frame into the model's `f64` one.
pub(crate) fn to_model(v: Vec3) -> AVec3 {
    AVec3::new(f64::from(v.x), f64::from(v.y), f64::from(v.z))
}

/// Unit vector in the world for sector `i` round a listener facing
/// `facing` (sector 0 ahead, then clockwise).
fn sector_dir(i: usize, facing: f32) -> AVec3 {
    let deg = f64::from(facing) + i as f64 * 45.0;
    let rad = deg.to_radians();
    AVec3::new(rad.sin(), rad.cos(), 0.0)
}

/// The obstacle-sense floor's sectors for a listener at `pos` facing
/// `facing`: a ray along each sector finds walls and boxes; poles nearby
/// shadow whichever sector they stand in.
pub(crate) fn sense_sectors(
    scene: &Scene,
    pos: Vec3,
    facing: f32,
    radius: f64,
) -> [Sector; SECTORS] {
    let mut out = [Sector::CLEAR; SECTORS];
    let p = to_model(pos);
    let radius = if radius.is_finite() {
        radius.clamp(0.5, 20.0)
    } else {
        4.0
    };
    let c = scene.speed_of_sound().max(1.0);
    for (i, s) in out.iter_mut().enumerate() {
        let dir = sector_dir(i, facing);
        let Some((d, _, id)) = scene.ray_cast(p, dir, radius) else {
            continue;
        };
        let Some(obj) = scene.object(id) else {
            continue;
        };
        if obj.shape.kind() == ShapeKind::Cylinder {
            // Poles are handled below with their real width.
            continue;
        }
        let (mid, high) = scene
            .material(obj.material)
            .map(|m| {
                let a = m.absorption.0;
                ((a[2] + a[3] + a[4]) / 3.0, (a[5] + a[6]) / 2.0)
            })
            .unwrap_or((0.1, 0.1));
        // Nearer walls count for more; at the edge of the radius, nothing.
        let w = (1.0 - d / radius).clamp(0.0, 1.0).sqrt() as f32;
        s.open = 1.0 - 0.9 * w;
        s.reflect = w * (1.0 - mid.clamp(0.0, 1.0)).sqrt() as f32 * 0.9;
        s.delay_s = (2.0 * d / c) as f32;
        s.reflect_hz = (1_000.0 + 15_000.0 * (1.0 - high.clamp(0.0, 1.0))) as f32;
    }
    let near = nearby_surfaces(
        scene,
        p,
        &NearbyOptions {
            radius,
            max: 32,
            include_planes: false,
        },
    );
    for surf in near
        .surfaces
        .iter()
        .filter(|s| s.kind == ShapeKind::Cylinder)
    {
        let local = to_listener_frame(surf.direction, f64::from(facing));
        if local.x == 0.0 && local.y == 0.0 {
            continue;
        }
        let az = libm::atan2(local.x, local.y).to_degrees();
        let i = (((az / 45.0).round() as i64).rem_euclid(SECTORS as i64)) as usize;
        let d = surf.distance.max(0.05);
        // The share of the sector it hides, and the frequency above which
        // it casts a shadow (its width about one wavelength).
        let cover = (surf.width_m / (d * std::f64::consts::FRAC_PI_4)).clamp(0.0, 1.0);
        let w = (1.0 - d / radius).clamp(0.0, 1.0);
        let cover = (cover * w) as f32;
        if cover > out[i].cover {
            out[i].cover = cover;
            out[i].cover_hz = (c / surf.width_m.max(0.01)).clamp(200.0, 16_000.0) as f32;
            out[i].reflect = out[i].reflect.max(0.3 * cover);
            if out[i].delay_s == 0.0 {
                out[i].delay_s = (2.0 * d / c) as f32;
            }
        }
    }
    out
}
