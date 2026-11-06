//! A small 3D vector in metres.
//!
//! Axes follow the rest of Hearken: x east, y north, z up. A 2D game keeps
//! every z the same (for example 1.5, ear height).

use core::ops::{Add, Mul, Neg, Sub};
use serde::{Deserialize, Serialize};

/// Farthest a query position may be from the origin on any axis, metres.
pub const MAX_POSITION: f64 = 1.0e6;

/// A point or direction in metres: x east, y north, z up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3::new(0.0, 0.0, 0.0);
    pub const UP: Vec3 = Vec3::new(0.0, 0.0, 1.0);

    pub const fn new(x: f64, y: f64, z: f64) -> Vec3 {
        Vec3 { x, y, z }
    }

    pub fn dot(self, o: Vec3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    pub fn length_sq(self) -> f64 {
        self.dot(self)
    }

    pub fn length(self) -> f64 {
        self.length_sq().sqrt()
    }

    /// The unit vector, or `None` for a zero or non-finite vector.
    pub fn normalized(self) -> Option<Vec3> {
        let l = self.length();
        if l > 1e-12 && l.is_finite() {
            Some(self * (1.0 / l))
        } else {
            None
        }
    }

    pub fn distance(self, o: Vec3) -> f64 {
        (self - o).length()
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    /// Whether this is a position queries accept: finite and within
    /// [`MAX_POSITION`] of the origin on every axis. Queries given anything
    /// else return an empty or silent result.
    pub fn is_usable(self) -> bool {
        self.x.abs() <= MAX_POSITION && self.y.abs() <= MAX_POSITION && self.z.abs() <= MAX_POSITION
    }

    pub fn min(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x.min(o.x), self.y.min(o.y), self.z.min(o.z))
    }

    pub fn max(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z))
    }

    /// The point `t` of the way from `self` to `o`.
    pub fn lerp(self, o: Vec3, t: f64) -> Vec3 {
        self + (o - self) * t
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f64> for Vec3 {
    type Output = Vec3;
    fn mul(self, s: f64) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }
}

impl Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_and_dot() {
        let x = Vec3::new(1.0, 0.0, 0.0);
        let y = Vec3::new(0.0, 1.0, 0.0);
        assert_eq!(x.cross(y), Vec3::UP);
        assert_eq!(x.dot(y), 0.0);
        assert_eq!(Vec3::new(3.0, 4.0, 0.0).length(), 5.0);
    }

    #[test]
    fn normalizing_zero_or_nan_gives_none() {
        assert!(Vec3::ZERO.normalized().is_none());
        assert!(Vec3::new(f64::NAN, 0.0, 0.0).normalized().is_none());
        assert!(Vec3::new(f64::INFINITY, 0.0, 0.0).normalized().is_none());
        assert!(!Vec3::new(f64::NAN, 0.0, 0.0).is_usable());
        assert!(!Vec3::new(0.0, 1e300, 0.0).is_usable());
        assert!(Vec3::new(0.0, -1e6, 0.0).is_usable());
    }
}
