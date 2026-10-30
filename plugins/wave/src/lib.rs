//! An example Grid4D plugin: a deformer that ripples its parent along a sine
//! wave, moving points up and down (its Y axis). It shows what a deformer
//! plugin needs: a type id, a name, a parameter description, the parameter
//! that is its box's size, the one its handle sets, and a `deform`. Its
//! Phase, keyed over time, makes the wave travel.

use std::f32::consts::TAU;

use glam::Vec3;
use grid4d_plugin::{DeformerPlugin, ParamKind, ParamSpec, Plugin, Unit, Values};
use grid4d_scene::PluginValue;

/// The wave deformer.
#[derive(Clone, Copy, Debug, Default)]
pub struct Wave;

const PARAMS: &[ParamSpec] = &[
    ParamSpec {
        name: "Size",
        kind: ParamKind::Vector { unit: Unit::Length },
        default: PluginValue::Vector(Vec3::new(200.0, 200.0, 200.0)),
        hint: "The deformer's box",
    },
    ParamSpec {
        name: "Amplitude",
        kind: ParamKind::Float {
            min: -100_000.0,
            max: 100_000.0,
            unit: Unit::Length,
        },
        default: PluginValue::Float(20.0),
        hint: "How far points move up and down",
    },
    ParamSpec {
        name: "Wavelength",
        kind: ParamKind::Float {
            min: 0.1,
            max: 100_000.0,
            unit: Unit::Length,
        },
        default: PluginValue::Float(100.0),
        hint: "The distance from one crest to the next",
    },
    ParamSpec {
        name: "Phase",
        kind: ParamKind::Float {
            min: -1000.0,
            max: 1000.0,
            unit: Unit::Angle,
        },
        default: PluginValue::Float(0.0),
        hint: "Shifts the wave along; key it to make the wave travel",
    },
    ParamSpec {
        name: "Direction",
        kind: ParamKind::Choice(&["Along X", "Along Z", "Radial"]),
        default: PluginValue::Choice(0),
        hint: "Which way the wave runs",
    },
    ParamSpec {
        name: "Within Box",
        kind: ParamKind::Bool,
        default: PluginValue::Bool(false),
        hint: "Only points inside the box move",
    },
];

impl Plugin for Wave {
    fn type_id(&self) -> &'static str {
        "grid4d.wave"
    }

    fn name(&self) -> &'static str {
        "Wave"
    }

    fn description(&self) -> &'static str {
        "Ripples the object it is under along a wave"
    }

    fn params(&self) -> &'static [ParamSpec] {
        PARAMS
    }
}

impl DeformerPlugin for Wave {
    fn size_param(&self) -> &'static str {
        "Size"
    }

    fn handle_param(&self) -> Option<&'static str> {
        Some("Amplitude")
    }

    fn deform(&self, values: &Values<'_>, points: &mut [Vec3]) {
        let amplitude = values.float("Amplitude");
        let wavelength = values.float("Wavelength");
        let phase = values.float("Phase");
        let direction = values.choice("Direction");
        let half = values.vector("Size").abs() * 0.5;
        let within = values.bool("Within Box");
        for point in points {
            if within && (point.x.abs() > half.x || point.y.abs() > half.y || point.z.abs() > half.z) {
                continue;
            }
            let along = match direction {
                0 => point.x,
                1 => point.z,
                _ => point.x.hypot(point.z),
            };
            point.y += amplitude * (TAU * along / wavelength + phase).sin();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_2;

    use grid4d_plugin::Registry;
    use grid4d_scene::PluginObject;

    use super::*;

    fn deformed(object: &PluginObject, points: &[Vec3]) -> Vec<Vec3> {
        let mut out = points.to_vec();
        Wave.deform(&Values::new(PARAMS, object), &mut out);
        out
    }

    #[test]
    fn points_ride_a_sine_wave() {
        let mut object = Registry::new_object(&Wave);
        // Crest a quarter wavelength along X, trough at three quarters, flat between.
        let points = [Vec3::ZERO, Vec3::new(25.0, 0.0, 0.0), Vec3::new(75.0, 3.0, 9.0)];
        let moved = deformed(&object, &points);
        assert!(moved[0].abs_diff_eq(Vec3::ZERO, 1e-4));
        assert!(moved[1].abs_diff_eq(Vec3::new(25.0, 20.0, 0.0), 1e-3));
        assert!(
            moved[2].abs_diff_eq(Vec3::new(75.0, -17.0, 9.0), 1e-3),
            "{:?}",
            moved[2]
        );

        // A quarter turn of phase moves the crest to the origin.
        object.set("Phase", PluginValue::Float(FRAC_PI_2));
        assert!((deformed(&object, &[Vec3::ZERO])[0].y - 20.0).abs() < 1e-3);

        // Along Z and radially.
        object.set("Phase", PluginValue::Float(0.0));
        object.set("Direction", PluginValue::Choice(1));
        assert!((deformed(&object, &[Vec3::new(0.0, 0.0, 25.0)])[0].y - 20.0).abs() < 1e-3);
        object.set("Direction", PluginValue::Choice(2));
        let ring = deformed(&object, &[Vec3::new(15.0, 0.0, 20.0), Vec3::new(-25.0, 0.0, 0.0)]);
        assert!((ring[0].y - 20.0).abs() < 1e-3 && (ring[1].y - 20.0).abs() < 1e-3);
    }

    #[test]
    fn within_its_box_only_when_asked_and_flat_without_amplitude() {
        let mut object = Registry::new_object(&Wave);
        let outside = Vec3::new(125.0, 0.0, 0.0);
        assert!((deformed(&object, &[outside])[0].y - 20.0).abs() < 1e-3);
        object.set("Within Box", PluginValue::Bool(true));
        assert_eq!(deformed(&object, &[outside])[0], outside);
        object.set("Amplitude", PluginValue::Float(0.0));
        let inside = Vec3::new(25.0, 10.0, 0.0);
        assert_eq!(deformed(&object, &[inside])[0], inside);
    }
}
