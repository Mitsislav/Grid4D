use std::f32::consts::FRAC_PI_2;

use glam::{Mat4, Quat, Vec2, Vec3, camera::rh};

/// A ray in world space. `dir` is normalised.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

/// How a camera maps the world to the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Projection {
    /// Things further away look smaller.
    #[default]
    Perspective,
    /// Orthographic: rays are parallel, sizes do not change with depth; the
    /// visible height is `2 · distance · tan(fov_y / 2)`, so zooming still changes
    /// `distance`.
    Parallel,
}

/// Turntable editor camera orbiting around a target point.
///
/// Navigation mirrors Cinema 4D: Alt+LMB orbits, Alt+MMB pans, Alt+RMB dollies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitCamera {
    pub target: Vec3,
    pub distance: f32,
    /// Rotation about world +Y, radians.
    pub yaw: f32,
    /// Elevation above the XZ plane, radians (clamped short of the poles).
    pub pitch: f32,
    /// Bank: rotation about the viewing direction, radians (0 keeps world Y up).
    /// Set when the view looks through a scene camera; navigation keeps it.
    pub roll: f32,
    /// Vertical field of view, radians.
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
    pub projection: Projection,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: Vec3::new(0.0, 80.0, 0.0),
            distance: 1500.0,
            yaw: 0.62,
            pitch: 0.42,
            roll: 0.0,
            fov_y: 40f32.to_radians(),
            near: 2.0,
            far: 200_000.0,
            projection: Projection::Perspective,
        }
    }
}

impl OrbitCamera {
    const MIN_DISTANCE: f32 = 1.0;
    const MAX_DISTANCE: f32 = 100_000.0;
    const PITCH_LIMIT: f32 = FRAC_PI_2 - 0.001;
    /// A parallel camera's eye sits this far behind its target, so the scene in
    /// front of the target and well behind it is in view.
    const PARALLEL_EYE: f32 = 50_000.0;

    #[must_use]
    pub fn is_parallel(&self) -> bool {
        self.projection == Projection::Parallel
    }

    /// Half the visible height at the target (what a parallel camera shows).
    #[must_use]
    pub fn half_height(&self) -> f32 {
        self.distance * (self.fov_y * 0.5).tan()
    }

    /// Unit vector from the target towards the eye.
    #[must_use]
    pub fn back(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(cp * sy, sp, cp * cy)
    }

    #[must_use]
    pub fn eye(&self) -> Vec3 {
        match self.projection {
            Projection::Perspective => self.target + self.back() * self.distance,
            Projection::Parallel => self.target + self.back() * Self::PARALLEL_EYE,
        }
    }

    #[must_use]
    pub fn forward(&self) -> Vec3 {
        -self.back()
    }

    /// Screen right: level (world Y up) turned by the bank; well defined even when
    /// looking straight down or up.
    #[must_use]
    pub fn right(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let level = Vec3::new(cy, 0.0, -sy);
        if self.roll == 0.0 {
            level
        } else {
            Quat::from_axis_angle(self.forward(), self.roll) * level
        }
    }

    #[must_use]
    pub fn up(&self) -> Vec3 {
        self.right().cross(self.forward())
    }

    #[must_use]
    pub fn view(&self) -> Mat4 {
        rh::view::look_at_mat4(self.eye(), self.target, self.up())
    }

    /// The view as an object placed in the world: at the eye, looking along its
    /// −Z with its +Y up (Grid4D's camera axes), unit scale.
    #[must_use]
    pub fn eye_matrix(&self) -> Mat4 {
        Mat4::from_cols(
            self.right().extend(0.0),
            self.up().extend(0.0),
            self.back().extend(0.0),
            self.eye().extend(1.0),
        )
    }

    /// The view of an eye placed by `world` (looking along its −Z, up its +Y; scale
    /// ignored), with field of view `fov_y` (radians) and the orbit target
    /// `distance` ahead; near and far planes as the default view's.
    #[must_use]
    pub fn from_world(world: &Mat4, fov_y: f32, distance: f32) -> Self {
        let (_, rotation, eye) = world.to_scale_rotation_translation();
        let forward = (rotation * Vec3::NEG_Z).normalize_or(Vec3::NEG_Z);
        let up = (rotation * Vec3::Y).normalize_or(Vec3::Y);
        let back = -forward;
        let mut camera = Self {
            distance: distance.clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE),
            yaw: back.x.atan2(back.z).rem_euclid(std::f32::consts::TAU),
            pitch: back
                .y
                .clamp(-1.0, 1.0)
                .asin()
                .clamp(-Self::PITCH_LIMIT, Self::PITCH_LIMIT),
            roll: 0.0,
            fov_y: if fov_y.is_finite() && fov_y > 0.0 {
                fov_y
            } else {
                Self::default().fov_y
            },
            ..Self::default()
        };
        // Bank: the angle from the level up vector to the object's, about the view axis.
        let level = camera.up();
        let axis = camera.forward();
        camera.roll = axis.dot(level.cross(up)).atan2(level.dot(up));
        camera.target = eye + camera.forward() * camera.distance;
        camera
    }

    /// Right-handed perspective or orthographic projection with wgpu's `[0, 1]`
    /// depth range.
    #[must_use]
    pub fn projection(&self, aspect: f32) -> Mat4 {
        let aspect = aspect.max(1e-4);
        match self.projection {
            Projection::Perspective => rh::proj::directx::perspective(self.fov_y, aspect, self.near, self.far),
            Projection::Parallel => {
                let h = self.half_height().max(1e-3);
                let w = h * aspect;
                rh::proj::directx::orthographic(-w, w, -h, h, 1.0, Self::PARALLEL_EYE + self.far)
            }
        }
    }

    #[must_use]
    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        self.projection(aspect) * self.view()
    }

    /// Orbit by angle deltas (radians).
    pub fn orbit(&mut self, delta_yaw: f32, delta_pitch: f32) {
        self.yaw = (self.yaw + delta_yaw).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch + delta_pitch).clamp(-Self::PITCH_LIMIT, Self::PITCH_LIMIT);
    }

    /// Pan so that content under the cursor follows a drag of `delta_px` pixels.
    pub fn pan(&mut self, delta_px: Vec2, viewport_height_px: f32) {
        let world_per_px = self.world_units_per_pixel(self.distance, viewport_height_px);
        self.target += (-self.right() * delta_px.x + self.up() * delta_px.y) * world_per_px;
    }

    /// Multiplies the distance to the target (`factor < 1` moves closer).
    pub fn dolly(&mut self, factor: f32) {
        if factor.is_finite() && factor > 0.0 {
            self.distance = (self.distance * factor).clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE);
        }
    }

    /// Size in world units of one pixel at `depth` along the view direction (the
    /// same at every depth for a parallel camera).
    #[must_use]
    pub fn world_units_per_pixel(&self, depth: f32, viewport_height_px: f32) -> f32 {
        let depth = match self.projection {
            Projection::Perspective => depth,
            Projection::Parallel => self.distance,
        };
        2.0 * depth * (self.fov_y * 0.5).tan() / viewport_height_px.max(1.0)
    }

    /// Centres the view on a bounding sphere so that it fills the viewport.
    pub fn frame_sphere(&mut self, center: Vec3, radius: f32) {
        self.target = center;
        let radius = radius.max(10.0);
        let distance = match self.projection {
            Projection::Perspective => radius / (self.fov_y * 0.5).sin() * 1.15,
            // The visible half-height is the radius, with the same margin.
            Projection::Parallel => radius * 1.15 / (self.fov_y * 0.5).tan(),
        };
        self.distance = distance.clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE);
    }

    /// Picking ray through a point in normalised device coordinates (`[-1, 1]`, Y up).
    #[must_use]
    pub fn ray_from_ndc(&self, ndc: Vec2, aspect: f32) -> Ray {
        let half_h = (self.fov_y * 0.5).tan();
        match self.projection {
            Projection::Perspective => {
                let dir = self.forward() + self.right() * (ndc.x * half_h * aspect) + self.up() * (ndc.y * half_h);
                Ray {
                    origin: self.eye(),
                    dir: dir.normalize_or(self.forward()),
                }
            }
            Projection::Parallel => {
                let h = self.half_height();
                Ray {
                    origin: self.eye() + self.right() * (ndc.x * h * aspect) + self.up() * (ndc.y * h),
                    dir: self.forward(),
                }
            }
        }
    }

    /// Adaptive grid spacing (a power of ten, ≥ 1 cm) like C4D's "Grid Spacing" HUD.
    #[must_use]
    pub fn grid_spacing(&self) -> f32 {
        10f32.powf((self.distance * 0.1).max(1.0).log10().floor())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_projects_to_screen_center() {
        let cam = OrbitCamera::default();
        let p = cam.view_projection(16.0 / 9.0).project_point3(cam.target);
        assert!(p.truncate().abs_diff_eq(Vec2::ZERO, 1e-4), "{p:?}");
        assert!((0.0..=1.0).contains(&p.z));
    }

    #[test]
    fn center_ray_hits_target() {
        let cam = OrbitCamera::default();
        let ray = cam.ray_from_ndc(Vec2::ZERO, 1.5);
        let hit = ray.origin + ray.dir * cam.distance;
        assert!(hit.abs_diff_eq(cam.target, 1e-2), "{hit:?}");
    }

    #[test]
    fn a_world_placement_round_trips_through_the_view() {
        let cam = OrbitCamera {
            yaw: 2.1,
            pitch: -0.4,
            roll: 0.3,
            distance: 700.0,
            target: Vec3::new(10.0, -20.0, 30.0),
            ..Default::default()
        };
        let world = cam.eye_matrix();
        assert!(world.determinant() > 0.99, "a right-handed rotation");
        let back = OrbitCamera::from_world(&world, cam.fov_y, cam.distance);
        assert!(
            back.eye().abs_diff_eq(cam.eye(), 1e-2),
            "{} vs {}",
            back.eye(),
            cam.eye()
        );
        assert!(back.forward().abs_diff_eq(cam.forward(), 1e-4));
        assert!(back.up().abs_diff_eq(cam.up(), 1e-4), "the bank survives");
        assert!((back.roll - 0.3).abs() < 1e-4);
        assert!(back.view().abs_diff_eq(cam.view(), 1e-3));

        // An object turned 90° about Y looks along −X; scale is ignored.
        let turned = Mat4::from_scale_rotation_translation(
            Vec3::splat(3.0),
            Quat::from_rotation_y(FRAC_PI_2),
            Vec3::new(0.0, 100.0, 0.0),
        );
        let view = OrbitCamera::from_world(&turned, 0.5, 400.0);
        assert!(view.forward().abs_diff_eq(Vec3::NEG_X, 1e-5), "{}", view.forward());
        assert!(view.eye().abs_diff_eq(Vec3::new(0.0, 100.0, 0.0), 1e-3));
        assert!(view.roll.abs() < 1e-5);
        assert_eq!(view.fov_y, 0.5);
    }

    #[test]
    fn a_banked_view_rays_and_pans_along_its_own_axes() {
        let cam = OrbitCamera {
            roll: FRAC_PI_2,
            ..Default::default()
        };
        let level = OrbitCamera::default();
        // A quarter bank: screen up is the level view's right (or left).
        assert!(cam.up().dot(level.right()).abs() > 0.999, "{}", cam.up());
        let ray = cam.ray_from_ndc(Vec2::new(0.0, 0.5), 1.0);
        let level_ray = level.ray_from_ndc(Vec2::new(0.5, 0.0), 1.0);
        assert!(
            ray.dir.abs_diff_eq(level_ray.dir, 1e-4)
                || ray
                    .dir
                    .abs_diff_eq(level.ray_from_ndc(Vec2::new(-0.5, 0.0), 1.0).dir, 1e-4)
        );
        let p = cam.view_projection(1.0).project_point3(cam.target);
        assert!(p.truncate().abs_diff_eq(Vec2::ZERO, 1e-4), "the target stays centred");
    }

    #[test]
    fn parallel_views_keep_sizes_and_cast_parallel_rays() {
        // Looking straight down (the Top view): X right, −Z up on screen.
        let top = OrbitCamera {
            yaw: 0.0,
            pitch: FRAC_PI_2,
            target: Vec3::ZERO,
            distance: 1000.0,
            projection: Projection::Parallel,
            ..Default::default()
        };
        assert!(top.forward().abs_diff_eq(Vec3::NEG_Y, 1e-6));
        assert!(top.right().abs_diff_eq(Vec3::X, 1e-6));
        assert!(top.up().abs_diff_eq(Vec3::NEG_Z, 1e-6));
        let vp = top.view_projection(1.0);
        let screen = |p: Vec3| vp.project_point3(p).truncate();
        // Equal sizes at any height: no perspective.
        let near = screen(Vec3::new(100.0, 500.0, 0.0)) - screen(Vec3::new(0.0, 500.0, 0.0));
        let far = screen(Vec3::new(100.0, -500.0, 0.0)) - screen(Vec3::new(0.0, -500.0, 0.0));
        assert!(near.abs_diff_eq(far, 1e-5), "{near} vs {far}");
        assert!(screen(Vec3::new(0.0, 0.0, -100.0)).y > 0.0, "−Z is up");
        let a = top.ray_from_ndc(Vec2::new(0.5, 0.0), 1.0);
        let b = top.ray_from_ndc(Vec2::new(-0.5, 0.3), 1.0);
        assert_eq!(a.dir, b.dir, "parallel rays");
        let hit = a.origin + a.dir * (a.origin.y / -a.dir.y);
        assert!(
            hit.abs_diff_eq(Vec3::new(top.half_height() * 0.5, 0.0, 0.0), 1e-2),
            "{hit}"
        );
        assert_eq!(
            top.world_units_per_pixel(1.0, 100.0),
            top.world_units_per_pixel(9_000.0, 100.0),
            "the same at every depth"
        );
        let mut framed = top;
        framed.frame_sphere(Vec3::ZERO, 200.0);
        assert!((framed.half_height() - 230.0).abs() < 1e-2, "{}", framed.half_height());
    }

    #[test]
    fn grid_spacing_is_power_of_ten() {
        let mut cam = OrbitCamera {
            distance: 1500.0,
            ..Default::default()
        };
        assert_eq!(cam.grid_spacing(), 100.0);
        cam.distance = 90.0;
        assert_eq!(cam.grid_spacing(), 1.0);
    }
}
