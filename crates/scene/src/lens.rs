//! A camera's lens and shutter (see `docs/reference/render-lens.md`): depth
//! of field through a thin lens with a real-sized aperture, and how long the
//! shutter stays open for motion blur.

use crate::NodeId;

/// Where the shutter's open time lies around the frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ShutterPosition {
    /// Centred on the frame.
    #[default]
    Centred,
    /// Starting at the frame.
    Opening,
    /// Ending at the frame.
    Closing,
}

impl ShutterPosition {
    pub const ALL: [Self; 3] = [Self::Centred, Self::Opening, Self::Closing];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Centred => "Centred",
            Self::Opening => "Opening",
            Self::Closing => "Closing",
        }
    }
}

/// A camera's lens and shutter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lens {
    /// Renders through a lens with an aperture; off: a pinhole, everything
    /// sharp.
    pub depth_of_field: bool,
    /// The distance along the camera's axis that is sharp (cm).
    pub focus_distance: f32,
    /// When set, the focus distance is this object's distance along the
    /// axis (a deleted one: [`Lens::focus_distance`]).
    pub focus_object: Option<NodeId>,
    /// The aperture: its diameter is the focal length ÷ this.
    pub f_stop: f32,
    /// The aperture's blades (0: round, else 3 … 16).
    pub blades: u32,
    /// Turns the bladed aperture (radians).
    pub blade_rotation: f32,
    /// How long the shutter stays open, in frames (0: no motion blur).
    pub shutter: f32,
    pub shutter_position: ShutterPosition,
}

impl Lens {
    pub const MIN_FOCUS_DISTANCE: f32 = 1.0;
    pub const MAX_FOCUS_DISTANCE: f32 = 1_000_000.0;
    pub const MIN_F_STOP: f32 = 0.5;
    pub const MAX_F_STOP: f32 = 64.0;
    pub const MAX_BLADES: u32 = 16;
    pub const MAX_SHUTTER: f32 = 2.0;

    /// The focal length (mm) of a lens with vertical field of view `fov_y`
    /// (radians) on a 36 × 24 mm sensor.
    #[must_use]
    pub fn focal_length_mm(fov_y: f32) -> f32 {
        12.0 / (fov_y * 0.5).tan().max(1e-6)
    }

    /// The aperture's radius in the scene (cm) for a vertical field of view
    /// `fov_y` (radians): half the focal length ÷ F-Stop.
    #[must_use]
    pub fn aperture_radius(&self, fov_y: f32) -> f32 {
        let f_stop = self.f_stop.clamp(Self::MIN_F_STOP, Self::MAX_F_STOP);
        Self::focal_length_mm(fov_y) / f_stop * 0.5 / 10.0
    }

    /// The blades as the renderer uses them: 0 (round) or 3 … 16.
    #[must_use]
    pub fn blade_count(&self) -> u32 {
        match self.blades {
            0..=2 => 0,
            n => n.min(Self::MAX_BLADES),
        }
    }

    /// The shutter's open time around `frame`: (start, end), in frames.
    #[must_use]
    pub fn shutter_interval(&self, frame: f64) -> (f64, f64) {
        let open = f64::from(self.shutter.clamp(0.0, Self::MAX_SHUTTER));
        match self.shutter_position {
            ShutterPosition::Centred => (frame - open * 0.5, frame + open * 0.5),
            ShutterPosition::Opening => (frame, frame + open),
            ShutterPosition::Closing => (frame - open, frame),
        }
    }
}

impl Default for Lens {
    /// A pinhole (depth of field off), f/2.8 at 5 m, a 180° shutter.
    fn default() -> Self {
        Self {
            depth_of_field: false,
            focus_distance: 500.0,
            focus_object: None,
            f_stop: 2.8,
            blades: 0,
            blade_rotation: 0.0,
            shutter: 0.5,
            shutter_position: ShutterPosition::Centred,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_aperture_follows_the_focal_length_and_f_stop() {
        // 50 mm on a 36 × 24 sensor: tan(fov / 2) = 12 / 50.
        let fov = 2.0 * (12.0f32 / 50.0).atan();
        assert!((Lens::focal_length_mm(fov) - 50.0).abs() < 1e-3);
        let lens = Lens::default();
        // f/2.8: 17.9 mm across, 0.89 cm in radius.
        assert!((lens.aperture_radius(fov) - 50.0 / 2.8 / 20.0).abs() < 1e-4);
        assert!(lens.aperture_radius(fov) < Lens { f_stop: 1.4, ..lens }.aperture_radius(fov));
        assert_eq!(Lens { blades: 2, ..lens }.blade_count(), 0);
        assert_eq!(Lens { blades: 40, ..lens }.blade_count(), 16);
    }

    #[test]
    fn the_shutter_opens_around_the_frame() {
        let lens = Lens::default();
        assert_eq!(lens.shutter_interval(10.0), (9.75, 10.25));
        let opening = Lens {
            shutter_position: ShutterPosition::Opening,
            ..lens
        };
        assert_eq!(opening.shutter_interval(10.0), (10.0, 10.5));
        let closing = Lens {
            shutter_position: ShutterPosition::Closing,
            shutter: 1.0,
            ..lens
        };
        assert_eq!(closing.shutter_interval(10.0), (9.0, 10.0));
    }
}
