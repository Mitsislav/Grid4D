//! Rigid body simulation data (see `docs/reference/simulation-rigid-body.md`):
//! what a Dynamics tag makes of its object, and the document's simulation
//! settings. The simulation itself lives in `grid4d-sim`.

use glam::{Quat, Vec3};

use crate::NodeId;

/// How a body moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Motion {
    /// Moved by the simulation: falls, collides, bounces.
    #[default]
    Dynamic,
    /// Moved only by its own animation (if any); pushes dynamic bodies, never
    /// pushed back.
    Collider,
}

impl Motion {
    pub const ALL: [Self; 2] = [Self::Dynamic, Self::Collider];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Dynamic => "Dynamic",
            Self::Collider => "Collider",
        }
    }
}

/// Whether the object's children belong to its body.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Inherit {
    /// Only the object itself.
    #[default]
    None,
    /// The object and its visible children as one body.
    Compound,
}

impl Inherit {
    pub const ALL: [Self; 2] = [Self::None, Self::Compound];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Compound => "Compound",
        }
    }
}

/// The shape collisions use, made from the object's shape as shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CollisionShape {
    /// A convex hull for a dynamic body, the mesh itself for a collider.
    #[default]
    Automatic,
    /// The bounding box.
    Box,
    /// Around the bounding box's centre, its largest half-size.
    Sphere,
    ConvexHull,
    /// The exact mesh (colliders only; a dynamic body uses a convex hull).
    StaticMesh,
}

impl CollisionShape {
    pub const ALL: [Self; 5] = [
        Self::Automatic,
        Self::Box,
        Self::Sphere,
        Self::ConvexHull,
        Self::StaticMesh,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Automatic => "Automatic",
            Self::Box => "Box",
            Self::Sphere => "Sphere",
            Self::ConvexHull => "Convex Hull",
            Self::StaticMesh => "Static Mesh",
        }
    }
}

/// How a body's mass is found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MassMode {
    /// Density 1: a 100 cm cube weighs 1 kg.
    #[default]
    GlobalDensity,
    /// [`Body::density`] times the volume.
    CustomDensity,
    /// [`Body::mass`], whatever the size.
    CustomMass,
}

impl MassMode {
    pub const ALL: [Self; 3] = [Self::GlobalDensity, Self::CustomDensity, Self::CustomMass];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::GlobalDensity => "Global Density",
            Self::CustomDensity => "Custom Density",
            Self::CustomMass => "Custom Mass",
        }
    }
}

/// What a Dynamics tag makes of its object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub enabled: bool,
    pub motion: Motion,
    pub inherit: Inherit,
    pub shape: CollisionShape,
    /// How much speed survives a hit (1.0 = 100 %).
    pub bounce: f32,
    /// How much sliding is resisted (1.0 = 100 %).
    pub friction: f32,
    pub mass_mode: MassMode,
    /// Custom Density (1 = water: a 100 cm cube weighs 1 kg).
    pub density: f32,
    /// Custom Mass (kg).
    pub mass: f32,
    /// How hard it is to set spinning (1.0 = 100 %).
    pub rotational_mass: f32,
    /// Speed lost per second, moving (1.0 = 100 %).
    pub linear_damping: f32,
    /// Speed lost per second, spinning (1.0 = 100 %).
    pub angular_damping: f32,
    /// At the start frame, world axes (cm/s).
    pub initial_velocity: Vec3,
    /// At the start frame, about the world axes (radians/s).
    pub initial_spin: Vec3,
}

impl Body {
    pub const MAX_ROTATIONAL_MASS: f32 = 100.0;
    pub const MIN_ROTATIONAL_MASS: f32 = 0.01;

    /// A Rigid Body tag's defaults (bounce 50 %, friction 30 % — ours).
    #[must_use]
    pub const fn rigid() -> Self {
        Self {
            enabled: true,
            motion: Motion::Dynamic,
            inherit: Inherit::None,
            shape: CollisionShape::Automatic,
            bounce: 0.5,
            friction: 0.3,
            mass_mode: MassMode::GlobalDensity,
            density: 1.0,
            mass: 1.0,
            rotational_mass: 1.0,
            linear_damping: 0.0,
            angular_damping: 0.0,
            initial_velocity: Vec3::ZERO,
            initial_spin: Vec3::ZERO,
        }
    }

    /// A Collider Body tag's defaults.
    #[must_use]
    pub const fn collider() -> Self {
        Self {
            motion: Motion::Collider,
            ..Self::rigid()
        }
    }

    /// The body with every value in its range (non-finite numbers become the
    /// defaults).
    #[must_use]
    pub fn validated(self) -> Self {
        let defaults = Self::rigid();
        let finite = |v: f32, d: f32| if v.is_finite() { v } else { d };
        let vector = |v: Vec3| if v.is_finite() { v } else { Vec3::ZERO };
        Self {
            bounce: finite(self.bounce, defaults.bounce).max(0.0),
            friction: finite(self.friction, defaults.friction).max(0.0),
            density: finite(self.density, defaults.density).max(0.0),
            mass: finite(self.mass, defaults.mass).max(0.0),
            rotational_mass: finite(self.rotational_mass, 1.0)
                .clamp(Self::MIN_ROTATIONAL_MASS, Self::MAX_ROTATIONAL_MASS),
            linear_damping: finite(self.linear_damping, 0.0).clamp(0.0, 1.0),
            angular_damping: finite(self.angular_damping, 0.0).clamp(0.0, 1.0),
            initial_velocity: vector(self.initial_velocity),
            initial_spin: vector(self.initial_spin),
            ..self
        }
    }
}

/// The document's simulation settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimulationSettings {
    pub enabled: bool,
    /// cm/s².
    pub gravity: Vec3,
    pub steps_per_frame: u32,
    pub solver_iterations: u32,
    /// Below 1: slow motion.
    pub time_scale: f32,
    /// Most memory the simulated frames may take (MB).
    pub memory_limit_mb: u32,
}

impl Default for SimulationSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            gravity: Vec3::new(0.0, -981.0, 0.0),
            steps_per_frame: 5,
            solver_iterations: 4,
            time_scale: 1.0,
            memory_limit_mb: 256,
        }
    }
}

impl SimulationSettings {
    pub const MAX_STEPS: u32 = 100;
    pub const MAX_ITERATIONS: u32 = 100;
    pub const MIN_TIME_SCALE: f32 = 0.01;
    pub const MAX_TIME_SCALE: f32 = 100.0;
    pub const MAX_MEMORY_MB: u32 = 65_536;

    /// The settings with every value in its range (non-finite numbers become
    /// the defaults).
    #[must_use]
    pub fn validated(self) -> Self {
        let defaults = Self::default();
        Self {
            enabled: self.enabled,
            gravity: if self.gravity.is_finite() {
                self.gravity
            } else {
                defaults.gravity
            },
            steps_per_frame: self.steps_per_frame.clamp(1, Self::MAX_STEPS),
            solver_iterations: self.solver_iterations.clamp(1, Self::MAX_ITERATIONS),
            time_scale: if self.time_scale.is_finite() {
                self.time_scale.clamp(Self::MIN_TIME_SCALE, Self::MAX_TIME_SCALE)
            } else {
                defaults.time_scale
            },
            memory_limit_mb: self.memory_limit_mb.clamp(1, Self::MAX_MEMORY_MB),
        }
    }
}

/// A simulation's frames as saved with a document (see
/// `docs/reference/simulation-cache.md`): every body's pose per frame from the
/// start frame on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SimulationCache {
    pub start: i32,
    pub bodies: Vec<CachedBody>,
    /// Frames × bodies: position (cm) and rotation.
    pub poses: Vec<(Vec3, Quat)>,
}

/// A body of a [`SimulationCache`]: an object, or a clone of a Cloner, and its
/// scale (which the simulation keeps).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CachedBody {
    pub node: NodeId,
    /// The clone's number, for a Cloner's clone.
    pub clone: Option<u32>,
    pub scale: Vec3,
}

impl SimulationCache {
    /// Frames held.
    #[must_use]
    pub fn frames(&self) -> usize {
        self.poses.len().checked_div(self.bodies.len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_out_of_range_are_brought_back() {
        let wild = SimulationSettings {
            enabled: false,
            gravity: Vec3::new(f32::NAN, 0.0, 0.0),
            steps_per_frame: 0,
            solver_iterations: 1000,
            time_scale: 0.0,
            memory_limit_mb: 0,
        }
        .validated();
        assert_eq!(wild.gravity, SimulationSettings::default().gravity);
        assert_eq!((wild.steps_per_frame, wild.solver_iterations), (1, 100));
        assert_eq!(wild.time_scale, SimulationSettings::MIN_TIME_SCALE);
        assert!(!wild.enabled);
        let body = Body {
            bounce: -1.0,
            friction: f32::INFINITY,
            rotational_mass: 0.0,
            linear_damping: 2.0,
            initial_velocity: Vec3::NAN,
            ..Body::rigid()
        }
        .validated();
        assert_eq!((body.bounce, body.friction), (0.0, 0.3));
        assert_eq!(body.rotational_mass, Body::MIN_ROTATIONAL_MASS);
        assert_eq!(body.linear_damping, 1.0);
        assert_eq!(body.initial_velocity, Vec3::ZERO);
        assert_eq!(Body::collider().motion, Motion::Collider);
    }
}
