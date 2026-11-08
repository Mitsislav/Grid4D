//! What the picture is seen through.

use std::f32::consts::TAU;

use glam::{Mat4, Quat, Vec2, Vec3, camera::rh};

/// How a camera projects the scene.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CameraProjection {
    /// Vertical field of view, radians.
    Perspective { fov_y: f32 },
    /// Half the visible height, world units.
    Parallel { half_height: f32 },
}

/// A thin lens: depth of field (see `docs/reference/render-lens.md`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThinLens {
    /// The aperture's radius (world units; a bladed one's corners lie on it).
    pub radius: f32,
    /// The distance along the camera's axis that is sharp.
    pub focus_distance: f32,
    /// The aperture's blades: 0 for a round one, else 3 or more.
    pub blades: u32,
    /// Turns the bladed aperture (radians).
    pub rotation: f32,
}

impl ThinLens {
    /// A point on the aperture (in its own plane, world units) from `u`, `v`
    /// in [0, 1): evenly over the disc or the polygon.
    #[must_use]
    pub fn aperture_point(&self, u: f32, v: f32) -> Vec2 {
        if self.blades < 3 {
            let (r, phi) = (u.sqrt() * self.radius, TAU * v);
            return Vec2::new(r * phi.cos(), r * phi.sin());
        }
        // One of the polygon's triangles (centre and an edge), then a point
        // evenly in it.
        let n = f32::from(u16::try_from(self.blades).unwrap_or(u16::MAX));
        let scaled = (u * n).min(n - 1e-3);
        let k = scaled.floor();
        let w = scaled - k;
        let a0 = self.rotation + TAU * k / n;
        let a1 = a0 + TAU / n;
        let (p0, p1) = (
            Vec2::new(a0.cos(), a0.sin()) * self.radius,
            Vec2::new(a1.cos(), a1.sin()) * self.radius,
        );
        let s = w.sqrt();
        p0 * (s * (1.0 - v)) + p1 * (s * v)
    }
}

/// A camera for the renderer: a placement in the world (looking along its −Z
/// axis, +Y up; scale is ignored) and a projection; optionally a thin lens
/// and a move over the shutter interval.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderCamera {
    /// Where it is (at the shutter's opening when it moves).
    pub world: Mat4,
    pub projection: CameraProjection,
    /// Depth of field (a perspective camera's only); `None`: a pinhole.
    pub lens: Option<ThinLens>,
    /// Where it is at the shutter's closing; `None`: it stands still. In
    /// between it moves straight, turning evenly.
    pub end: Option<Mat4>,
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
        Self::still(view.inverse(), CameraProjection::Perspective { fov_y })
    }

    /// A pinhole camera at `world` that stands still.
    #[must_use]
    pub const fn still(world: Mat4, projection: CameraProjection) -> Self {
        Self {
            world,
            projection,
            lens: None,
            end: None,
        }
    }

    /// Where it is at `time` (0 – 1 over the shutter interval).
    #[must_use]
    pub fn world_at(&self, time: f32) -> Mat4 {
        let Some(end) = self.end else {
            return self.world;
        };
        let (s0, r0, t0) = self.world.to_scale_rotation_translation();
        let (s1, r1, t1) = end.to_scale_rotation_translation();
        let r1 = if r0.dot(r1) < 0.0 { -r1 } else { r1 };
        Mat4::from_scale_rotation_translation(s0.lerp(s1, time), Quat::slerp(r0, r1, time), t0.lerp(t1, time))
    }

    /// The ray of a camera sample through the picture point (`x`, `y`) (as
    /// [`Self::ray`]) at `time`, from the aperture point `aperture` (`u`,
    /// `v` in [0, 1); used only with a lens): aimed at its point on the
    /// focus plane, so that plane is sharp and the rest blurs.
    pub(crate) fn sample_ray(
        &self,
        x: f32,
        y: f32,
        aspect: f32,
        time: f32,
        aperture: Option<(f32, f32)>,
    ) -> (Vec3, Vec3) {
        let camera = if self.end.is_some() {
            Self {
                world: self.world_at(time),
                ..*self
            }
        } else {
            *self
        };
        let (origin, dir) = camera.ray(x, y, aspect);
        let (Some(lens), Some((u, v)), CameraProjection::Perspective { .. }) = (self.lens, aperture, self.projection)
        else {
            return (origin, dir);
        };
        let (_, right, up, forward) = camera.frame();
        let along = dir.dot(forward);
        if along <= f32::MIN_POSITIVE {
            return (origin, dir);
        }
        let focus = origin + dir * (lens.focus_distance / along);
        let offset = lens.aperture_point(u, v);
        let start = origin + right * offset.x + up * offset.y;
        (start, (focus - start).normalize_or(dir))
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
    pub(crate) fn screen(&self, p: Vec3, aspect: f32) -> Option<Vec2> {
        let (origin, right, up, forward) = self.frame();
        let d = p - origin;
        let (x, y, depth) = (d.dot(right), d.dot(up), d.dot(forward));
        match self.projection {
            CameraProjection::Perspective { fov_y } => {
                let h = (fov_y * 0.5).tan();
                (depth.abs() > f32::MIN_POSITIVE && h > 0.0)
                    .then(|| Vec2::new(x / (depth * h * aspect), y / (depth * h)))
            }
            CameraProjection::Parallel { half_height } => {
                (half_height > 0.0).then(|| Vec2::new(x / (half_height * aspect), y / half_height))
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
            lens: None,
            end: None,
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
            assert!(at.abs_diff_eq(Vec2::new(0.5, -0.25), 1e-5), "{at}");
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

    #[test]
    fn apertures_are_discs_or_polygons_of_the_radius() {
        let round = ThinLens {
            radius: 2.0,
            focus_distance: 100.0,
            blades: 0,
            rotation: 0.0,
        };
        let hexagon = ThinLens { blades: 6, ..round };
        let mut far_corner: f32 = 0.0;
        let mut far_edge: f32 = 0.0;
        for i in 0..64u16 {
            for j in 0..64u16 {
                let (u, v) = ((f32::from(i) + 0.5) / 64.0, (f32::from(j) + 0.5) / 64.0);
                assert!(round.aperture_point(u, v).length() <= 2.0 + 1e-4);
                let p = hexagon.aperture_point(u, v);
                assert!(p.length() <= 2.0 + 1e-4);
                // Towards a corner (0°) it reaches the radius; towards an
                // edge's middle (30°) only the radius × cos 30°.
                let angle = p.y.atan2(p.x).to_degrees();
                if angle.abs() < 1.0 {
                    far_corner = far_corner.max(p.length());
                }
                if (angle - 30.0).abs() < 1.0 {
                    far_edge = far_edge.max(p.length());
                }
            }
        }
        assert!(far_corner > 1.9, "{far_corner}");
        assert!((far_edge - 2.0 * 30f32.to_radians().cos()).abs() < 0.05, "{far_edge}");
    }

    #[test]
    fn a_moving_camera_is_between_its_ends() {
        let camera = RenderCamera {
            end: Some(Mat4::from_translation(Vec3::new(100.0, 0.0, 0.0))),
            ..RenderCamera::still(Mat4::IDENTITY, CameraProjection::Perspective { fov_y: 1.0 })
        };
        assert!(
            camera
                .world_at(0.25)
                .w_axis
                .truncate()
                .abs_diff_eq(Vec3::new(25.0, 0.0, 0.0), 1e-4)
        );
        let (origin, _) = camera.sample_ray(0.0, 0.0, 1.0, 1.0, None);
        assert!(origin.abs_diff_eq(Vec3::new(100.0, 0.0, 0.0), 1e-4));
    }
}
