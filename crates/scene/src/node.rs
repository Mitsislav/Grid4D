use glam::Vec3;

use crate::{Aabb, Transform};

slotmap::new_key_type! {
    /// Stable, generational handle to a node in a [`Scene`](crate::Scene).
    ///
    /// Handles stay valid across edits and never alias a different node after the
    /// original was removed — store these, never references or raw indices.
    pub struct NodeId;
}

/// Radius (cm) of the viewport marker used to display light objects.
pub const LIGHT_MARKER_RADIUS: f32 = 12.0;
/// Edge length (cm) of the viewport marker used to display camera objects.
pub const CAMERA_MARKER_SIZE: f32 = 40.0;

/// What a node *is*, together with its type-specific parameters.
///
/// Primitive parameters mirror Cinema 4D's "Object" tab (sizes in cm).
#[derive(Clone, Debug, PartialEq)]
pub enum ObjectKind {
    /// Empty transform used to group other objects.
    Null,
    Cube {
        size: Vec3,
    },
    Sphere {
        radius: f32,
    },
    Cylinder {
        radius: f32,
        height: f32,
    },
    Plane {
        width: f32,
        depth: f32,
    },
    Camera {
        fov_y_deg: f32,
    },
    /// `color` is linear RGB; `intensity` is a multiplier (1.0 = 100 %).
    Light {
        color: [f32; 3],
        intensity: f32,
    },
}

/// Field-less discriminant of [`ObjectKind`] — for icons, menus and dispatch tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectKindTag {
    Null,
    Cube,
    Sphere,
    Cylinder,
    Plane,
    Camera,
    Light,
}

impl ObjectKindTag {
    pub const ALL: [Self; 7] = [
        Self::Null,
        Self::Cube,
        Self::Sphere,
        Self::Cylinder,
        Self::Plane,
        Self::Camera,
        Self::Light,
    ];

    /// Geometric primitives (the "Create ▸ Object" family).
    pub const PRIMITIVES: [Self; 4] = [Self::Cube, Self::Sphere, Self::Cylinder, Self::Plane];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Null => "Null",
            Self::Cube => "Cube",
            Self::Sphere => "Sphere",
            Self::Cylinder => "Cylinder",
            Self::Plane => "Plane",
            Self::Camera => "Camera",
            Self::Light => "Light",
        }
    }

    /// Default parameters, matching Cinema 4D's defaults where they exist.
    #[must_use]
    pub fn default_kind(self) -> ObjectKind {
        match self {
            Self::Null => ObjectKind::Null,
            Self::Cube => ObjectKind::Cube {
                size: Vec3::splat(200.0),
            },
            Self::Sphere => ObjectKind::Sphere { radius: 100.0 },
            Self::Cylinder => ObjectKind::Cylinder {
                radius: 50.0,
                height: 200.0,
            },
            Self::Plane => ObjectKind::Plane {
                width: 400.0,
                depth: 400.0,
            },
            Self::Camera => ObjectKind::Camera { fov_y_deg: 40.0 },
            Self::Light => ObjectKind::Light {
                color: [1.0, 1.0, 1.0],
                intensity: 1.0,
            },
        }
    }
}

impl ObjectKind {
    #[must_use]
    pub const fn tag(&self) -> ObjectKindTag {
        match self {
            Self::Null => ObjectKindTag::Null,
            Self::Cube { .. } => ObjectKindTag::Cube,
            Self::Sphere { .. } => ObjectKindTag::Sphere,
            Self::Cylinder { .. } => ObjectKindTag::Cylinder,
            Self::Plane { .. } => ObjectKindTag::Plane,
            Self::Camera { .. } => ObjectKindTag::Camera,
            Self::Light { .. } => ObjectKindTag::Light,
        }
    }

    #[must_use]
    pub const fn label(&self) -> &'static str {
        self.tag().label()
    }

    /// Object-space bounds used for framing and picking.
    /// `None` for objects without spatial extent (a Null).
    #[must_use]
    pub fn local_bounds(&self) -> Option<Aabb> {
        let half = match *self {
            Self::Null => return None,
            Self::Cube { size } => size * 0.5,
            Self::Sphere { radius } => Vec3::splat(radius),
            Self::Cylinder { radius, height } => Vec3::new(radius, height * 0.5, radius),
            // Give the infinitely thin plane 1 cm of thickness so it can be picked edge-on.
            Self::Plane { width, depth } => Vec3::new(width * 0.5, 0.5, depth * 0.5),
            Self::Camera { .. } => Vec3::splat(CAMERA_MARKER_SIZE * 0.5),
            Self::Light { .. } => Vec3::splat(LIGHT_MARKER_RADIUS),
        };
        Some(Aabb::from_half_extents(half.abs()))
    }
}

/// An object in the scene graph.
///
/// Hierarchy links (`parent`, `children`) are owned by the [`Scene`](crate::Scene)
/// and can only be changed through its API, which keeps them consistent.
#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub kind: ObjectKind,
    pub transform: Transform,
    /// Linear RGB colour used by the viewport ("Display Color" in C4D's Basic tab).
    pub display_color: [f32; 3],
    /// Upper traffic-light dot in the Object Manager.
    pub visible_in_editor: bool,
    /// Lower traffic-light dot in the Object Manager.
    pub visible_in_render: bool,
    /// The check mark in the Object Manager.
    pub enabled: bool,
    pub(crate) parent: Option<NodeId>,
    pub(crate) children: Vec<NodeId>,
}

impl Node {
    /// Neutral viewport grey (linear RGB).
    pub const DEFAULT_COLOR: [f32; 3] = [0.55, 0.56, 0.58];

    #[must_use]
    pub fn new(name: impl Into<String>, kind: ObjectKind) -> Self {
        Self {
            name: name.into(),
            kind,
            transform: Transform::IDENTITY,
            display_color: Self::DEFAULT_COLOR,
            visible_in_editor: true,
            visible_in_render: true,
            enabled: true,
            parent: None,
            children: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_transform(mut self, transform: Transform) -> Self {
        self.transform = transform;
        self
    }

    #[must_use]
    pub fn with_position(mut self, position: Vec3) -> Self {
        self.transform.position = position;
        self
    }

    #[must_use]
    pub fn with_color(mut self, linear_rgb: [f32; 3]) -> Self {
        self.display_color = linear_rgb;
        self
    }

    /// World-space bounds given this node's world matrix. Objects without extent
    /// (Nulls) yield a zero-size box at their origin.
    #[must_use]
    pub fn world_bounds(&self, world: &glam::Mat4) -> Aabb {
        match self.kind.local_bounds() {
            Some(local) => local.transformed(world),
            None => {
                let p = world.w_axis.truncate();
                Aabb { min: p, max: p }
            }
        }
    }

    #[must_use]
    pub fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    #[must_use]
    pub fn children(&self) -> &[NodeId] {
        &self.children
    }
}
