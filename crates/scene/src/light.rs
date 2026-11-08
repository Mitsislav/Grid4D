//! Light parameters beyond colour and intensity (see
//! `docs/reference/object-light.md` and `render-lights-area.md`): the type,
//! and the cone, falloff and area shape of Cinema 4D's Details tab.

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
    /// From a shape with a size ([`LightDetails::area_shape`]): soft light and
    /// soft shadows, falling off with distance by itself.
    Area,
}

impl LightType {
    pub const ALL: [Self; 4] = [Self::Omni, Self::Spot, Self::Infinite, Self::Area];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Omni => "Omni",
            Self::Spot => "Spot",
            Self::Infinite => "Infinite",
            Self::Area => "Area",
        }
    }

    /// Whether the light has a position.
    #[must_use]
    pub const fn has_position(self) -> bool {
        matches!(self, Self::Omni | Self::Spot | Self::Area)
    }

    /// Whether its Falloff setting applies (an area light falls off by
    /// itself, an Infinite light not at all).
    #[must_use]
    pub const fn has_falloff(self) -> bool {
        matches!(self, Self::Omni | Self::Spot)
    }
}

/// The shape an area light gives off light from, in its own space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AreaShape {
    /// In the light's XY plane, Size X wide and Size Y high, facing −Z.
    #[default]
    Rectangle,
    /// In the light's XY plane, Size X across, facing −Z.
    Disc,
    /// Centred on the light, Size X across; shines all round.
    Sphere,
    /// Along the light's Z axis, Size X across and Size Y long; shines all round.
    Tube,
}

impl AreaShape {
    pub const ALL: [Self; 4] = [Self::Rectangle, Self::Disc, Self::Sphere, Self::Tube];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Rectangle => "Rectangle",
            Self::Disc => "Disc",
            Self::Sphere => "Sphere",
            Self::Tube => "Tube",
        }
    }

    /// Whether it is flat (and so one- or two-sided).
    #[must_use]
    pub const fn is_flat(self) -> bool {
        matches!(self, Self::Rectangle | Self::Disc)
    }

    /// Whether Size Y means anything for it.
    #[must_use]
    pub const fn uses_height(self) -> bool {
        matches!(self, Self::Rectangle | Self::Tube)
    }
}

/// The shadow a light casts (the reference's General tab).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LightShadow {
    /// No shadow.
    #[default]
    None,
    /// Soft-edged.
    ShadowMaps,
    /// Sharp-edged.
    Raytraced,
    /// Soft, from the light's area (the viewport shows it like Shadow Maps).
    Area,
}

impl LightShadow {
    pub const ALL: [Self; 4] = [Self::None, Self::ShadowMaps, Self::Raytraced, Self::Area];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::ShadowMaps => "Shadow Maps (Soft)",
            Self::Raytraced => "Raytraced (Hard)",
            Self::Area => "Area",
        }
    }

    /// Whether the light casts a shadow at all.
    #[must_use]
    pub const fn casts(self) -> bool {
        !matches!(self, Self::None)
    }

    /// Whether its edge is sharp (else soft) in the viewport.
    #[must_use]
    pub const fn is_hard(self) -> bool {
        matches!(self, Self::Raytraced)
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
    /// An area light's shape.
    pub area_shape: AreaShape,
    /// An area light's Size X and Size Y, in cm (see [`AreaShape`]).
    pub area_size: [f32; 2],
    /// Camera rays and reflections see an area light's shape.
    pub area_visible: bool,
    /// A flat area light shines from both faces (else only along −Z).
    pub two_sided: bool,
}

impl LightDetails {
    /// The largest opening angle, in degrees.
    pub const MAX_ANGLE_DEG: f32 = 175.0;
    /// The smallest Radius/Decay, in cm.
    pub const MIN_RADIUS: f32 = 0.01;
    /// The smallest and largest Size X / Y of an area light, in cm.
    pub const MIN_AREA_SIZE: f32 = 0.1;
    pub const MAX_AREA_SIZE: f32 = 100_000.0;

    /// Every number, for finiteness checks.
    #[must_use]
    pub const fn numbers(&self) -> [f32; 5] {
        [
            self.inner_angle_deg,
            self.outer_angle_deg,
            self.radius,
            self.area_size[0],
            self.area_size[1],
        ]
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
            area_shape: AreaShape::Rectangle,
            area_size: [100.0, 100.0],
            area_visible: false,
            two_sided: false,
        }
    }
}
