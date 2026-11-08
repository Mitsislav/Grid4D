//! What surrounds the scene (see `docs/reference/render-environment.md`): a
//! daylight [`Sky`] with a sun, or an [`Environment`] picture. Both stand at
//! infinity: only their rotation matters, never their position or scale.

use std::f32::consts::PI;

use crate::TextureId;

/// Where a Sky's sun is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SunPlacement {
    /// The sun shines along the object's −Z axis (turn it like an Infinite
    /// light).
    FromObject,
    /// At [`Sky::elevation`] and [`Sky::azimuth`], whatever the object's turn.
    #[default]
    Angles,
}

impl SunPlacement {
    pub const ALL: [Self; 2] = [Self::Angles, Self::FromObject];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::FromObject => "From Object",
            Self::Angles => "Elevation and Azimuth",
        }
    }
}

/// An open daylight sky with a sun (a sky model from its turbidity and the
/// sun's height).
#[derive(Clone, Debug, PartialEq)]
pub struct Sky {
    pub placement: SunPlacement,
    /// The sun's height above the horizon (radians, −10° … 90°).
    pub elevation: f32,
    /// Its compass direction (radians), from −Z towards +X.
    pub azimuth: f32,
    /// Haze, [`Sky::MIN_TURBIDITY`] … [`Sky::MAX_TURBIDITY`].
    pub turbidity: f32,
    /// Sky and sun together (1.0 = 100 %).
    pub intensity: f32,
    /// The sun as a light and a disc.
    pub sun: bool,
    /// The disc's size relative to the real sun's.
    pub sun_size: f32,
    /// The sun's brightness relative to the model's (1.0 = 100 %).
    pub sun_intensity: f32,
    /// Camera rays that miss everything see it.
    pub visible: bool,
    /// Its light group (empty: none).
    pub group: String,
}

impl Sky {
    pub const MIN_TURBIDITY: f32 = 1.7;
    pub const MAX_TURBIDITY: f32 = 10.0;
    /// The lowest sun (radians): −10°.
    pub const MIN_ELEVATION: f32 = -10.0 * PI / 180.0;
    /// The highest sun (radians): straight up.
    pub const MAX_ELEVATION: f32 = PI / 2.0;
    pub const MIN_SUN_SIZE: f32 = 0.1;
    pub const MAX_SUN_SIZE: f32 = 20.0;
    /// The most any intensity may be (10 000 %).
    pub const MAX_INTENSITY: f32 = 100.0;
}

impl Default for Sky {
    /// A clear day, the sun 45° up in the south-east.
    fn default() -> Self {
        Self {
            placement: SunPlacement::Angles,
            elevation: PI / 4.0,
            azimuth: 135.0 * PI / 180.0,
            turbidity: 3.0,
            intensity: 1.0,
            sun: true,
            sun_size: 1.0,
            sun_intensity: 1.0,
            visible: true,
            group: String::new(),
        }
    }
}

/// A picture all around the scene: an equirectangular (latitude-longitude)
/// texture, its centre column along −Z.
#[derive(Clone, Debug, PartialEq)]
pub struct Environment {
    /// One of the document's textures (`None`, or a dangling id: a uniform
    /// grey of half the intensity).
    pub texture: Option<TextureId>,
    /// 1.0 = 100 %.
    pub intensity: f32,
    /// A turn about the vertical axis (radians), before the object's own.
    pub rotation: f32,
    /// Camera rays that miss everything see it.
    pub visible: bool,
    /// Its light group (empty: none).
    pub group: String,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            texture: None,
            intensity: 1.0,
            rotation: 0.0,
            visible: true,
            group: String::new(),
        }
    }
}
