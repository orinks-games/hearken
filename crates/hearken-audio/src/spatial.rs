use crate::types::{Listener, Vec3};

/// Configuration for the spatial positioning model.
/// Every number is the game's to change; the occlusion values are Lucky Break's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpatialConfig {
    pub min_distance: f32,         // 1.5  full volume inside this
    pub max_distance: f32,         // 30.0 no quieter beyond this
    pub rolloff: f32,              // 1.0
    pub behind_lowpass_hz: f32,    // 3000.0 for a sound straight behind
    pub behind_gain: f32,          // 0.8
    pub occlusion_lowpass_hz: f32, // 800.0
    pub occlusion_gain: f32,       // 0.5, -6 dB
}

impl Default for SpatialConfig {
    fn default() -> Self {
        SpatialConfig {
            min_distance: 1.5,
            max_distance: 30.0,
            rolloff: 1.0,
            behind_lowpass_hz: 3000.0,
            behind_gain: 0.8,
            occlusion_lowpass_hz: 800.0,
            occlusion_gain: 0.5,
        }
    }
}

/// The result of positioning a sound relative to the listener.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub gain: f32,
    pub pan: f32,
    pub lowpass_hz: Option<f32>,
    pub relative: Vec3,
}

/// Calculate the placement of a sound relative to the listener.
/// `relative` is the sound's position in the listener's frame: x to the right, y ahead, z up.
pub fn place(listener: &Listener, pos: Vec3, occluded: bool, cfg: &SpatialConfig) -> Placement {
    let rel = pos - listener.pos;
    if !rel.is_finite() {
        return Placement {
            gain: 1.0,
            pan: 0.0,
            lowpass_hz: None,
            relative: Vec3::ZERO,
        };
    }
    let facing = if listener.facing.is_finite() {
        listener.facing
    } else {
        0.0
    };
    let (s, c) = facing.to_radians().sin_cos();
    let right = rel.x * c - rel.y * s;
    let ahead = rel.x * s + rel.y * c;
    let min = cfg.min_distance.max(f32::EPSILON);
    let d = rel.length().clamp(min, cfg.max_distance.max(min));
    let mut gain = min / (min + cfg.rolloff.max(0.0) * (d - min));
    let flat = right.hypot(ahead);
    let (pan, behind) = if flat > 1e-4 {
        ((right / flat).clamp(-1.0, 1.0), (-ahead / flat).max(0.0))
    } else {
        (0.0, 0.0)
    };
    let mut lowpass = None;
    if behind > 0.0 {
        gain *= 1.0 - (1.0 - cfg.behind_gain) * behind;
        lowpass = Some(20_000.0 * (cfg.behind_lowpass_hz / 20_000.0).powf(behind));
    }
    if occluded {
        gain *= cfg.occlusion_gain;
        lowpass = Some(lowpass.map_or(cfg.occlusion_lowpass_hz, |hz: f32| {
            hz.min(cfg.occlusion_lowpass_hz)
        }));
    }
    Placement {
        gain,
        pan,
        lowpass_hz: lowpass,
        relative: Vec3::new(right, ahead, rel.z),
    }
}
