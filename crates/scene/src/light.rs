//! Light parameters beyond colour and intensity (see
//! `docs/reference/object-light.md`): the type, and the cone and falloff of
//! Cinema 4D's Details tab.

/// How a light shines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LightType {
    /// In every direction from its position.
    #[default]
    Omni,
    /// From its position along its −Z axis, in a cone.
    Spot,
    /// Along its −Z axis everywhere, like the sun: only its direction matters.
    Infinite,
}

impl LightType {
    pub const ALL: [Self; 3] = [Self::Omni, Self::Spot, Self::Infinite];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Omni => "Omni",
            Self::Spot => "Spot",
            Self::Infinite => "Infinite",
        }
    }

    /// Whether the light has a position (and so a falloff with distance).
    #[must_use]
    pub const fn has_position(self) -> bool {
        matches!(self, Self::Omni | Self::Spot)
    }
}

/// How a light's brightness changes with distance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Falloff {
    /// The same everywhere.
    #[default]
    None,
    /// Full at the light, nothing at the radius.
    Linear,
    /// Exactly the set intensity at the radius, with the square of the distance
    /// nearer and farther.
    InverseSquare,
}

impl Falloff {
    pub const ALL: [Self; 3] = [Self::None, Self::Linear, Self::InverseSquare];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Linear => "Linear",
            Self::InverseSquare => "Inverse Square",
        }
    }
}

/// A light's cone (spots) and falloff.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightDetails {
    /// Off: the whole cone at full brightness (a hard edge).
    pub use_inner: bool,
    /// Full brightness inside this angle (degrees, full opening).
    pub inner_angle_deg: f32,
    /// The cone's full opening angle (degrees); brightness fades from the inner
    /// angle to this one.
    pub outer_angle_deg: f32,
    pub falloff: Falloff,
    /// Radius/Decay, in cm (see [`Falloff`]).
    pub radius: f32,
}

impl LightDetails {
    /// The largest opening angle, in degrees.
    pub const MAX_ANGLE_DEG: f32 = 175.0;
    /// The smallest Radius/Decay, in cm.
    pub const MIN_RADIUS: f32 = 0.01;

    /// Every number, for finiteness checks.
    #[must_use]
    pub const fn numbers(&self) -> [f32; 3] {
        [self.inner_angle_deg, self.outer_angle_deg, self.radius]
    }
}

impl Default for LightDetails {
    fn default() -> Self {
        Self {
            use_inner: true,
            inner_angle_deg: 0.0,
            outer_angle_deg: 30.0,
            falloff: Falloff::None,
            radius: 500.0,
        }
    }
}
