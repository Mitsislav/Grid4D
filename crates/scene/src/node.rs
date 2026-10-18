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
/// Primitive parameters mirror Cinema 4D's "Object" tab: sizes in cm, the axis
/// along +Y, and segment counts that decide how finely the shape is tessellated
/// (geometry comes from `grid4d_geom::primitives`).
#[derive(Clone, Debug, PartialEq)]
pub enum ObjectKind {
    /// Empty transform used to group other objects.
    Null,
    /// Box; `segments` subdivide X, Y and Z.
    Cube {
        size: Vec3,
        segments: [u32; 3],
    },
    /// UV sphere; `segments` around the axis, half as many from pole to pole.
    Sphere {
        radius: f32,
        segments: u32,
    },
    Cylinder {
        radius: f32,
        height: f32,
        rotation_segments: u32,
        height_segments: u32,
    },
    Plane {
        width: f32,
        depth: f32,
        width_segments: u32,
        depth_segments: u32,
    },
    /// Frustum; a zero radius ends in a point.
    Cone {
        bottom_radius: f32,
        top_radius: f32,
        height: f32,
        rotation_segments: u32,
        height_segments: u32,
    },
    /// `ring_radius` to the centre of the tube, `pipe_radius` of the tube.
    Torus {
        ring_radius: f32,
        pipe_radius: f32,
        ring_segments: u32,
        pipe_segments: u32,
    },
    /// Flat ring on the XZ plane; a zero `inner_radius` makes a solid disc.
    Disc {
        inner_radius: f32,
        outer_radius: f32,
        rotation_segments: u32,
        disc_segments: u32,
    },
    /// Cylinder with hemispherical ends; `height` is the total height.
    Capsule {
        radius: f32,
        height: f32,
        rotation_segments: u32,
        height_segments: u32,
        cap_segments: u32,
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
    Cone,
    Torus,
    Disc,
    Capsule,
    Camera,
    Light,
}

impl ObjectKindTag {
    pub const ALL: [Self; 11] = [
        Self::Null,
        Self::Cube,
        Self::Sphere,
        Self::Cylinder,
        Self::Plane,
        Self::Cone,
        Self::Torus,
        Self::Disc,
        Self::Capsule,
        Self::Camera,
        Self::Light,
    ];

    /// Geometric primitives (the "Create ▸ Object" family).
    pub const PRIMITIVES: [Self; 8] = [
        Self::Cube,
        Self::Sphere,
        Self::Cylinder,
        Self::Plane,
        Self::Cone,
        Self::Torus,
        Self::Disc,
        Self::Capsule,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Null => "Null",
            Self::Cube => "Cube",
            Self::Sphere => "Sphere",
            Self::Cylinder => "Cylinder",
            Self::Plane => "Plane",
            Self::Cone => "Cone",
            Self::Torus => "Torus",
            Self::Disc => "Disc",
            Self::Capsule => "Capsule",
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
                segments: [1, 1, 1],
            },
            Self::Sphere => ObjectKind::Sphere {
                radius: 100.0,
                segments: 24,
            },
            Self::Cylinder => ObjectKind::Cylinder {
                radius: 50.0,
                height: 200.0,
                rotation_segments: 36,
                height_segments: 4,
            },
            Self::Plane => ObjectKind::Plane {
                width: 400.0,
                depth: 400.0,
                width_segments: 20,
                depth_segments: 20,
            },
            Self::Cone => ObjectKind::Cone {
                bottom_radius: 100.0,
                top_radius: 0.0,
                height: 200.0,
                rotation_segments: 36,
                height_segments: 8,
            },
            Self::Torus => ObjectKind::Torus {
                ring_radius: 200.0,
                pipe_radius: 50.0,
                ring_segments: 36,
                pipe_segments: 18,
            },
            Self::Disc => ObjectKind::Disc {
                inner_radius: 0.0,
                outer_radius: 100.0,
                rotation_segments: 36,
                disc_segments: 2,
            },
            Self::Capsule => ObjectKind::Capsule {
                radius: 50.0,
                height: 200.0,
                rotation_segments: 36,
                height_segments: 4,
                cap_segments: 8,
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
            Self::Cone { .. } => ObjectKindTag::Cone,
            Self::Torus { .. } => ObjectKindTag::Torus,
            Self::Disc { .. } => ObjectKindTag::Disc,
            Self::Capsule { .. } => ObjectKindTag::Capsule,
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
            Self::Cube { size, .. } => size * 0.5,
            Self::Sphere { radius, .. } => Vec3::splat(radius),
            Self::Cylinder { radius, height, .. } => Vec3::new(radius, height * 0.5, radius),
            // Give infinitely thin shapes 1 cm of thickness so they can be picked edge-on.
            Self::Plane { width, depth, .. } => Vec3::new(width * 0.5, 0.5, depth * 0.5),
            Self::Cone {
                bottom_radius,
                top_radius,
                height,
                ..
            } => {
                let r = bottom_radius.abs().max(top_radius.abs());
                Vec3::new(r, height * 0.5, r)
            }
            Self::Torus {
                ring_radius,
                pipe_radius,
                ..
            } => {
                let r = ring_radius.abs() + pipe_radius.abs();
                Vec3::new(r, pipe_radius, r)
            }
            Self::Disc { outer_radius, .. } => Vec3::new(outer_radius, 0.5, outer_radius),
            // The ends are hemispheres, so a capsule is never shorter than its diameter.
            Self::Capsule { radius, height, .. } => Vec3::new(radius, (height * 0.5).max(radius.abs()), radius),
            Self::Camera { .. } => Vec3::splat(CAMERA_MARKER_SIZE * 0.5),
            Self::Light { .. } => Vec3::splat(LIGHT_MARKER_RADIUS),
        };
        Some(Aabb::from_half_extents(half.abs()))
    }
}

/// Everything about a node except its place in the hierarchy: the payload that
/// the Attribute Manager and the Object Manager edit.
///
/// Kept separate from the hierarchy links so that a value edit is one cheap
/// snapshot/swap of `NodeData` (the unit of undo for attribute edits) and can
/// never corrupt the parent/child structure.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeData {
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
}

impl NodeData {
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
        }
    }

    /// World-space bounds given the node's world matrix. Objects without extent
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
}

/// An object in the scene graph: its [`NodeData`] plus hierarchy links.
///
/// Hierarchy links (`parent`, `children`) and the detached flag are owned by the
/// [`Scene`](crate::Scene) and can only be changed through its API, which keeps
/// them consistent.
#[derive(Clone, Debug)]
pub struct Node {
    pub data: NodeData,
    pub(crate) parent: Option<NodeId>,
    pub(crate) children: Vec<NodeId>,
    /// Part of a subtree removed with [`Scene::detach`](crate::Scene::detach):
    /// still stored (so its `NodeId`s survive an undo) but invisible to every
    /// public query.
    pub(crate) detached: bool,
    /// Changes whenever the node may have been modified (see [`Node::revision`]).
    pub(crate) revision: u64,
}

impl Node {
    #[must_use]
    pub fn new(name: impl Into<String>, kind: ObjectKind) -> Self {
        Self::from_data(NodeData::new(name, kind))
    }

    #[must_use]
    pub fn from_data(data: NodeData) -> Self {
        Self {
            data,
            parent: None,
            children: Vec::new(),
            detached: false,
            revision: crate::scene::next_revision(),
        }
    }

    #[must_use]
    pub fn with_transform(mut self, transform: Transform) -> Self {
        self.data.transform = transform;
        self
    }

    #[must_use]
    pub fn with_position(mut self, position: Vec3) -> Self {
        self.data.transform.position = position;
        self
    }

    #[must_use]
    pub fn with_color(mut self, linear_rgb: [f32; 3]) -> Self {
        self.data.display_color = linear_rgb;
        self
    }

    /// See [`NodeData::world_bounds`].
    #[must_use]
    pub fn world_bounds(&self, world: &glam::Mat4) -> Aabb {
        self.data.world_bounds(world)
    }

    #[must_use]
    pub fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    #[must_use]
    pub fn children(&self) -> &[NodeId] {
        &self.children
    }

    /// Changes whenever this node may have been modified: mutable access, insertion,
    /// re-attachment or a move. Values are unique across all scenes, so caches keyed
    /// by revision can never confuse two nodes or two documents.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn half(kind: &ObjectKind) -> Vec3 {
        kind.local_bounds().map_or(Vec3::ZERO, |b| b.max)
    }

    #[test]
    fn every_tag_has_a_matching_default() {
        for tag in ObjectKindTag::ALL {
            assert_eq!(tag.default_kind().tag(), tag);
        }
        assert!(ObjectKindTag::PRIMITIVES.iter().all(|t| ObjectKindTag::ALL.contains(t)));
    }

    #[test]
    fn bounds_of_the_round_primitives() {
        assert_eq!(
            half(&ObjectKindTag::Torus.default_kind()),
            Vec3::new(250.0, 50.0, 250.0)
        );
        assert_eq!(
            half(&ObjectKindTag::Cone.default_kind()),
            Vec3::new(100.0, 100.0, 100.0)
        );
        assert_eq!(half(&ObjectKindTag::Disc.default_kind()), Vec3::new(100.0, 0.5, 100.0));
        let squat = ObjectKind::Capsule {
            radius: 50.0,
            height: 20.0,
            rotation_segments: 8,
            height_segments: 1,
            cap_segments: 4,
        };
        assert_eq!(half(&squat), Vec3::splat(50.0), "never shorter than its diameter");
    }
}
