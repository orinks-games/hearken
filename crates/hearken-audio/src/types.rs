use std::ops::Sub;

/// A position in 3D space: x east, y north, z up.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    pub const fn new(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3 { x, y, z }
    }

    pub fn length(self) -> f32 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

impl Sub for Vec3 {
    type Output = Vec3;

    fn sub(self, other: Vec3) -> Vec3 {
        Vec3 {
            x: self.x - other.x,
            y: self.y - other.y,
            z: self.z - other.z,
        }
    }
}

/// An audio bus identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Bus(pub &'static str);

impl Bus {
    /// Every bus at once: the player's overall volume.
    pub const MASTER: Bus = Bus("master");
    /// Menu and interface earcons.
    pub const UI: Bus = Bus("ui");
    /// One-shot and looping sound effects.
    pub const EFFECTS: Bus = Bus("effects");
    /// Ambience beds.
    pub const AMBIENCE: Bus = Bus("ambience");
    /// Music.
    pub const MUSIC: Bus = Bus("music");
    /// Recorded character dialogue, not synth voices or the game's own speech.
    pub const VOICE: Bus = Bus("voice");

    pub const BUILT_IN: [Bus; 6] = [
        Bus::MASTER,
        Bus::UI,
        Bus::EFFECTS,
        Bus::AMBIENCE,
        Bus::MUSIC,
        Bus::VOICE,
    ];

    pub fn name(self) -> &'static str {
        self.0
    }
}

/// Mix parameters: volume, stereo position, pitch and optional low-pass filter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mix {
    pub gain: f32,
    pub pan: f32,
    pub pitch: f32,
    pub lowpass_hz: Option<f32>,
    pub relative: Option<Vec3>,
}

impl Mix {
    pub const UNITY: Mix = Mix {
        gain: 1.0,
        pan: 0.0,
        pitch: 1.0,
        lowpass_hz: None,
        relative: None,
    };
}

/// The part of a clip a looping sound repeats, in seconds from the clip's
/// start: it plays from the start, repeats `start..end` until released, then
/// plays on through whatever follows `end`. `end: None` loops to the clip's
/// end. This is how a horn, a bell or an engine is one file with its attack,
/// its sustain and its release, instead of three files cut by hand.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopRegion {
    pub start: f64,
    pub end: Option<f64>,
}

impl LoopRegion {
    /// Repeats `start..end` seconds.
    pub const fn new(start: f64, end: f64) -> LoopRegion {
        LoopRegion {
            start,
            end: Some(end),
        }
    }

    /// Repeats from `start` seconds to the clip's end.
    pub const fn from(start: f64) -> LoopRegion {
        LoopRegion { start, end: None }
    }

    /// Whether the region is usable: a finite, non-negative start, and an
    /// end after it if there is one.
    pub fn is_valid(&self) -> bool {
        self.start.is_finite()
            && self.start >= 0.0
            && self
                .end
                .is_none_or(|end| end.is_finite() && end > self.start)
    }
}

/// The listener's position and orientation.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Listener {
    pub pos: Vec3,
    pub facing: f32, // degrees clockwise from north
}

/// How long each part of a mix change ramps, in milliseconds; zero is
/// instant. A game that sets a sound's pitch every frame over one length and
/// its volume at once (an engine) needs the two apart, which a single ramp
/// for the whole [`Mix`] cannot give.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Ramps {
    pub gain_ms: u32,
    pub pan_ms: u32,
    pub pitch_ms: u32,
    pub filter_ms: u32,
}

impl Ramps {
    /// Every part over the same `ms`.
    pub const fn all(ms: u32) -> Ramps {
        Ramps {
            gain_ms: ms,
            pan_ms: ms,
            pitch_ms: ms,
            filter_ms: ms,
        }
    }
}

/// How a sound starts, for [`Audio::start_with`](crate::Audio::start_with)
/// and [`Audio::play_clip`](crate::Audio::play_clip): its own volume, pan
/// and pitch from the first sample (no ramp from unity), whether it loops,
/// the region a loop repeats, and how far into the clip it starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Start {
    /// `0..=1`, before the bus volume.
    pub volume: f32,
    /// `-1` (left) to `1` (right). Ignored for a positioned sound, which
    /// takes its pan from where it is.
    pub pan: f32,
    pub pitch: f32,
    pub looping: bool,
    /// Only for a looping sound; see [`Audio::start_loop_region`](crate::Audio::start_loop_region).
    pub region: Option<LoopRegion>,
    /// Seconds into the clip to start from; `0` is the top.
    pub from_s: f64,
}

impl Start {
    /// Once, at unity, centred, from the top.
    pub const ONCE: Start = Start {
        volume: 1.0,
        pan: 0.0,
        pitch: 1.0,
        looping: false,
        region: None,
        from_s: 0.0,
    };

    /// Looping, at unity, centred, from the top.
    pub const LOOP: Start = Start {
        looping: true,
        ..Start::ONCE
    };

    pub const fn volume(self, volume: f32) -> Start {
        Start { volume, ..self }
    }

    pub const fn pan(self, pan: f32) -> Start {
        Start { pan, ..self }
    }

    pub const fn pitch(self, pitch: f32) -> Start {
        Start { pitch, ..self }
    }

    pub const fn region(self, region: LoopRegion) -> Start {
        Start {
            region: Some(region),
            ..self
        }
    }

    pub const fn from_s(self, from_s: f64) -> Start {
        Start { from_s, ..self }
    }
}

impl Default for Start {
    fn default() -> Start {
        Start::ONCE
    }
}
