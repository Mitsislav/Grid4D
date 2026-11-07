//! Tags: properties or behaviour added to an object without changing its kind
//! (see `docs/reference/tag-basics.md`). They live in [`NodeData::tags`], so every
//! tag edit is an ordinary data edit (undo, save) of the object.
//!
//! [`NodeData::tags`]: crate::NodeData::tags

use glam::{Affine2, Mat4, Vec2, Vec3};
pub use grid4d_geom::uv::Projection;

use crate::{MaterialId, NodeId, Transform};

/// Grid4D's smoothing for objects without a Phong tag: 80° (radians).
pub const DEFAULT_PHONG_ANGLE: f32 = 80.0 * std::f32::consts::PI / 180.0;

/// A tag on an object.
#[derive(Clone, Debug, PartialEq)]
pub enum Tag {
    /// How the object's mesh is smoothed. With `angle_limit` off everything
    /// connected is smooth; otherwise edges sharper than `angle` (radians) stay hard.
    Phong { angle_limit: bool, angle: f32 },
    /// Channels of the transform the viewport and the Coordinates fields may not change.
    Protection {
        position: bool,
        scale: bool,
        rotation: bool,
    },
    /// Turns the object's forward axis (−Z) towards `target`, its up axis towards
    /// `up` (world up when `None`); with `pitch` off it only turns around Y.
    Target {
        target: Option<NodeId>,
        up: Option<NodeId>,
        pitch: bool,
    },
    /// Links a material to the object (and its children without their own)
    /// and says how it lies on the surface. An object may carry several:
    /// later ones lie over earlier ones.
    Material(MaterialTag),
    /// Makes the object a rigid or collider body in the simulation.
    Dynamics(crate::Body),
}

/// The kinds of tag, for menus and icons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TagKind {
    Phong,
    Protection,
    Target,
    Material,
    Dynamics,
}

impl TagKind {
    pub const ALL: [Self; 5] = [
        Self::Phong,
        Self::Protection,
        Self::Target,
        Self::Material,
        Self::Dynamics,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Phong => "Phong",
            Self::Protection => "Protection",
            Self::Target => "Target",
            Self::Material => "Material",
            Self::Dynamics => "Rigid Body",
        }
    }

    /// A new tag of this kind with its default settings.
    #[must_use]
    pub const fn default_tag(self) -> Tag {
        match self {
            Self::Phong => Tag::Phong {
                angle_limit: true,
                angle: DEFAULT_PHONG_ANGLE,
            },
            Self::Protection => Tag::Protection {
                position: true,
                scale: true,
                rotation: true,
            },
            Self::Target => Tag::Target {
                target: None,
                up: None,
                pitch: true,
            },
            Self::Material => Tag::Material(MaterialTag::new(None)),
            Self::Dynamics => Tag::Dynamics(crate::Body::rigid()),
        }
    }
}

impl Tag {
    #[must_use]
    pub const fn kind(&self) -> TagKind {
        match self {
            Self::Phong { .. } => TagKind::Phong,
            Self::Protection { .. } => TagKind::Protection,
            Self::Target { .. } => TagKind::Target,
            Self::Material(_) => TagKind::Material,
            Self::Dynamics(_) => TagKind::Dynamics,
        }
    }
}

/// Where a projection's shape sits in the object's space (see
/// `docs/reference/tag-material.md`): its centre, its turn (heading, pitch,
/// bank in radians) and its full size along its own axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub position: Vec3,
    pub rotation: Vec3,
    pub size: Vec3,
}

impl Placement {
    /// The placement that fits the box `min` – `max` (unturned).
    #[must_use]
    pub fn fitted(min: Vec3, max: Vec3) -> Self {
        Self {
            position: (min + max) * 0.5,
            rotation: Vec3::ZERO,
            size: max - min,
        }
    }

    /// From the object's space into the shape's own (the cube −1 … 1).
    #[must_use]
    pub fn to_unit(&self) -> Mat4 {
        let transform = Transform {
            position: self.position,
            rotation: self.rotation,
            scale: (self.size * 0.5).max(Vec3::splat(1e-4)),
        };
        transform.matrix().inverse()
    }
}

/// A Material tag: the material and how it is projected onto the surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialTag {
    pub material: Option<MaterialId>,
    pub projection: Projection,
    /// Added to the coordinates after tiling, in picture widths (U, V).
    pub offset: [f32; 2],
    /// Repeats across the coordinates (U, V); never 0.
    pub tiling: [f32; 2],
    /// Turns the coordinates about the picture's centre (radians).
    pub rotation: f32,
    /// Where the projection's shape sits; `None`: fitted to the bounding box
    /// of each object the tag reaches (follows it as it changes). Unused by UV
    /// and Camera.
    pub placement: Option<Placement>,
    /// Camera projection: the camera object (`None`: the view's camera).
    pub camera: Option<NodeId>,
}

impl MaterialTag {
    /// `material` on the object's own UVs, once.
    #[must_use]
    pub const fn new(material: Option<MaterialId>) -> Self {
        Self {
            material,
            projection: Projection::Uv,
            offset: [0.0; 2],
            tiling: [1.0; 2],
            rotation: 0.0,
            placement: None,
            camera: None,
        }
    }

    /// The change the tag makes to projected coordinates: turned about the
    /// picture's centre, repeated `tiling` times, then shifted by `offset`.
    #[must_use]
    pub fn uv_transform(&self) -> Affine2 {
        Affine2::from_translation(Vec2::from(self.offset))
            * Affine2::from_scale(Vec2::from(self.tiling))
            * Affine2::from_translation(Vec2::splat(0.5))
            * Affine2::from_angle(-self.rotation)
            * Affine2::from_translation(Vec2::splat(-0.5))
    }

    /// From the space of an object it reaches into the projection's shape's
    /// own, for an object with object-space `bounds` (min, max); `None` for UV
    /// and Camera, or a fitted shape on an object without bounds.
    #[must_use]
    pub fn to_unit(&self, bounds: Option<(Vec3, Vec3)>) -> Option<Mat4> {
        if !self.projection.has_shape() {
            return None;
        }
        match self.placement {
            Some(placement) => Some(placement.to_unit()),
            None => bounds.map(|(min, max)| grid4d_geom::uv::fit(min, max)),
        }
    }

    /// Every number, for finiteness checks.
    #[must_use]
    pub fn numbers(&self) -> Vec<f32> {
        let mut out = vec![
            self.offset[0],
            self.offset[1],
            self.tiling[0],
            self.tiling[1],
            self.rotation,
        ];
        if let Some(p) = &self.placement {
            out.extend(p.position.to_array());
            out.extend(p.rotation.to_array());
            out.extend(p.size.to_array());
        }
        out
    }

    /// The tag with a zero tiling made 1 and a placement's size kept positive.
    #[must_use]
    pub fn validated(mut self) -> Self {
        for t in &mut self.tiling {
            if t.abs() < 1e-6 {
                *t = 1.0;
            }
        }
        if let Some(p) = &mut self.placement {
            p.size = p.size.abs();
        }
        self
    }
}

/// Which transform channels are protected (all `false` without a Protection tag).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Protection {
    pub position: bool,
    pub scale: bool,
    pub rotation: bool,
}

/// What a Target tag asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Aim {
    pub target: NodeId,
    pub up: Option<NodeId>,
    pub pitch: bool,
}

/// Reading an object's tags.
pub trait Tags {
    /// The object's smoothing angle (radians): its Phong tag's, 180° with the
    /// angle limit off, or [`DEFAULT_PHONG_ANGLE`] without a tag.
    fn smoothing_angle(&self) -> f32;
    fn protection(&self) -> Protection;
    /// Its Target tag, if that names a target.
    fn aim(&self) -> Option<Aim>;
    /// The index of its tag of `kind`, if it has one.
    fn tag_index(&self, kind: TagKind) -> Option<usize>;
    /// The material it shows: its last Material tag's that links one.
    fn material(&self) -> Option<MaterialId>;
    /// The material of its last Material tag whose link passes `exists`.
    fn material_in(&self, exists: impl Fn(MaterialId) -> bool) -> Option<MaterialId>;
    /// Its Material tags, in order (each later one over the earlier ones).
    fn material_tags(&self) -> impl Iterator<Item = &MaterialTag>;
}

impl Tags for [Tag] {
    fn smoothing_angle(&self) -> f32 {
        self.iter()
            .find_map(|tag| match *tag {
                Tag::Phong { angle_limit, angle } => Some(if angle_limit {
                    angle.clamp(0.0, std::f32::consts::PI)
                } else {
                    std::f32::consts::PI
                }),
                Tag::Protection { .. } | Tag::Target { .. } | Tag::Material(_) | Tag::Dynamics(_) => None,
            })
            .unwrap_or(DEFAULT_PHONG_ANGLE)
    }

    fn protection(&self) -> Protection {
        self.iter().fold(Protection::default(), |p, tag| match *tag {
            Tag::Protection {
                position,
                scale,
                rotation,
            } => Protection {
                position: p.position || position,
                scale: p.scale || scale,
                rotation: p.rotation || rotation,
            },
            Tag::Phong { .. } | Tag::Target { .. } | Tag::Material(_) | Tag::Dynamics(_) => p,
        })
    }

    fn aim(&self) -> Option<Aim> {
        self.iter().find_map(|tag| match *tag {
            Tag::Target { target, up, pitch } => target.map(|target| Aim { target, up, pitch }),
            Tag::Phong { .. } | Tag::Protection { .. } | Tag::Material(_) | Tag::Dynamics(_) => None,
        })
    }

    fn tag_index(&self, kind: TagKind) -> Option<usize> {
        self.iter().position(|tag| tag.kind() == kind)
    }

    fn material(&self) -> Option<MaterialId> {
        self.material_tags().filter_map(|tag| tag.material).last()
    }

    fn material_in(&self, exists: impl Fn(MaterialId) -> bool) -> Option<MaterialId> {
        self.iter().rev().find_map(|tag| match tag {
            Tag::Material(tag) => tag.material.filter(|&m| exists(m)),
            Tag::Phong { .. } | Tag::Protection { .. } | Tag::Target { .. } | Tag::Dynamics(_) => None,
        })
    }

    fn material_tags(&self) -> impl Iterator<Item = &MaterialTag> {
        self.iter().filter_map(|tag| match tag {
            Tag::Material(tag) => Some(tag),
            Tag::Phong { .. } | Tag::Protection { .. } | Tag::Target { .. } | Tag::Dynamics(_) => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_readers() {
        let none: &[Tag] = &[];
        assert_eq!(none.smoothing_angle(), DEFAULT_PHONG_ANGLE);
        assert_eq!(none.protection(), Protection::default());
        assert_eq!(none.aim(), None);

        let tags = [
            Tag::Phong {
                angle_limit: false,
                angle: 0.3,
            },
            TagKind::Protection.default_tag(),
            TagKind::Target.default_tag(),
        ];
        assert_eq!(tags.smoothing_angle(), std::f32::consts::PI, "no limit = 180°");
        assert!(tags.protection().position && tags.protection().rotation);
        assert_eq!(tags.aim(), None, "a Target tag without a target does nothing");
        assert_eq!(tags.tag_index(TagKind::Target), Some(2));
        assert_eq!(TagKind::ALL.map(|k| k.default_tag().kind()), TagKind::ALL);
        assert_eq!(tags.material(), None);
        let mut materials = crate::Materials::default();
        let m = materials.insert(crate::Material::new("Mat"));
        let n = materials.insert(crate::Material::new("Label"));
        let with = [Tag::Material(MaterialTag::new(Some(m)))];
        assert_eq!(with.material(), Some(m));
        assert_eq!(TagKind::Material.default_tag(), Tag::Material(MaterialTag::new(None)));
        // Several Material tags: the last that links one is what the object shows.
        let layered = [
            Tag::Material(MaterialTag::new(Some(m))),
            Tag::Material(MaterialTag::new(Some(n))),
            Tag::Material(MaterialTag::new(None)),
        ];
        assert_eq!(layered.material(), Some(n));
        assert_eq!(layered.material_tags().count(), 3);
        assert_eq!(
            layered.material_in(|id| id != n),
            Some(m),
            "a missing material is passed over"
        );
    }

    #[test]
    fn a_material_tag_maps_coordinates_and_places_its_shape() {
        let mut tag = MaterialTag::new(None);
        assert_eq!(tag.uv_transform(), Affine2::IDENTITY);
        assert_eq!(tag.to_unit(Some((Vec3::ZERO, Vec3::ONE))), None, "UV has no shape");
        tag.tiling = [2.0, 3.0];
        tag.offset = [0.25, 0.0];
        let t = tag.uv_transform();
        assert!(
            t.transform_point2(Vec2::new(1.0, 1.0))
                .abs_diff_eq(Vec2::new(2.25, 3.0), 1e-6)
        );
        tag.tiling = [1.0; 2];
        tag.offset = [0.0; 2];
        tag.rotation = std::f32::consts::FRAC_PI_2;
        // A quarter turn keeps the centre and turns the picture.
        let t = tag.uv_transform();
        assert!(t.transform_point2(Vec2::splat(0.5)).abs_diff_eq(Vec2::splat(0.5), 1e-6));
        assert!(
            t.transform_point2(Vec2::new(1.0, 0.5))
                .abs_diff_eq(Vec2::new(0.5, 0.0), 1e-6)
        );

        tag.projection = Projection::Box;
        let fitted = tag
            .to_unit(Some((Vec3::new(-2.0, 0.0, 0.0), Vec3::new(2.0, 2.0, 4.0))))
            .unwrap();
        assert!(
            fitted
                .transform_point3(Vec3::new(2.0, 2.0, 4.0))
                .abs_diff_eq(Vec3::ONE, 1e-6)
        );
        assert_eq!(tag.to_unit(None), None, "fitted to nothing");
        tag.placement = Some(Placement {
            position: Vec3::new(0.0, 10.0, 0.0),
            rotation: Vec3::ZERO,
            size: Vec3::splat(4.0),
        });
        let placed = tag.to_unit(None).unwrap();
        assert!(
            placed
                .transform_point3(Vec3::new(2.0, 12.0, -2.0))
                .abs_diff_eq(Vec3::new(1.0, 1.0, -1.0), 1e-6)
        );
        let box_ = Placement::fitted(Vec3::ZERO, Vec3::new(2.0, 4.0, 6.0));
        assert_eq!(box_.position, Vec3::new(1.0, 2.0, 3.0));
        tag.tiling = [0.0, 2.0];
        assert_eq!(tag.validated().tiling, [1.0, 2.0]);
        assert_eq!(tag.numbers().len(), 14);
    }
}
