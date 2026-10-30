//! An example Grid4D plugin: a toothed wheel, lying in the XZ plane around
//! the Y axis, built from its parameters. It shows what a generator plugin
//! needs: a type id, a name, a parameter description and a `build`.

use std::f32::consts::TAU;

use glam::Vec3;
use grid4d_geom::PolyMesh;
use grid4d_plugin::{GeneratorPlugin, ParamKind, ParamSpec, Unit, Values};
use grid4d_scene::PluginValue;

/// The gear generator.
#[derive(Clone, Copy, Debug, Default)]
pub struct Gear;

const PARAMS: &[ParamSpec] = &[
    ParamSpec {
        name: "Teeth",
        kind: ParamKind::Int { min: 3, max: 200 },
        default: PluginValue::Int(12),
        hint: "How many teeth go round the wheel",
    },
    ParamSpec {
        name: "Radius",
        kind: ParamKind::Float {
            min: 1.0,
            max: 100_000.0,
            unit: Unit::Length,
        },
        default: PluginValue::Float(100.0),
        hint: "From the centre to the tips of the teeth",
    },
    ParamSpec {
        name: "Tooth Depth",
        kind: ParamKind::Float {
            min: 0.0,
            max: 100_000.0,
            unit: Unit::Length,
        },
        default: PluginValue::Float(20.0),
        hint: "How far the teeth stand out",
    },
    ParamSpec {
        name: "Tooth Width",
        kind: ParamKind::Float {
            min: 0.05,
            max: 0.95,
            unit: Unit::Percent,
        },
        default: PluginValue::Float(0.4),
        hint: "The tip of a tooth, as a share of the space from one tooth to the next",
    },
    ParamSpec {
        name: "Thickness",
        kind: ParamKind::Float {
            min: 0.1,
            max: 100_000.0,
            unit: Unit::Length,
        },
        default: PluginValue::Float(20.0),
        hint: "",
    },
    ParamSpec {
        name: "Hole Radius",
        kind: ParamKind::Float {
            min: 0.0,
            max: 100_000.0,
            unit: Unit::Length,
        },
        default: PluginValue::Float(20.0),
        hint: "0: no hole",
    },
];

impl GeneratorPlugin for Gear {
    fn type_id(&self) -> &'static str {
        "grid4d.gear"
    }

    fn name(&self) -> &'static str {
        "Gear"
    }

    fn description(&self) -> &'static str {
        "A toothed wheel"
    }

    fn params(&self) -> &'static [ParamSpec] {
        PARAMS
    }

    fn build(&self, values: &Values<'_>) -> PolyMesh {
        let teeth = u32::try_from(values.int("Teeth")).unwrap_or(3).max(3);
        let tip = values.float("Radius");
        let root = (tip - values.float("Tooth Depth")).max(tip * 0.05);
        let width = values.float("Tooth Width");
        let half = values.float("Thickness") * 0.5;
        let hole = values.float("Hole Radius").min(root * 0.95);
        gear(teeth, root, tip, width, half, hole)
    }
}

/// The wheel's outline (counter-clockwise seen from +Y) as (angle, radius):
/// four points a tooth — up the flank, across the tip, down the flank.
fn outline(teeth: u32, root: f32, tip: f32, width: f32) -> Vec<(f32, f32)> {
    let step = TAU / teeth as f32;
    let tip_half = step * width * 0.5;
    // The base of a tooth is wider than its tip, short of touching the next.
    let base_half = step * ((width + 1.0) * 0.5).min(0.98) * 0.5;
    (0..teeth)
        .flat_map(|i| {
            let centre = i as f32 * step;
            [
                (centre - base_half, root),
                (centre - tip_half, tip),
                (centre + tip_half, tip),
                (centre + base_half, root),
            ]
        })
        .collect()
}

/// A point at `angle` and `radius` in the XZ plane, at height `y`. Angles
/// grow counter-clockwise seen from +Y (from +X towards −Z).
fn at(angle: f32, radius: f32, y: f32) -> Vec3 {
    Vec3::new(radius * angle.cos(), y, -radius * angle.sin())
}

fn gear(teeth: u32, root: f32, tip: f32, width: f32, half: f32, hole: f32) -> PolyMesh {
    let ring = outline(teeth, root, tip, width);
    let n = u32::try_from(ring.len()).unwrap_or(0);
    let next = |k: u32| (k + 1) % n;
    let mut points: Vec<Vec3> = Vec::new();
    // 0..n: top outline, n..2n: bottom outline.
    points.extend(ring.iter().map(|&(a, r)| at(a, r, half)));
    points.extend(ring.iter().map(|&(a, r)| at(a, r, -half)));
    let (top, bottom) = (0, n);
    let mut faces: Vec<Vec<u32>> = Vec::new();
    // The outer wall, facing out.
    for k in 0..n {
        faces.push(vec![bottom + k, bottom + next(k), top + next(k), top + k]);
    }
    if hole > 0.0 {
        // 2n..3n: top hole, 3n..4n: bottom hole, at the outline's angles.
        points.extend(ring.iter().map(|&(a, _)| at(a, hole, half)));
        points.extend(ring.iter().map(|&(a, _)| at(a, hole, -half)));
        let (top_hole, bottom_hole) = (2 * n, 3 * n);
        for k in 0..n {
            // Caps as rings of quads, the top facing up, the bottom down.
            faces.push(vec![top + k, top + next(k), top_hole + next(k), top_hole + k]);
            faces.push(vec![
                bottom + next(k),
                bottom + k,
                bottom_hole + k,
                bottom_hole + next(k),
            ]);
            // The hole's wall, facing in.
            faces.push(vec![
                top_hole + k,
                top_hole + next(k),
                bottom_hole + next(k),
                bottom_hole + k,
            ]);
        }
    } else {
        // Caps as fans around a centre point each.
        let (top_centre, bottom_centre) = (2 * n, 2 * n + 1);
        points.push(Vec3::new(0.0, half, 0.0));
        points.push(Vec3::new(0.0, -half, 0.0));
        for k in 0..n {
            faces.push(vec![top_centre, top + k, top + next(k)]);
            faces.push(vec![bottom_centre, bottom + next(k), bottom + k]);
        }
    }
    PolyMesh::from_faces(points, faces).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use grid4d_plugin::Registry;
    use grid4d_scene::PluginObject;

    use super::*;

    /// A face's normal by Newell's method (not normalised).
    fn normal(mesh: &PolyMesh, face: usize) -> Vec3 {
        let corners: Vec<Vec3> = mesh.face(face).iter().map(|&p| mesh.points()[p as usize]).collect();
        let mut n = Vec3::ZERO;
        for (i, a) in corners.iter().enumerate() {
            let b = corners[(i + 1) % corners.len()];
            n += Vec3::new(
                (a.y - b.y) * (a.z + b.z),
                (a.z - b.z) * (a.x + b.x),
                (a.x - b.x) * (a.y + b.y),
            );
        }
        n
    }

    fn centre(mesh: &PolyMesh, face: usize) -> Vec3 {
        let face = mesh.face(face);
        face.iter().map(|&p| mesh.points()[p as usize]).sum::<Vec3>() / face.len() as f32
    }

    #[test]
    fn a_closed_gear_faces_outwards_with_and_without_a_hole() {
        let mut registry = Registry::default();
        registry.add_generator(Gear);
        let plugin = registry.generator("grid4d.gear").unwrap();
        let mut object = Registry::new_object(plugin);
        for hole in [20.0, 0.0] {
            object.set("Hole Radius", PluginValue::Float(hole));
            let mesh = registry.build(&object).unwrap();
            let n = 12 * 4;
            let expected = if hole > 0.0 { 4 * n } else { 3 * n };
            assert_eq!(mesh.face_count(), expected, "hole {hole}");
            for face in 0..mesh.face_count() {
                let (normal, centre) = (normal(&mesh, face), centre(&mesh, face));
                // Caps face up or down by their side; walls away from the axis,
                // the hole's towards it.
                let outward = if normal.y.abs() > normal.length() * 0.9 {
                    normal.y * centre.y
                } else {
                    let radial = Vec3::new(centre.x, 0.0, centre.z);
                    let into_hole = hole > 0.0 && radial.length() < hole + 1.0;
                    normal.dot(radial) * if into_hole { -1.0 } else { 1.0 }
                };
                assert!(outward > 0.0, "hole {hole}: face {face} faces inwards");
            }
            let (min, max) = mesh.bounds().unwrap();
            assert!((max.x - 100.0).abs() < 1.0 && (max.y - 10.0).abs() < 1e-4 && (min.y + 10.0).abs() < 1e-4);
        }
    }

    #[test]
    fn values_out_of_range_still_make_a_gear() {
        let mut object = PluginObject::new("grid4d.gear");
        object.set("Teeth", PluginValue::Int(-5));
        object.set("Tooth Depth", PluginValue::Float(1.0e6));
        object.set("Hole Radius", PluginValue::Float(1.0e6));
        let mesh = Gear.build(&Values::new(PARAMS, &object));
        assert!(mesh.face_count() > 0);
        assert!(mesh.points().iter().all(|p| p.is_finite()));
    }
}
