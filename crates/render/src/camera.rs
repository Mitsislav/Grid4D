use std::f32::consts::FRAC_PI_2;

use glam::{Mat4, Vec2, Vec3, camera::rh};

/// A ray in world space. `dir` is normalised.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
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
    /// Vertical field of view, radians.
    pub fov_y: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: Vec3::new(0.0, 80.0, 0.0),
            distance: 1500.0,
            yaw: 0.62,
            pitch: 0.42,
            fov_y: 40f32.to_radians(),
            near: 2.0,
            far: 200_000.0,
        }
    }
}

impl OrbitCamera {
    const MIN_DISTANCE: f32 = 1.0;
    const MAX_DISTANCE: f32 = 100_000.0;
    const PITCH_LIMIT: f32 = FRAC_PI_2 - 0.001;

    /// Unit vector from the target towards the eye.
    #[must_use]
    pub fn back(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(cp * sy, sp, cp * cy)
    }

    #[must_use]
    pub fn eye(&self) -> Vec3 {
        self.target + self.back() * self.distance
    }

    #[must_use]
    pub fn forward(&self) -> Vec3 {
        -self.back()
    }

    #[must_use]
    pub fn right(&self) -> Vec3 {
        self.forward().cross(Vec3::Y).normalize_or(Vec3::X)
    }

    #[must_use]
    pub fn up(&self) -> Vec3 {
        self.right().cross(self.forward())
    }

    #[must_use]
    pub fn view(&self) -> Mat4 {
        rh::view::look_at_mat4(self.eye(), self.target, Vec3::Y)
    }

    /// Right-handed perspective with wgpu's `[0, 1]` depth range.
    #[must_use]
    pub fn projection(&self, aspect: f32) -> Mat4 {
        rh::proj::directx::perspective(self.fov_y, aspect.max(1e-4), self.near, self.far)
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

    /// Size in world units of one pixel at `depth` along the view direction.
    #[must_use]
    pub fn world_units_per_pixel(&self, depth: f32, viewport_height_px: f32) -> f32 {
        2.0 * depth * (self.fov_y * 0.5).tan() / viewport_height_px.max(1.0)
    }

    /// Centres the view on a bounding sphere so that it fills the viewport.
    pub fn frame_sphere(&mut self, center: Vec3, radius: f32) {
        self.target = center;
        let radius = radius.max(10.0);
        self.distance = (radius / (self.fov_y * 0.5).sin() * 1.15).clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE);
    }

    /// Picking ray through a point in normalised device coordinates (`[-1, 1]`, Y up).
    #[must_use]
    pub fn ray_from_ndc(&self, ndc: Vec2, aspect: f32) -> Ray {
        let half_h = (self.fov_y * 0.5).tan();
        let dir = self.forward() + self.right() * (ndc.x * half_h * aspect) + self.up() * (ndc.y * half_h);
        Ray {
            origin: self.eye(),
            dir: dir.normalize_or(self.forward()),
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
