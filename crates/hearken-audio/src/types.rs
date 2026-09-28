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

/// The listener's position and orientation.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Listener {
    pub pos: Vec3,
    pub facing: f32, // degrees clockwise from north
}
