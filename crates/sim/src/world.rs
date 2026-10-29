//! A physics world: bodies with simple shapes, stepped frame by frame. The
//! simulation of a document (11.2) builds one from the evaluated scene.

use glam::{Quat, Vec3};
use grid4d_scene::{Body, MassMode, Motion, SimulationSettings};
use rapier3d::prelude::{ColliderBuilder, PhysicsWorld, RigidBodyBuilder, RigidBodyHandle};

use crate::convert::{from_engine, length_to_engine, rotation_from_engine, rotation_to_engine, to_engine};

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

    /// Adds a body shaped like a ball of `radius` (cm), centred at `position`.
    pub fn add_ball(&mut self, position: Vec3, rotation: Quat, radius: f32, body: &Body) -> BodyId {
        self.add(
            position,
            rotation,
            ColliderBuilder::ball(length_to_engine(radius)),
            body,
        )
    }

    /// Adds a body shaped like a box of half-sizes `half` (cm), centred at `position`.
    pub fn add_box(&mut self, position: Vec3, rotation: Quat, half: Vec3, body: &Body) -> BodyId {
        let [x, y, z] = half.abs().to_array().map(length_to_engine);
        self.add(position, rotation, ColliderBuilder::cuboid(x, y, z), body)
    }

    fn add(&mut self, position: Vec3, rotation: Quat, shape: ColliderBuilder, body: &Body) -> BodyId {
        let body = body.validated();
        let builder = match body.motion {
            Motion::Dynamic => RigidBodyBuilder::dynamic()
                .linvel(to_engine(body.initial_velocity))
                .angvel(rapier3d::prelude::Vector::from_array(body.initial_spin.to_array()))
                .linear_damping(body.linear_damping)
                .angular_damping(body.angular_damping),
            // Moved only by its animation (11.2): kinematic.
            Motion::Collider => RigidBodyBuilder::kinematic_position_based(),
        }
        .translation(to_engine(position))
        .rotation(rotation_to_engine(rotation).to_scaled_axis());
        let mut collider = shape.restitution(body.bounce).friction(body.friction);
        collider = match body.mass_mode {
            // Density 1: a 100 cm cube (1 m³) weighs 1 kg.
            MassMode::GlobalDensity => collider.density(1.0),
            MassMode::CustomDensity => collider.density(body.density),
            MassMode::CustomMass => collider.mass(body.mass),
        };
        let (handle, _) = self.inner.insert(builder, collider);
        BodyId(handle)
    }

    /// Advances one frame (its sub-steps).
    pub fn step_frame(&mut self) {
        for _ in 0..self.steps_per_frame {
            self.inner.step();
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
}
