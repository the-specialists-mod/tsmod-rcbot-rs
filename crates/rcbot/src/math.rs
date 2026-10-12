//! The HL SDK's `Vector` (`dlls/vector.h`), with RCBot's own `Length` (`std::pow(…, 0.5f)`).

use std::ops::{Add, Mul, Neg, Sub};

/// A 3D vector of `f32`, laid out like the engine's `vec3_t`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vector {
    /// X.
    pub x: f32,
    /// Y.
    pub y: f32,
    /// Z.
    pub z: f32,
}

impl Vector {
    /// The zero vector.
    pub const ZERO: Vector = Vector { x: 0.0, y: 0.0, z: 0.0 };

    /// A vector from its components.
    pub const fn new(x: f32, y: f32, z: f32) -> Vector {
        Vector { x, y, z }
    }

    /// `Vector::Length`: RCBot computes it as `std::pow(x*x + y*y + z*z, 0.5f)`, which promotes to
    /// `double` and narrows the result back to `float`.
    pub fn length(self) -> f32 {
        f64::from(self.x * self.x + self.y * self.y + self.z * self.z).powf(0.5) as f32
    }

    /// `Vector::Length2D`.
    pub fn length_2d(self) -> f32 {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    /// `Vector::Normalize`: a zero vector normalises to `(0, 0, 1)`.
    pub fn normalize(self) -> Vector {
        let len = self.length();
        if len == 0.0 {
            return Vector::new(0.0, 0.0, 1.0);
        }
        let inv = 1.0 / len;
        Vector::new(self.x * inv, self.y * inv, self.z * inv)
    }

    /// The vector as the engine's `float[3]`.
    pub const fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }

    /// `AngleVectors` forward vector for `angles` (pitch, yaw, roll in degrees), as
    /// `UTIL_MakeVectors` leaves it in `gpGlobals->v_forward`.
    pub fn forward_from_angles(angles: Vector) -> Vector {
        let (sp, cp) = (angles.x * (std::f32::consts::PI * 2.0 / 360.0)).sin_cos();
        let (sy, cy) = (angles.y * (std::f32::consts::PI * 2.0 / 360.0)).sin_cos();
        Vector::new(cp * cy, cp * sy, -sp)
    }
}

impl Add for Vector {
    type Output = Vector;
    fn add(self, o: Vector) -> Vector {
        Vector::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for Vector {
    type Output = Vector;
    fn sub(self, o: Vector) -> Vector {
        Vector::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f32> for Vector {
    type Output = Vector;
    fn mul(self, f: f32) -> Vector {
        Vector::new(self.x * f, self.y * f, self.z * f)
    }
}

impl Neg for Vector {
    type Output = Vector;
    fn neg(self) -> Vector {
        Vector::new(-self.x, -self.y, -self.z)
    }
}

impl From<[f32; 3]> for Vector {
    fn from(a: [f32; 3]) -> Vector {
        Vector::new(a[0], a[1], a[2])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_and_normalize() {
        assert_eq!(Vector::new(3.0, 4.0, 0.0).length(), 5.0);
        assert_eq!(Vector::new(3.0, 4.0, 12.0).length(), 13.0);
        assert_eq!(Vector::ZERO.normalize(), Vector::new(0.0, 0.0, 1.0));
        let n = Vector::new(0.0, 10.0, 0.0).normalize();
        assert_eq!(n, Vector::new(0.0, 1.0, 0.0));
    }

    #[test]
    fn forward() {
        let f = Vector::forward_from_angles(Vector::new(0.0, 90.0, 0.0));
        assert!(f.x.abs() < 1e-6 && (f.y - 1.0).abs() < 1e-6 && f.z == 0.0);
    }
}
