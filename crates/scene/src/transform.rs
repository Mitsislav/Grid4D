use glam::{EulerRot, Mat4, Quat, Vec3};

/// Local transform of a node, relative to its parent.
///
/// Rotation is stored as Euler angles in Cinema 4D's **HPB** convention, in radians:
///
/// * `rotation.x` — **H**eading, about +Y
/// * `rotation.y` — **P**itch, about +X
/// * `rotation.z` — **B**ank, about +Z
///
/// applied intrinsically in that order (H, then P, then B). Keeping Euler angles as
/// the source of truth means values typed by the user stay exactly as typed
/// (e.g. `370°` remains `370°` instead of being normalised by a quaternion).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub position: Vec3,
    /// HPB Euler angles in radians.
    pub rotation: Vec3,
    pub scale: Vec3,
}

impl Transform {
    pub const IDENTITY: Self = Self {
        position: Vec3::ZERO,
        rotation: Vec3::ZERO,
        scale: Vec3::ONE,
    };

    #[must_use]
    pub const fn from_position(position: Vec3) -> Self {
        Self {
            position,
            rotation: Vec3::ZERO,
            scale: Vec3::ONE,
        }
    }

    /// Sets the rotation from HPB angles given in **degrees**.
    #[must_use]
    pub fn with_rotation_deg(mut self, hpb_degrees: Vec3) -> Self {
        self.rotation = Vec3::new(
            hpb_degrees.x.to_radians(),
            hpb_degrees.y.to_radians(),
            hpb_degrees.z.to_radians(),
        );
        self
    }

    #[must_use]
    pub fn rotation_quat(&self) -> Quat {
        Quat::from_euler(EulerRot::YXZ, self.rotation.x, self.rotation.y, self.rotation.z)
    }

    /// Replaces the rotation with the HPB decomposition of `q`.
    pub fn set_rotation_quat(&mut self, q: Quat) {
        let (h, p, b) = q.normalize().to_euler(EulerRot::YXZ);
        // `+ 0.0` turns -0.0 into 0.0 so the UI never shows "-0.0°".
        self.rotation = Vec3::new(h, p, b) + Vec3::ZERO;
    }

    /// Local-to-parent matrix: `T * R * S`.
    #[must_use]
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation_quat(), self.position)
    }

    /// Decomposes an affine matrix (without shear) back into a transform.
    #[must_use]
    pub fn from_matrix(m: &Mat4) -> Self {
        let (scale, rotation, position) = m.to_scale_rotation_translation();
        let mut t = Self {
            position,
            rotation: Vec3::ZERO,
            scale,
        };
        t.set_rotation_quat(rotation);
        t
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hpb_round_trips_through_quaternion() {
        let original = Transform::IDENTITY.with_rotation_deg(Vec3::new(35.0, -20.0, 10.0));
        let mut copy = Transform::IDENTITY;
        copy.set_rotation_quat(original.rotation_quat());
        assert!(original.rotation.abs_diff_eq(copy.rotation, 1e-5));
    }

    #[test]
    fn heading_rotates_about_y() {
        let t = Transform::IDENTITY.with_rotation_deg(Vec3::new(90.0, 0.0, 0.0));
        let x = t.matrix().transform_vector3(Vec3::X);
        assert!(x.abs_diff_eq(-Vec3::Z, 1e-5), "{x:?}");
    }

    #[test]
    fn matrix_round_trip() {
        let t = Transform {
            position: Vec3::new(10.0, -4.0, 3.0),
            rotation: Vec3::new(0.3, 0.2, -0.1),
            scale: Vec3::new(2.0, 1.0, 0.5),
        };
        let back = Transform::from_matrix(&t.matrix());
        assert!(t.position.abs_diff_eq(back.position, 1e-4));
        assert!(t.rotation.abs_diff_eq(back.rotation, 1e-4));
        assert!(t.scale.abs_diff_eq(back.scale, 1e-4));
    }
}
