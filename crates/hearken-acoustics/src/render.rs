//! Everything about how one sound reaches the listener, and the compact,
//! fixed-size form an audio thread uses.

use crate::bands::{Bands, BAND_COUNT};
use crate::direct::{direct_with, DirectPath};
use crate::options::QueryOptions;
use crate::path::{diffracted_path, DiffractedPath};
use crate::reflections::{early_reflections, Reflection};
use crate::reverb::ReverbEstimate;
use crate::scene::{Probe, Scene};
use crate::vec::Vec3;

/// Most reflection taps in a [`SoundRender`].
pub const MAX_TAPS: usize = 8;

/// Gains below this become exactly zero, so audio code never sees
/// denormals from here.
const GAIN_FLOOR: f64 = 1e-9;

/// How a sound reaches the listener: straight, round corners and off walls.
#[derive(Clone, Debug, PartialEq)]
pub struct Propagation {
    pub direct: DirectPath,
    /// The path round obstacles, when the straight line is blocked.
    pub diffracted: Option<DiffractedPath>,
    /// Early reflections, strongest first.
    pub reflections: Vec<Reflection>,
    /// Whether any part of the query hit a cap.
    pub truncated: bool,
    source: Vec3,
    listener: Vec3,
}

/// Works out how a sound at `source` reaches `listener`.
pub fn propagate(scene: &Scene, source: Vec3, listener: Vec3, opts: &QueryOptions) -> Propagation {
    let opts = opts.clamped();
    let direct = direct_with(scene, source, listener, &opts, &mut Probe::unlimited());
    let mut truncated = direct.truncated;
    let diffracted = if direct.is_blocked() {
        let d = diffracted_path(scene, source, listener, &opts);
        truncated |= d.truncated;
        d.path
    } else {
        None
    };
    let refl = early_reflections(scene, source, listener, &opts);
    truncated |= refl.truncated;
    Propagation {
        direct,
        diffracted,
        reflections: refl.list,
        truncated,
        source,
        listener,
    }
}

/// One reflection as an audio thread uses it.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Tap {
    /// Seconds after the sound leaves the source.
    pub delay_s: f32,
    /// Linear amplitude per band.
    pub gains: [f32; BAND_COUNT],
    /// Unit direction in the listener's frame: x right, y ahead, z up.
    pub direction: [f32; 3],
}

/// Per-sound render parameters: plain numbers in a fixed-size `Copy` value,
/// so it can cross to an audio thread through a lock-free queue or a
/// triple buffer without allocating.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct SoundRender {
    /// Delay of the main arrival, seconds. A smoothly changing delay gives
    /// Doppler.
    pub delay_s: f32,
    /// Length of the main arrival's path, metres.
    pub distance_m: f32,
    /// Linear amplitude per band of the direct and diffracted sound
    /// together.
    pub gains: [f32; BAND_COUNT],
    /// Unit direction the sound is heard from, in the listener's frame:
    /// x right, y ahead, z up. Zero when source and listener coincide.
    pub direction: [f32; 3],
    /// Degrees clockwise from straight ahead, -180 to 180.
    pub azimuth_deg: f32,
    /// Degrees above the horizontal, -90 to 90.
    pub elevation_deg: f32,
    /// Whether the straight line is blocked.
    pub blocked: bool,
    /// Send level to the shared reverb, as a linear amplitude.
    pub reverb_send: f32,
    /// Early reflections, strongest first; only `tap_count` are used.
    pub taps: [Tap; MAX_TAPS],
    pub tap_count: u8,
}

/// Room settings for a shared reverb, from [`ReverbEstimate`].
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct RoomRender {
    /// Reverberation time per band, seconds.
    pub rt60_s: [f32; BAND_COUNT],
    /// Reverberant level per band, linear amplitude.
    pub wet_gains: [f32; BAND_COUNT],
    /// A pre-delay for the tail: the mean free path over the speed of
    /// sound, seconds.
    pub predelay_s: f32,
}

impl ReverbEstimate {
    /// The compact form for an audio thread. `speed_of_sound` is the
    /// scene's.
    pub fn render(&self, speed_of_sound: f64) -> RoomRender {
        let c = if speed_of_sound > 0.0 {
            speed_of_sound
        } else {
            343.0
        };
        RoomRender {
            rt60_s: self.rt60().to_f32(),
            wet_gains: floor(self.wet_gain).to_f32(),
            predelay_s: (self.mean_free_path_m / c) as f32,
        }
    }
}

fn floor(b: Bands) -> Bands {
    b.map(|v| if v.abs() < GAIN_FLOOR { 0.0 } else { v })
}

/// A world direction in the listener's frame (x right, y ahead, z up), for a
/// listener facing `facing_deg` degrees clockwise from north.
pub fn to_listener_frame(dir: Vec3, facing_deg: f64) -> Vec3 {
    let f = if facing_deg.is_finite() {
        facing_deg
    } else {
        0.0
    };
    let rad = f * (core::f64::consts::PI / 180.0);
    let (s, c) = (libm::sin(rad), libm::cos(rad));
    Vec3::new(dir.x * c - dir.y * s, dir.x * s + dir.y * c, dir.z)
}

fn to_f32(v: Vec3) -> [f32; 3] {
    [v.x as f32, v.y as f32, v.z as f32]
}

impl Propagation {
    /// The render parameters for a listener facing `facing_deg` (degrees
    /// clockwise from north). With `room`, the reverb send is the room's
    /// wet level scaled by how well the sound reaches the listener's space.
    ///
    /// When the straight line is blocked, the transmitted and diffracted
    /// sound add by energy per band; the direction is their energy-weighted
    /// mean and the delay is the stronger one's.
    pub fn render(&self, facing_deg: f64, room: Option<&ReverbEstimate>) -> SoundRender {
        let d = &self.direct;
        let to_source = (self.source - self.listener)
            .normalized()
            .unwrap_or(Vec3::ZERO);
        let (gains, dir, delay, dist) = match &self.diffracted {
            Some(p) if d.is_blocked() => {
                let ed = d.gain.energy();
                let ep = p.gain.energy();
                let gains = d.gain.zip(p.gain, |a, b| (a * a + b * b).sqrt());
                let to_arrival = (p.arrival - self.listener)
                    .normalized()
                    .unwrap_or(to_source);
                let mixed = to_source * ed + to_arrival * ep;
                let strong = if ep > ed { to_arrival } else { to_source };
                let dir = mixed.normalized().unwrap_or(strong);
                if ep > ed {
                    (gains, dir, p.delay_s, p.length)
                } else {
                    (gains, dir, d.delay_s, d.distance)
                }
            }
            _ => (d.gain, to_source, d.delay_s, d.distance),
        };
        let local = to_listener_frame(dir, facing_deg);
        let flat = (local.x * local.x + local.y * local.y).sqrt();
        let (az, el) = if local.length_sq() > 0.0 {
            (
                libm::atan2(local.x, local.y) * (180.0 / core::f64::consts::PI),
                libm::atan2(local.z, flat) * (180.0 / core::f64::consts::PI),
            )
        } else {
            (0.0, 0.0)
        };
        let mut out = SoundRender {
            delay_s: delay as f32,
            distance_m: dist as f32,
            gains: floor(gains).to_f32(),
            direction: to_f32(local),
            azimuth_deg: az as f32,
            elevation_deg: el as f32,
            blocked: d.is_blocked(),
            ..SoundRender::default()
        };
        if let Some(room) = room {
            // How much of the free-field level arrives, 0 to 1.
            let reach = if d.spreading > 0.0 {
                (gains.mean() / d.spreading).min(1.0)
            } else {
                0.0
            };
            let send = room.wet_gain.mean() * reach;
            out.reverb_send = if send < GAIN_FLOOR { 0.0 } else { send as f32 };
        }
        for (tap, r) in out.taps.iter_mut().zip(&self.reflections) {
            *tap = Tap {
                delay_s: r.delay_s as f32,
                gains: floor(r.gain).to_f32(),
                direction: to_f32(to_listener_frame(r.direction, facing_deg)),
            };
            out.tap_count += 1;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Shape;
    use crate::material::Builtin;
    use crate::reverb::{estimate_reverb, ReverbOptions};
    use crate::scene::SceneBuilder;

    #[test]
    fn frame_matches_hearken_audio_convention() {
        // Facing north, a sound to the east is on the right.
        let v = to_listener_frame(Vec3::new(1.0, 0.0, 0.0), 0.0);
        assert!((v.x - 1.0).abs() < 1e-12 && v.y.abs() < 1e-12);
        // Facing east, a sound to the east is ahead.
        let v = to_listener_frame(Vec3::new(1.0, 0.0, 0.0), 90.0);
        assert!(v.x.abs() < 1e-12 && (v.y - 1.0).abs() < 1e-12);
    }

    #[test]
    fn open_field_render() {
        let s = Scene::default();
        let p = propagate(
            &s,
            Vec3::new(10.0, 0.0, 1.5),
            Vec3::new(0.0, 0.0, 1.5),
            &QueryOptions::default(),
        );
        let r = p.render(0.0, None);
        assert!(!r.blocked);
        assert!((r.azimuth_deg - 90.0).abs() < 1e-4);
        assert_eq!(r.elevation_deg, 0.0);
        assert!((r.gains[0] - 0.1).abs() < 1e-3);
        assert_eq!(r.tap_count, 0);
        assert_eq!(r.reverb_send, 0.0);
    }

    #[test]
    fn behind_a_doorway_sound_comes_from_the_door() {
        let mut b = SceneBuilder::new();
        let brick = Builtin::Brick.id();
        b.add(Shape::wall_between(5.0, -10.0, 5.0, 2.0, 0.0, 3.0), brick)
            .unwrap();
        b.add(Shape::wall_between(5.0, 3.0, 5.0, 10.0, 0.0, 3.0), brick)
            .unwrap();
        let s = b.build();
        let p = propagate(
            &s,
            Vec3::new(8.0, -3.0, 1.5),
            Vec3::new(2.0, 0.0, 1.5),
            &QueryOptions::default(),
        );
        assert!(p.direct.is_blocked());
        assert!(p.diffracted.is_some());
        let r = p.render(0.0, None);
        assert!(r.blocked);
        // The straight line points south-east, the doorway north-east;
        // diffraction is far stronger than 50 dB of brick, so the sound is
        // heard from the north-east.
        assert!(r.direction[1] > 0.0, "{:?}", r.direction);
        assert!(r.direction[0] > 0.0);
    }

    #[test]
    fn taps_and_reverb_send_in_a_room() {
        let mut b = SceneBuilder::new();
        let m = Builtin::Plaster.id();
        b.add(Shape::wall_between(0.0, 0.0, 6.0, 0.0, 0.0, 3.0), m)
            .unwrap();
        b.add(Shape::wall_between(6.0, 0.0, 6.0, 5.0, 0.0, 3.0), m)
            .unwrap();
        b.add(Shape::wall_between(6.0, 5.0, 0.0, 5.0, 0.0, 3.0), m)
            .unwrap();
        b.add(Shape::wall_between(0.0, 5.0, 0.0, 0.0, 0.0, 3.0), m)
            .unwrap();
        b.add(Shape::Ground { z: 0.0 }, Builtin::Wood.id()).unwrap();
        b.add(Shape::Ceiling { z: 3.0 }, m).unwrap();
        let s = b.build();
        let lis = Vec3::new(3.0, 2.0, 1.5);
        let p = propagate(&s, Vec3::new(4.0, 3.0, 1.2), lis, &QueryOptions::default());
        let room = estimate_reverb(&s, lis, &ReverbOptions::default());
        let r = p.render(45.0, Some(&room));
        assert_eq!(r.tap_count as usize, MAX_TAPS);
        assert!(r.reverb_send > 0.0);
        for t in &r.taps {
            assert!(t.delay_s > r.delay_s);
            let l =
                (t.direction[0].powi(2) + t.direction[1].powi(2) + t.direction[2].powi(2)).sqrt();
            assert!((l - 1.0).abs() < 1e-5);
        }
        let rr = room.render(s.speed_of_sound());
        assert!(rr.rt60_s[3] > 0.2 && rr.predelay_s > 0.0);
    }

    #[test]
    fn coincident_source_and_listener() {
        let s = Scene::default();
        let p = propagate(&s, Vec3::ZERO, Vec3::ZERO, &QueryOptions::default());
        let r = p.render(f64::NAN, None);
        assert_eq!(r.direction, [0.0; 3]);
        assert_eq!(r.gains, [1.0; BAND_COUNT]);
    }
}
