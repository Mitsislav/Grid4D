//! Tags: properties or behaviour added to an object without changing its kind
//! (see `docs/reference/tag-basics.md`). They live in [`NodeData::tags`], so every
//! tag edit is an ordinary data edit (undo, save) of the object.
//!
//! [`NodeData::tags`]: crate::NodeData::tags

use crate::{MaterialId, NodeId};

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
    /// Links a material to the object (and its children without their own).
    Material { material: Option<MaterialId> },
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
            Self::Material => Tag::Material { material: None },
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
            Self::Material { .. } => TagKind::Material,
            Self::Dynamics(_) => TagKind::Dynamics,
        }
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
    /// The material its Material tag links, if any.
    fn material(&self) -> Option<MaterialId>;
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
                Tag::Protection { .. } | Tag::Target { .. } | Tag::Material { .. } | Tag::Dynamics(_) => None,
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
            Tag::Phong { .. } | Tag::Target { .. } | Tag::Material { .. } | Tag::Dynamics(_) => p,
        })
    }

    fn aim(&self) -> Option<Aim> {
        self.iter().find_map(|tag| match *tag {
            Tag::Target { target, up, pitch } => target.map(|target| Aim { target, up, pitch }),
            Tag::Phong { .. } | Tag::Protection { .. } | Tag::Material { .. } | Tag::Dynamics(_) => None,
        })
    }

    fn tag_index(&self, kind: TagKind) -> Option<usize> {
        self.iter().position(|tag| tag.kind() == kind)
    }

    fn material(&self) -> Option<MaterialId> {
        self.iter().find_map(|tag| match *tag {
            Tag::Material { material } => material,
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
        let with = [Tag::Material { material: Some(m) }];
        assert_eq!(with.material(), Some(m));
        assert_eq!(TagKind::Material.default_tag(), Tag::Material { material: None });
    }
}
