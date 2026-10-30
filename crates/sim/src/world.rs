//! A physics world: bodies made of colliders, stepped frame by frame. The
//! simulation of a document ([`crate::Simulation`]) builds one from the
//! evaluated scene.

use glam::{Quat, Vec3};
use grid4d_scene::{Body, Motion, SimulationSettings};
use rapier3d::prelude::{ColliderBuilder, PhysicsWorld, Pose, RigidBodyBuilder, RigidBodyHandle};

use crate::convert::{from_engine, length_to_engine, rotation_from_engine, rotation_to_engine, to_engine};
use crate::shape;

/// A body in a [`World`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BodyId(RigidBodyHandle);

/// A rapier world set up from a document's simulation settings.
pub struct World {
    inner: PhysicsWorld,
    steps_per_frame: u32,
}

impl std::fmt::Debug for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("World")
            .field("bodies", &self.inner.bodies.len())
            .field("steps_per_frame", &self.steps_per_frame)
            .finish_non_exhaustive()
    }
}

impl World {
    /// An empty world for a document at `fps` frames per second.
    #[must_use]
    pub fn new(settings: &SimulationSettings, fps: u32) -> Self {
        let settings = settings.validated();
        let mut inner = PhysicsWorld::new();
        inner.gravity = to_engine(settings.gravity);
        let frame = settings.time_scale / f32::from(u16::try_from(fps.max(1)).unwrap_or(u16::MAX));
        inner.integration_parameters.dt = frame / f32::from(u16::try_from(settings.steps_per_frame).unwrap_or(1));
        inner.integration_parameters.num_solver_iterations =
            usize::try_from(settings.solver_iterations).unwrap_or(1).max(1);
        Self {
            inner,
            steps_per_frame: settings.steps_per_frame,
        }
    }

    /// Sub-steps in a frame.
    #[must_use]
    pub const fn steps_per_frame(&self) -> u32 {
        self.steps_per_frame
    }

    /// Adds a body shaped like a ball of `radius` (cm), centred at `position`.
    pub fn add_ball(&mut self, position: Vec3, rotation: Quat, radius: f32, body: &Body) -> BodyId {
        let ball = ColliderBuilder::ball(length_to_engine(radius));
        self.insert(position, rotation, body, vec![ball])
    }

    /// Adds a body shaped like a box of half-sizes `half` (cm), centred at `position`.
    pub fn add_box(&mut self, position: Vec3, rotation: Quat, half: Vec3, body: &Body) -> BodyId {
        let [x, y, z] = half.abs().to_array().map(length_to_engine);
        self.insert(position, rotation, body, vec![ColliderBuilder::cuboid(x, y, z)])
    }

    /// Adds a body at `position` (cm) made of `colliders` (in its own space,
    /// metres), with `body`'s surface and mass.
    pub(crate) fn insert(
        &mut self,
        position: Vec3,
        rotation: Quat,
        body: &Body,
        colliders: Vec<ColliderBuilder>,
    ) -> BodyId {
        let body = body.validated();
        let builder = match body.motion {
            Motion::Dynamic => RigidBodyBuilder::dynamic()
                .linvel(to_engine(body.initial_velocity))
                .angvel(rapier3d::prelude::Vector::from_array(body.initial_spin.to_array()))
                .linear_damping(damping_rate(body.linear_damping))
                .angular_damping(damping_rate(body.angular_damping)),
            // Moved only by its animation: kinematic.
            Motion::Collider => RigidBodyBuilder::kinematic_position_based(),
        }
        .pose(Pose::from_parts(to_engine(position), rotation_to_engine(rotation)));
        let handle = self.inner.insert_body(builder);
        let total = shape::volume_mass(&colliders);
        for collider in colliders {
            let collider = shape::surface(collider, &body, total);
            self.inner.insert_collider(collider, Some(handle));
        }
        BodyId(handle)
    }

    /// Where a collider body should be at the end of the next sub-step.
    pub fn move_kinematic(&mut self, body: BodyId, position: Vec3, rotation: Quat) {
        if let Some(b) = self.inner.bodies.get_mut(body.0) {
            b.set_next_kinematic_position(Pose::from_parts(to_engine(position), rotation_to_engine(rotation)));
        }
    }

    /// Advances one sub-step.
    pub fn substep(&mut self) {
        self.inner.step();
    }

    /// Advances one frame (its sub-steps).
    pub fn step_frame(&mut self) {
        for _ in 0..self.steps_per_frame {
            self.substep();
        }
    }

    /// Where `body` is (cm) and how it is turned.
    #[must_use]
    pub fn pose(&self, body: BodyId) -> Option<(Vec3, Quat)> {
        let body = self.inner.bodies.get(body.0)?;
        Some((from_engine(body.translation()), rotation_from_engine(*body.rotation())))
    }

    /// How fast `body` moves (cm/s).
    #[must_use]
    pub fn velocity(&self, body: BodyId) -> Option<Vec3> {
        self.inner.bodies.get(body.0).map(|b| from_engine(b.linvel()))
    }

    /// The mass of `body` (kg).
    #[must_use]
    pub fn mass(&self, body: BodyId) -> Option<f32> {
        self.inner.bodies.get(body.0).map(rapier3d::prelude::RigidBody::mass)
    }
}

/// The engine's damping rate for a fraction of speed lost per second
/// (1.0 = all of it: a very high rate).
fn damping_rate(fraction: f32) -> f32 {
    let kept = (1.0 - fraction.clamp(0.0, 1.0)).max(1e-3);
    -kept.ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dropped_ball_falls_as_gravity_says() {
        let settings = SimulationSettings::default();
        let mut world = World::new(&settings, 30);
        let start = Vec3::new(0.0, 1000.0, 0.0);
        let ball = world.add_ball(start, Quat::IDENTITY, 10.0, &Body::rigid());
        for _ in 0..30 {
            world.step_frame();
        }
        // One second of free fall: ½ g t² = 490.5 cm, at 981 cm/s.
        let (at, _) = world.pose(ball).unwrap();
        let fallen = start.y - at.y;
        assert!((fallen - 490.5).abs() < 10.0, "fell {fallen} cm");
        let speed = world.velocity(ball).unwrap().y;
        assert!((speed + 981.0).abs() < 5.0, "{speed} cm/s");
    }

    #[test]
    fn a_ball_comes_to_rest_on_a_collider_and_identical_runs_agree() {
        let run = || {
            let mut world = World::new(&SimulationSettings::default(), 30);
            world.add_box(
                Vec3::ZERO,
                Quat::IDENTITY,
                Vec3::new(500.0, 10.0, 500.0),
                &Body::collider(),
            );
            let ball = world.add_ball(Vec3::new(0.0, 200.0, 0.0), Quat::IDENTITY, 25.0, &Body::rigid());
            let mut path = Vec::new();
            for _ in 0..150 {
                world.step_frame();
                path.push(world.pose(ball).unwrap().0);
            }
            path
        };
        let path = run();
        let rest = path.last().unwrap().y;
        assert!(
            (rest - 35.0).abs() < 1.0,
            "rests on the floor's top (10) by its radius (25): {rest}"
        );
        assert!(path.iter().all(|p| p.y > 30.0), "never through the floor");
        assert_eq!(path, run(), "deterministic");
    }

    #[test]
    fn mass_follows_density_or_the_custom_mass() {
        let mut world = World::new(&SimulationSettings::default(), 30);
        // A 100 cm cube at density 1 weighs 1 kg.
        let cube = world.add_box(Vec3::ZERO, Quat::IDENTITY, Vec3::splat(50.0), &Body::rigid());
        assert!((world.mass(cube).unwrap() - 1.0).abs() < 1e-4);
        let heavy = Body {
            mass_mode: grid4d_scene::MassMode::CustomMass,
            mass: 7.0,
            ..Body::rigid()
        };
        let custom = world.add_box(Vec3::X * 500.0, Quat::IDENTITY, Vec3::splat(200.0), &heavy);
        assert!((world.mass(custom).unwrap() - 7.0).abs() < 1e-3, "whatever the size");
    }
}
