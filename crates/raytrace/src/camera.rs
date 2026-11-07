//! What the picture is seen through.

use glam::{Mat4, Vec3, camera::rh};

/// How a camera projects the scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CameraProjection {
    /// Vertical field of view, radians.
    Perspective { fov_y: f32 },
    /// Half the visible height, world units.
    Parallel { half_height: f32 },
}

/// A camera for the renderer: a placement in the world (looking along its −Z
/// axis, +Y up; scale is ignored) and a projection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderCamera {
    pub world: Mat4,
    pub projection: CameraProjection,
}

impl RenderCamera {
    /// A perspective camera with field of view `fov_y` that frames the box
    /// `min`…`max`, looking at it from the front, a little from above and to the
    /// side (like a new view).
    #[must_use]
    pub fn framing(min: Vec3, max: Vec3, fov_y: f32) -> Self {
        let centre = (min + max) * 0.5;
        let radius = ((max - min).length() * 0.5).max(1.0);
        let distance = radius / (fov_y * 0.5).sin().max(0.05) * 1.1;
        let back = Vec3::new(0.45, 0.35, 0.82).normalize();
        let eye = centre + back * distance;
        let view = rh::view::look_at_mat4(eye, centre, Vec3::Y);
        Self {
            world: view.inverse(),
            projection: CameraProjection::Perspective { fov_y },
        }
    }

    /// The camera's position and unit axes: right, up and forward (its −Z).
    pub(crate) fn frame(&self) -> (Vec3, Vec3, Vec3, Vec3) {
        let origin = self.world.w_axis.truncate();
        let right = self.world.x_axis.truncate().normalize_or(Vec3::X);
        let up = self.world.y_axis.truncate().normalize_or(Vec3::Y);
        let forward = -self.world.z_axis.truncate().normalize_or(Vec3::Z);
        (origin, right, up, forward)
    }

    /// Where world point `p` lies in the picture, in normalised device
    /// coordinates (−1…1, +y up) of a picture `aspect` wide per unit of height;
    /// `None` for a point at the eye's plane of a perspective camera.
    pub(crate) fn screen(&self, p: Vec3, aspect: f32) -> Option<glam::Vec2> {
        let (origin, right, up, forward) = self.frame();
        let d = p - origin;
        let (x, y, depth) = (d.dot(right), d.dot(up), d.dot(forward));
        match self.projection {
            CameraProjection::Perspective { fov_y } => {
                let h = (fov_y * 0.5).tan();
                (depth.abs() > f32::MIN_POSITIVE && h > 0.0)
                    .then(|| glam::Vec2::new(x / (depth * h * aspect), y / (depth * h)))
            }
            CameraProjection::Parallel { half_height } => {
                (half_height > 0.0).then(|| glam::Vec2::new(x / (half_height * aspect), y / half_height))
            }
        }
    }

    /// The ray through the picture point (`x`, `y`) in normalised device
    /// coordinates (−1…1, +y up) of a picture `aspect` wide per unit of height.
    pub(crate) fn ray(&self, x: f32, y: f32, aspect: f32) -> (Vec3, Vec3) {
        let (origin, right, up, forward) = self.frame();
        match self.projection {
            CameraProjection::Perspective { fov_y } => {
                let h = (fov_y * 0.5).tan();
                let dir = (forward + right * (x * h * aspect) + up * (y * h)).normalize_or(forward);
                (origin, dir)
            }
            CameraProjection::Parallel { half_height } => {
                let start = origin + right * (x * half_height * aspect) + up * (y * half_height);
                (start, forward)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rays_leave_through_the_picture() {
        let camera = RenderCamera {
            world: Mat4::from_translation(Vec3::new(0.0, 0.0, 10.0)),
            projection: CameraProjection::Perspective {
                fov_y: 90f32.to_radians(),
            },
        };
        let (origin, centre) = camera.ray(0.0, 0.0, 2.0);
        assert!(origin.abs_diff_eq(Vec3::new(0.0, 0.0, 10.0), 1e-6) && centre.abs_diff_eq(Vec3::NEG_Z, 1e-6));
        // The top edge at 45° up, the right edge twice as far out (aspect 2).
        let (_, top) = camera.ray(0.0, 1.0, 2.0);
        assert!(top.abs_diff_eq(Vec3::new(0.0, 1.0, -1.0).normalize(), 1e-6));
        let (_, right) = camera.ray(1.0, 0.0, 2.0);
        assert!(right.abs_diff_eq(Vec3::new(2.0, 0.0, -1.0).normalize(), 1e-6));

        let parallel = RenderCamera {
            projection: CameraProjection::Parallel { half_height: 5.0 },
            ..camera
        };
        let (start, dir) = parallel.ray(1.0, -1.0, 2.0);
        assert!(start.abs_diff_eq(Vec3::new(10.0, -5.0, 10.0), 1e-5) && dir.abs_diff_eq(Vec3::NEG_Z, 1e-6));

        // Points along a ray land where the ray left the picture.
        for cam in [camera, parallel] {
            let (origin, dir) = cam.ray(0.5, -0.25, 2.0);
            let at = cam.screen(origin + dir * 7.0, 2.0).unwrap();
            assert!(at.abs_diff_eq(glam::Vec2::new(0.5, -0.25), 1e-5), "{at}");
        }
        assert!(
            camera.screen(Vec3::new(1.0, 0.0, 10.0), 2.0).is_none(),
            "at the eye's plane"
        );
    }

    #[test]
    fn framing_looks_at_the_box() {
        let camera = RenderCamera::framing(Vec3::splat(-100.0), Vec3::splat(100.0), 40f32.to_radians());
        let (origin, centre) = camera.ray(0.0, 0.0, 1.5);
        let towards = (Vec3::ZERO - origin).normalize();
        assert!(centre.abs_diff_eq(towards, 1e-5), "{centre} vs {towards}");
        assert!(origin.length() > 173.0, "outside the box");
    }
}
