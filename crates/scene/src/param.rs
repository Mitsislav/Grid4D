//! An object's own animatable numbers (see `docs/reference/animation-parameters.md`):
//! [`ObjectParam`] names one real-valued parameter of an [`ObjectKind`] by its
//! meaning, so that tracks and files refer to "the radius" rather than to a
//! field of one kind. A parameter an object does not have reads as `None` and
//! writes nothing.

use grid4d_geom::{
    cloner::Layout,
    spline::{ArcType, SplineShape},
    surface::ExtrudeDirection,
};

use crate::{Axis, Falloff, LightType, ObjectKind};

/// One real-valued parameter of an object, named by meaning. Labels match the
/// Attribute Manager's rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectParam {
    /// A cube's size along an axis.
    Size(Axis),
    Radius,
    Height,
    Width,
    TopRadius,
    BottomRadius,
    InnerRadius,
    OuterRadius,
    RingRadius,
    PipeRadius,
    /// A Circle's second radius (an ellipse).
    EllipseRadius,
    /// A Rectangle's or n-Side's rounding.
    RoundingRadius,
    StartRadius,
    EndRadius,
    /// Radians.
    StartAngle,
    /// Radians.
    EndAngle,
    /// Radians.
    Twist,
    /// 0 – 1.
    RadialBias,
    /// 0 – 1.
    HeightBias,
    HorizontalSpacing,
    VerticalSpacing,
    /// Degrees.
    FieldOfView,
    /// A multiplier (1.0 = 100 %).
    Intensity,
    /// Degrees.
    InnerAngle,
    /// Degrees.
    OuterAngle,
    FalloffRadius,
    /// An Extrude's length.
    Offset,
    /// An Extrude's movement vector.
    Movement(Axis),
    /// A Lathe's angle (radians).
    Angle,
    /// A Lathe's rise along Y.
    Rise,
    /// A Lathe's end size (1.0 = 100 %).
    Scaling,
    /// 1.0 = 100 %.
    EndScale,
    /// Radians.
    EndRotation,
    /// 0 – 1.
    StartGrowth,
    /// 0 – 1.
    EndGrowth,
    /// A deformer's strength (radians for a Bend).
    Strength,
    /// A deformer's strength as a fraction (Taper, Bulge; shown "Strength").
    Amount,
    /// A Taper's or Bulge's curvature (a fraction).
    Curvature,
}

impl ObjectParam {
    /// How many there are ([`ObjectParam::index`] is below this).
    pub const COUNT: usize = 42;

    /// Every parameter, in index order.
    pub const ALL: [Self; Self::COUNT] = [
        Self::Size(Axis::X),
        Self::Size(Axis::Y),
        Self::Size(Axis::Z),
        Self::Radius,
        Self::Height,
        Self::Width,
        Self::TopRadius,
        Self::BottomRadius,
        Self::InnerRadius,
        Self::OuterRadius,
        Self::RingRadius,
        Self::PipeRadius,
        Self::EllipseRadius,
        Self::RoundingRadius,
        Self::StartRadius,
        Self::EndRadius,
        Self::StartAngle,
        Self::EndAngle,
        Self::Twist,
        Self::RadialBias,
        Self::HeightBias,
        Self::HorizontalSpacing,
        Self::VerticalSpacing,
        Self::FieldOfView,
        Self::Intensity,
        Self::InnerAngle,
        Self::OuterAngle,
        Self::FalloffRadius,
        Self::Offset,
        Self::Movement(Axis::X),
        Self::Movement(Axis::Y),
        Self::Movement(Axis::Z),
        Self::Angle,
        Self::Rise,
        Self::Scaling,
        Self::EndScale,
        Self::EndRotation,
        Self::StartGrowth,
        Self::EndGrowth,
        Self::Strength,
        Self::Amount,
        Self::Curvature,
    ];

    /// The row label (shared by the Attribute Manager and the F-Curve editor).
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Size(Axis::X) => "Size . X",
            Self::Size(Axis::Y) => "Size . Y",
            Self::Size(Axis::Z) => "Size . Z",
            Self::Radius => "Radius",
            Self::Height => "Height",
            Self::Width => "Width",
            Self::TopRadius => "Top Radius",
            Self::BottomRadius => "Bottom Radius",
            Self::InnerRadius => "Inner Radius",
            Self::OuterRadius => "Outer Radius",
            Self::RingRadius => "Ring Radius",
            Self::PipeRadius => "Pipe Radius",
            Self::EllipseRadius => "Radius Y",
            Self::RoundingRadius => "Rounding Radius",
            Self::StartRadius => "Start Radius",
            Self::EndRadius => "End Radius",
            Self::StartAngle => "Start Angle",
            Self::EndAngle => "End Angle",
            Self::Twist => "Twist",
            Self::RadialBias => "Radial Bias",
            Self::HeightBias => "Height Bias",
            Self::HorizontalSpacing => "Horizontal Spacing",
            Self::VerticalSpacing => "Vertical Spacing",
            Self::FieldOfView => "Field of View (Vertical)",
            Self::Intensity => "Intensity",
            Self::InnerAngle => "Inner Angle",
            Self::OuterAngle => "Outer Angle",
            Self::FalloffRadius => "Radius/Decay",
            Self::Offset => "Offset",
            Self::Movement(Axis::X) => "Movement . X",
            Self::Movement(Axis::Y) => "Movement . Y",
            Self::Movement(Axis::Z) => "Movement . Z",
            Self::Angle => "Angle",
            Self::Rise => "Movement",
            Self::Scaling => "Scaling",
            Self::EndScale => "End Scale",
            Self::EndRotation => "End Rotation",
            Self::StartGrowth => "Start Growth",
            Self::EndGrowth => "End Growth",
            Self::Strength | Self::Amount => "Strength",
            Self::Curvature => "Curvature",
        }
    }

    /// A distinct number below [`ObjectParam::COUNT`] (sets of parameters).
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Size(axis) => axis.index(),
            Self::Radius => 3,
            Self::Height => 4,
            Self::Width => 5,
            Self::TopRadius => 6,
            Self::BottomRadius => 7,
            Self::InnerRadius => 8,
            Self::OuterRadius => 9,
            Self::RingRadius => 10,
            Self::PipeRadius => 11,
            Self::EllipseRadius => 12,
            Self::RoundingRadius => 13,
            Self::StartRadius => 14,
            Self::EndRadius => 15,
            Self::StartAngle => 16,
            Self::EndAngle => 17,
            Self::Twist => 18,
            Self::RadialBias => 19,
            Self::HeightBias => 20,
            Self::HorizontalSpacing => 21,
            Self::VerticalSpacing => 22,
            Self::FieldOfView => 23,
            Self::Intensity => 24,
            Self::InnerAngle => 25,
            Self::OuterAngle => 26,
            Self::FalloffRadius => 27,
            Self::Offset => 28,
            Self::Movement(axis) => 29 + axis.index(),
            Self::Angle => 32,
            Self::Rise => 33,
            Self::Scaling => 34,
            Self::EndScale => 35,
            Self::EndRotation => 36,
            Self::StartGrowth => 37,
            Self::EndGrowth => 38,
            Self::Strength => 39,
            Self::Amount => 40,
            Self::Curvature => 41,
        }
    }

    /// How a value is shown: stored radians in degrees (`Angle`), fractions in
    /// percent (`Percent`), anything else as it is.
    #[must_use]
    pub const fn unit(self) -> ParamUnit {
        match self {
            Self::StartAngle | Self::EndAngle | Self::Twist | Self::Angle | Self::EndRotation | Self::Strength => {
                ParamUnit::Radians
            }
            Self::RadialBias
            | Self::HeightBias
            | Self::Intensity
            | Self::Scaling
            | Self::EndScale
            | Self::StartGrowth
            | Self::EndGrowth
            | Self::Amount
            | Self::Curvature => ParamUnit::Fraction,
            Self::Size(_)
            | Self::Radius
            | Self::Height
            | Self::Width
            | Self::TopRadius
            | Self::BottomRadius
            | Self::InnerRadius
            | Self::OuterRadius
            | Self::RingRadius
            | Self::PipeRadius
            | Self::EllipseRadius
            | Self::RoundingRadius
            | Self::StartRadius
            | Self::EndRadius
            | Self::HorizontalSpacing
            | Self::VerticalSpacing
            | Self::FieldOfView
            | Self::InnerAngle
            | Self::OuterAngle
            | Self::FalloffRadius
            | Self::Offset
            | Self::Movement(_)
            | Self::Rise => ParamUnit::Plain,
        }
    }
}

/// How an [`ObjectParam`]'s stored value is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamUnit {
    /// As stored (cm, degrees stored as degrees).
    Plain,
    /// Stored in radians, shown in degrees.
    Radians,
    /// Stored as a fraction, shown in percent.
    Fraction,
}

impl ParamUnit {
    /// Stored value × this = shown value.
    #[must_use]
    pub const fn display_scale(self) -> f64 {
        match self {
            Self::Plain => 1.0,
            Self::Radians => 180.0 / std::f64::consts::PI,
            Self::Fraction => 100.0,
        }
    }
}

use ObjectParam as P;

impl ObjectKind {
    /// The parameters this kind can animate, in Attribute Manager order
    /// (optional ones only while they are on).
    #[must_use]
    pub fn params(&self) -> Vec<ObjectParam> {
        let all: &[ObjectParam] = match self {
            Self::Cube { .. } => &[P::Size(Axis::X), P::Size(Axis::Y), P::Size(Axis::Z)],
            Self::Sphere { .. } => &[P::Radius],
            Self::Cylinder { .. } | Self::Capsule { .. } => &[P::Radius, P::Height],
            Self::Plane { .. } => &[P::Width, P::Height],
            Self::Cone { .. } => &[P::TopRadius, P::BottomRadius, P::Height],
            Self::Torus { .. } => &[P::RingRadius, P::PipeRadius],
            Self::Disc { .. } => &[P::InnerRadius, P::OuterRadius],
            Self::Camera { .. } => &[P::FieldOfView],
            Self::Light { .. } => &[P::Intensity, P::InnerAngle, P::OuterAngle, P::FalloffRadius],
            Self::SplinePrimitive(primitive) => match primitive.shape {
                SplineShape::Circle { .. } => &[P::Radius, P::EllipseRadius, P::InnerRadius],
                SplineShape::Rectangle { .. } => &[P::Width, P::Height, P::RoundingRadius],
                SplineShape::Arc { .. } => &[P::Radius, P::InnerRadius, P::StartAngle, P::EndAngle],
                SplineShape::NSide { .. } => &[P::Radius, P::RoundingRadius],
                SplineShape::Star { .. } => &[P::InnerRadius, P::OuterRadius, P::Twist],
                SplineShape::Helix { .. } => &[
                    P::StartRadius,
                    P::StartAngle,
                    P::EndRadius,
                    P::EndAngle,
                    P::RadialBias,
                    P::Height,
                    P::HeightBias,
                ],
                SplineShape::Flower { .. } => &[P::InnerRadius, P::OuterRadius],
            },
            Self::Text(_) => &[P::Height, P::HorizontalSpacing, P::VerticalSpacing],
            Self::Extrude { .. } => &[
                P::Offset,
                P::Movement(Axis::X),
                P::Movement(Axis::Y),
                P::Movement(Axis::Z),
            ],
            Self::Lathe(_) => &[P::Angle, P::Rise, P::Scaling],
            Self::Sweep(_) => &[P::EndScale, P::EndRotation, P::StartGrowth, P::EndGrowth],
            Self::Bend(_) => &[
                P::Size(Axis::X),
                P::Size(Axis::Y),
                P::Size(Axis::Z),
                P::Strength,
                P::Angle,
            ],
            Self::Twist(_) => &[P::Size(Axis::X), P::Size(Axis::Y), P::Size(Axis::Z), P::Angle],
            Self::Taper(_) | Self::Bulge(_) => &[
                P::Size(Axis::X),
                P::Size(Axis::Y),
                P::Size(Axis::Z),
                P::Amount,
                P::Curvature,
            ],
            Self::Cloner { cloner, .. } => match cloner.layout {
                Layout::Linear(_) | Layout::Object(_) => &[],
                Layout::Radial(_) => &[P::Radius, P::StartAngle, P::EndAngle],
                Layout::Grid(_) => &[P::Size(Axis::X), P::Size(Axis::Y), P::Size(Axis::Z)],
            },
            Self::Null
            | Self::Polygon { .. }
            | Self::SubdivisionSurface { .. }
            | Self::Loft(_)
            | Self::Spline { .. }
            // The lattice is edited, not keyed.
            | Self::Ffd { .. } => &[],
        };
        all.iter().copied().filter(|&p| self.param(p).is_some()).collect()
    }

    /// The value of `param`, if this object has it (and it is on).
    #[must_use]
    pub fn param(&self, param: ObjectParam) -> Option<f32> {
        // A shared body for reading and writing: `slot` finds the field.
        let mut kind = self.clone();
        kind.slot(param).map(|value| *value)
    }

    /// Sets `param` if this object has it; values out of range are clamped
    /// where the object needs it (sizes and radii stay ≥ 0).
    pub fn set_param(&mut self, param: ObjectParam, value: f32) {
        if !value.is_finite() {
            return;
        }
        let non_negative = (matches!(param.unit(), ParamUnit::Fraction) && param != P::Amount)
            || matches!(
                param,
                P::Size(_)
                    | P::Radius
                    | P::Height
                    | P::Width
                    | P::TopRadius
                    | P::BottomRadius
                    | P::InnerRadius
                    | P::OuterRadius
                    | P::RingRadius
                    | P::PipeRadius
                    | P::EllipseRadius
                    | P::RoundingRadius
                    | P::StartRadius
                    | P::EndRadius
                    | P::FalloffRadius
            );
        let value = if non_negative { value.max(0.0) } else { value };
        let value = match param {
            P::RadialBias | P::HeightBias | P::StartGrowth | P::EndGrowth => value.min(1.0),
            P::FieldOfView => value.clamp(1.0, 170.0),
            _ => value,
        };
        if let Some(slot) = self.slot(param) {
            *slot = value;
        }
    }

    /// The field `param` names in this kind.
    fn slot(&mut self, param: ObjectParam) -> Option<&mut f32> {
        // A light's falloff radius only while there is a falloff (the row is
        // shown then).
        if param == P::FalloffRadius {
            return match self {
                Self::Light {
                    light_type, details, ..
                } if light_type.has_position() && details.falloff != Falloff::None => Some(&mut details.radius),
                _ => None,
            };
        }
        // An Extrude's offset and movement only where its direction uses them.
        if let Self::Extrude { extrusion, .. } = self {
            let direction = extrusion.direction;
            return match param {
                P::Offset if direction != ExtrudeDirection::Absolute => Some(&mut extrusion.offset),
                P::Movement(axis) if matches!(direction, ExtrudeDirection::Custom | ExtrudeDirection::Absolute) => {
                    Some(&mut extrusion.movement[axis.index()])
                }
                _ => None,
            };
        }
        match (self, param) {
            (Self::Cube { size, .. }, P::Size(axis)) => Some(&mut size[axis.index()]),
            (Self::Sphere { radius, .. }, P::Radius)
            | (Self::Cylinder { radius, .. } | Self::Capsule { radius, .. }, P::Radius) => Some(radius),
            (Self::Cylinder { height, .. } | Self::Capsule { height, .. } | Self::Cone { height, .. }, P::Height) => {
                Some(height)
            }
            (Self::Plane { width, .. }, P::Width) => Some(width),
            (Self::Plane { depth, .. }, P::Height) => Some(depth),
            (Self::Cone { top_radius, .. }, P::TopRadius) => Some(top_radius),
            (Self::Cone { bottom_radius, .. }, P::BottomRadius) => Some(bottom_radius),
            (Self::Torus { ring_radius, .. }, P::RingRadius) => Some(ring_radius),
            (Self::Torus { pipe_radius, .. }, P::PipeRadius) => Some(pipe_radius),
            (Self::Disc { inner_radius, .. }, P::InnerRadius) => Some(inner_radius),
            (Self::Disc { outer_radius, .. }, P::OuterRadius) => Some(outer_radius),
            (Self::Camera { fov_y_deg }, P::FieldOfView) => Some(fov_y_deg),
            (Self::Light { intensity, .. }, P::Intensity) => Some(intensity),
            (
                Self::Light {
                    light_type: LightType::Spot,
                    details,
                    ..
                },
                P::InnerAngle,
            ) => Some(&mut details.inner_angle_deg),
            (
                Self::Light {
                    light_type: LightType::Spot,
                    details,
                    ..
                },
                P::OuterAngle,
            ) => Some(&mut details.outer_angle_deg),
            (Self::SplinePrimitive(primitive), param) => shape_slot(&mut primitive.shape, param),
            (Self::Text(text), P::Height) => Some(&mut text.height),
            (Self::Text(text), P::HorizontalSpacing) => Some(&mut text.horizontal_spacing),
            (Self::Text(text), P::VerticalSpacing) => Some(&mut text.vertical_spacing),
            (Self::Lathe(lathe), P::Angle) => Some(&mut lathe.angle),
            (Self::Lathe(lathe), P::Rise) => Some(&mut lathe.movement),
            (Self::Lathe(lathe), P::Scaling) => Some(&mut lathe.scaling),
            (Self::Sweep(sweep), P::EndScale) => Some(&mut sweep.end_scale),
            (Self::Sweep(sweep), P::EndRotation) => Some(&mut sweep.end_rotation),
            (Self::Sweep(sweep), P::StartGrowth) => Some(&mut sweep.start_growth),
            (Self::Sweep(sweep), P::EndGrowth) => Some(&mut sweep.end_growth),
            (Self::Bend(bend), P::Size(axis)) => Some(&mut bend.size[axis.index()]),
            (Self::Bend(bend), P::Strength) => Some(&mut bend.strength),
            (Self::Bend(bend), P::Angle) => Some(&mut bend.angle),
            (Self::Twist(d), P::Size(axis)) => Some(&mut d.size[axis.index()]),
            (Self::Twist(d), P::Angle) => Some(&mut d.angle),
            (Self::Taper(d), P::Size(axis)) => Some(&mut d.size[axis.index()]),
            (Self::Taper(d), P::Amount) => Some(&mut d.strength),
            (Self::Taper(d), P::Curvature) => Some(&mut d.shape.curvature),
            (Self::Bulge(d), P::Size(axis)) => Some(&mut d.size[axis.index()]),
            (Self::Bulge(d), P::Amount) => Some(&mut d.strength),
            (Self::Bulge(d), P::Curvature) => Some(&mut d.shape.curvature),
            (Self::Cloner { cloner, .. }, param) => match (&mut cloner.layout, param) {
                (Layout::Radial(radial), P::Radius) => Some(&mut radial.radius),
                (Layout::Radial(radial), P::StartAngle) => Some(&mut radial.start_angle),
                (Layout::Radial(radial), P::EndAngle) => Some(&mut radial.end_angle),
                (Layout::Grid(grid), P::Size(axis)) => Some(&mut grid.size[axis.index()]),
                _ => None,
            },
            _ => None,
        }
    }
}

/// The field `param` names in a spline primitive's shape (optional values only
/// while they are on).
fn shape_slot(shape: &mut SplineShape, param: ObjectParam) -> Option<&mut f32> {
    match (shape, param) {
        (
            SplineShape::Circle { radius, .. } | SplineShape::Arc { radius, .. } | SplineShape::NSide { radius, .. },
            P::Radius,
        ) => Some(radius),
        (SplineShape::Circle { ellipse, .. }, P::EllipseRadius) => ellipse.as_mut(),
        (SplineShape::Circle { ring, .. }, P::InnerRadius) => ring.as_mut(),
        (SplineShape::Rectangle { width, .. }, P::Width) => Some(width),
        (SplineShape::Rectangle { height, .. } | SplineShape::Helix { height, .. }, P::Height) => Some(height),
        (SplineShape::Rectangle { rounding, .. } | SplineShape::NSide { rounding, .. }, P::RoundingRadius) => {
            rounding.as_mut()
        }
        // An Arc has an inner radius only as a Ring.
        (
            SplineShape::Arc {
                kind: ArcType::Ring,
                inner_radius,
                ..
            }
            | SplineShape::Star { inner_radius, .. }
            | SplineShape::Flower { inner_radius, .. },
            P::InnerRadius,
        ) => Some(inner_radius),
        (SplineShape::Arc { start, .. }, P::StartAngle) => Some(start),
        (SplineShape::Arc { end, .. }, P::EndAngle) => Some(end),
        (SplineShape::Star { outer_radius, .. } | SplineShape::Flower { outer_radius, .. }, P::OuterRadius) => {
            Some(outer_radius)
        }
        (SplineShape::Star { twist, .. }, P::Twist) => Some(twist),
        (SplineShape::Helix { start_radius, .. }, P::StartRadius) => Some(start_radius),
        (SplineShape::Helix { end_radius, .. }, P::EndRadius) => Some(end_radius),
        (SplineShape::Helix { start_angle, .. }, P::StartAngle) => Some(start_angle),
        (SplineShape::Helix { end_angle, .. }, P::EndAngle) => Some(end_angle),
        (SplineShape::Helix { radial_bias, .. }, P::RadialBias) => Some(radial_bias),
        (SplineShape::Helix { height_bias, .. }, P::HeightBias) => Some(height_bias),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ObjectKindTag;

    #[test]
    fn every_listed_parameter_reads_and_writes() {
        for tag in ObjectKindTag::ALL {
            let mut kind = tag.default_kind();
            for param in kind.params() {
                let before = kind.param(param).unwrap();
                let wanted = before * 0.5 + 0.1;
                kind.set_param(param, wanted);
                assert!((kind.param(param).unwrap() - wanted).abs() < 1e-4, "{tag:?} {param:?}");
            }
        }
        // ALL is in index order: every index once, below COUNT.
        for (i, param) in ObjectParam::ALL.into_iter().enumerate() {
            assert_eq!(param.index(), i, "{param:?}");
            assert!(!param.label().is_empty());
        }
    }

    #[test]
    fn parameters_an_object_lacks_do_nothing_and_values_are_clamped() {
        let mut cube = ObjectKindTag::Cube.default_kind();
        assert_eq!(cube.param(P::Radius), None);
        let before = cube.clone();
        cube.set_param(P::Radius, 5.0);
        assert_eq!(cube, before);
        cube.set_param(P::Size(Axis::Y), -10.0);
        assert_eq!(cube.param(P::Size(Axis::Y)), Some(0.0));
        cube.set_param(P::Size(Axis::X), f32::NAN);
        assert_eq!(cube.param(P::Size(Axis::X)), Some(200.0));

        let mut circle = ObjectKindTag::Circle.default_kind();
        assert_eq!(circle.params(), [P::Radius]);
        let ObjectKind::SplinePrimitive(primitive) = &mut circle else {
            unreachable!()
        };
        primitive.shape = SplineShape::Circle {
            radius: 100.0,
            ellipse: Some(50.0),
            ring: None,
        };
        assert_eq!(circle.params(), [P::Radius, P::EllipseRadius]);
        // An Omni light has no cone, and no falloff radius without a falloff.
        let mut light = ObjectKindTag::Light.default_kind();
        assert_eq!(light.params(), [P::Intensity]);
        let ObjectKind::Light { details, .. } = &mut light else {
            unreachable!()
        };
        details.falloff = Falloff::InverseSquare;
        assert_eq!(light.params(), [P::Intensity, P::FalloffRadius]);
    }
}
