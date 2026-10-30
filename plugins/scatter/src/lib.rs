//! An example Grid4D plugin: a command that spreads copies of the selected
//! objects at random over the surface of the active one. It shows what a
//! command plugin needs: a type id, a name, its options (a parameter
//! description) and a `run` that reads the document and returns the changes
//! — new objects under a new Null — which Grid4D applies as one undo step.

use glam::{Mat4, Quat, Vec3};
use grid4d_plugin::{Changes, CommandPlugin, Context, ParamKind, ParamSpec, Parent, Plugin, Unit, Values};
use grid4d_scene::{NodeData, NodeId, ObjectKind, Param, PluginValue, Transform};

/// The scatter command.
#[derive(Clone, Copy, Debug, Default)]
pub struct Scatter;

const PARAMS: &[ParamSpec] = &[
    ParamSpec {
        name: "Count",
        kind: ParamKind::Int { min: 1, max: 10_000 },
        default: PluginValue::Int(20),
        hint: "How many copies to place",
    },
    ParamSpec {
        name: "Seed",
        kind: ParamKind::Int { min: 0, max: 1_000_000 },
        default: PluginValue::Int(1),
        hint: "The same seed always gives the same result",
    },
    ParamSpec {
        name: "Align to Surface",
        kind: ParamKind::Bool,
        default: PluginValue::Bool(true),
        hint: "Copies stand up along the surface",
    },
    ParamSpec {
        name: "Random Rotation",
        kind: ParamKind::Float {
            min: 0.0,
            max: std::f32::consts::PI,
            unit: Unit::Angle,
        },
        default: PluginValue::Float(std::f32::consts::PI),
        hint: "Turns each copy about its up axis by up to this much either way",
    },
    ParamSpec {
        name: "Scale Variation",
        kind: ParamKind::Float {
            min: 0.0,
            max: 0.95,
            unit: Unit::Percent,
        },
        default: PluginValue::Float(0.0),
        hint: "Makes each copy bigger or smaller by up to this much",
    },
];

impl Plugin for Scatter {
    fn type_id(&self) -> &'static str {
        "grid4d.scatter"
    }

    fn name(&self) -> &'static str {
        "Scatter"
    }

    fn description(&self) -> &'static str {
        "Spreads copies of the selected objects over the surface of the one selected last"
    }

    fn params(&self) -> &'static [ParamSpec] {
        PARAMS
    }
}

/// A small, fast random number generator (`SplitMix64`): the same seed
/// gives the same numbers on every machine.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in 0 … 1 (1 excluded).
    fn unit(&mut self) -> f32 {
        // The top 24 bits: exactly representable.
        let bits = u32::try_from(self.next() >> 40).unwrap_or(0);
        #[expect(clippy::cast_precision_loss, reason = "24 bits fit an f32 exactly")]
        let value = bits as f32 / 16_777_216.0;
        value
    }

    /// A number in −1 … 1.
    fn signed(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }

    /// An index below `count` (> 0).
    fn below(&mut self, count: usize) -> usize {
        let count = u64::try_from(count).unwrap_or(u64::MAX).max(1);
        usize::try_from(self.next() % count).unwrap_or(0)
    }
}

/// The surface's triangles in the world, with the running total of their
/// areas (to pick one in proportion to its area).
struct Surface {
    triangles: Vec<[Vec3; 3]>,
    totals: Vec<f32>,
}

impl Surface {
    fn new(mesh: &grid4d_geom::PolyMesh, world: Mat4) -> Self {
        let mut triangles = Vec::new();
        let mut totals = Vec::new();
        let mut total = 0.0;
        for face in mesh.faces() {
            let corner = |i: usize| {
                let index = face.get(i).and_then(|&p| usize::try_from(p).ok()).unwrap_or(0);
                world.transform_point3(mesh.points().get(index).copied().unwrap_or(Vec3::ZERO))
            };
            // A fan from the first corner.
            for i in 1..face.len().saturating_sub(1) {
                let triangle = [corner(0), corner(i), corner(i + 1)];
                let area = (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]).length() * 0.5;
                if area > 1e-9 && area.is_finite() {
                    total += area;
                    triangles.push(triangle);
                    totals.push(total);
                }
            }
        }
        Self { triangles, totals }
    }

    /// A point evenly at random over the area, and the surface's normal there.
    fn sample(&self, random: &mut Random) -> (Vec3, Vec3) {
        let total = self.totals.last().copied().unwrap_or(0.0);
        let wanted = random.unit() * total;
        let index = self
            .totals
            .partition_point(|&t| t <= wanted)
            .min(self.triangles.len() - 1);
        let [a, b, c] = self.triangles[index];
        let (mut u, mut v) = (random.unit(), random.unit());
        if u + v > 1.0 {
            (u, v) = (1.0 - u, 1.0 - v);
        }
        let point = a + (b - a) * u + (c - a) * v;
        (point, (b - a).cross(c - a).normalize_or(Vec3::Y))
    }
}

impl CommandPlugin for Scatter {
    fn run(&self, context: &Context<'_>, options: &Values<'_>) -> Result<Changes, String> {
        let scene = &context.document.scene;
        // Messages are written in `Err(…)`, where translation finds them.
        let Some(surface) = context.active.filter(|&id| scene.contains(id)) else {
            return Err("Select the objects to scatter, then the surface last".into());
        };
        // The objects to copy: not the surface, nor one inside another
        // (it is copied with it).
        let selected = |id: NodeId| id != surface && scene.contains(id) && context.selection.contains(&id);
        let sources: Vec<NodeId> = context
            .selection
            .iter()
            .copied()
            .filter(|&id| selected(id))
            .filter(|&id| {
                !context
                    .selection
                    .iter()
                    .any(|&other| other != id && selected(other) && scene.is_ancestor_of(other, id))
            })
            .collect();
        if sources.is_empty() {
            return Err("Select the objects to scatter, then the surface last".into());
        }
        let Some(surface) = context
            .mesh(surface)
            .map(|mesh| Surface::new(&mesh, context.world_matrix(surface)))
            .filter(|s| !s.triangles.is_empty())
        else {
            return Err("The object selected last has no polygons to scatter on".into());
        };

        let count = usize::try_from(options.int("Count")).unwrap_or(1);
        let mut random = Random(u64::try_from(options.int("Seed")).unwrap_or(0));
        let align = options.bool("Align to Surface");
        let turn = options.float("Random Rotation");
        let variation = options.float("Scale Variation");

        let mut changes = Changes::default();
        let group = changes.insert(
            Parent::Root,
            NodeData::new(scene.unique_name("Scatter"), ObjectKind::Null),
        );
        for _ in 0..count {
            let source = sources[random.below(sources.len())];
            let (point, normal) = surface.sample(&mut random);
            let up = if align {
                Quat::from_rotation_arc(Vec3::Y, normal)
            } else {
                Quat::IDENTITY
            };
            let spin = Quat::from_rotation_y(random.signed() * turn);
            let factor = 1.0 + random.signed() * variation;
            // The source's own turn and size, stood up at the point.
            let (scale, rotation, _) = context.world_matrix(source).to_scale_rotation_translation();
            let placed = Mat4::from_rotation_translation(up * spin, point)
                * Mat4::from_scale_rotation_translation(scale * factor, rotation, Vec3::ZERO);
            copy(
                context,
                source,
                Parent::New(group),
                Some(Transform::from_matrix(&placed)),
                &mut changes,
            );
        }
        Ok(changes)
    }
}

/// Adds a copy of `id` and everything under it below `parent`; `placed`
/// replaces its own transform (whose keys are dropped: it stays where it is put).
fn copy(context: &Context<'_>, id: NodeId, parent: Parent, placed: Option<Transform>, changes: &mut Changes) {
    let scene = &context.document.scene;
    let Some(node) = scene.get(id) else { return };
    let mut data = node.data.clone();
    data.unkeyed = None;
    if let Some(transform) = placed {
        data.transform = transform;
        data.tracks.retain(|t| matches!(t.param, Param::Object(_)));
    }
    let new = changes.insert(parent, data);
    for &child in scene.children(id) {
        copy(context, child, Parent::New(new), None, changes);
    }
}

#[cfg(test)]
mod tests {
    use grid4d_geom::PolyMesh;
    use grid4d_plugin::{Change, Registry};
    use grid4d_scene::{Document, MeshHandle, Node, PluginObject};

    use super::*;

    const SELECT: &str = "Select the objects to scatter, then the surface last";

    /// A 200 × 200 floor at y = 10 (a polygon object), a cube with a child,
    /// and a sphere.
    fn scene() -> (Document, NodeId, NodeId, NodeId) {
        let mut doc = Document::new("scatter");
        let floor = PolyMesh::from_faces(
            vec![
                Vec3::new(-100.0, 0.0, 100.0),
                Vec3::new(100.0, 0.0, 100.0),
                Vec3::new(100.0, 0.0, -100.0),
                Vec3::new(-100.0, 0.0, -100.0),
            ],
            [[0, 1, 2, 3]],
        )
        .unwrap();
        let mut floor = Node::new(
            "Floor",
            ObjectKind::Polygon {
                mesh: MeshHandle::new(floor),
                components: grid4d_scene::Components::default(),
            },
        );
        floor.data.transform.position.y = 10.0;
        let floor = doc.scene.insert(floor);
        let cube = doc.scene.insert(Node::new("Cube", ObjectKind::Null));
        doc.scene
            .insert_child(cube, Node::new("Inside", ObjectKind::Null))
            .unwrap();
        let sphere = doc.scene.insert(Node::new("Sphere", ObjectKind::Null));
        (doc, floor, cube, sphere)
    }

    fn run(doc: &Document, selection: &[NodeId], options: &PluginObject) -> Result<Changes, String> {
        let shapes = |id: NodeId| match &doc.scene.get(id)?.data.kind {
            ObjectKind::Polygon { mesh, .. } => Some(mesh.mesh().clone()),
            _ => None,
        };
        let context = Context::new(doc, selection, selection.last().copied(), 0.0, &shapes);
        Scatter.run(&context, &Values::new(PARAMS, options))
    }

    /// The placed copies (not their children): name and transform.
    fn placed(changes: &Changes) -> Vec<(String, Transform)> {
        changes
            .list()
            .iter()
            .filter_map(|c| match c {
                Change::Insert {
                    parent: Parent::New(p),
                    data,
                } if p.index() == 0 => Some((data.name.clone(), data.transform)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn copies_land_on_the_surface_the_same_for_the_same_seed() {
        let (doc, floor, cube, sphere) = scene();
        let mut options = Registry::new_object(&Scatter);
        options.set("Scale Variation", PluginValue::Float(0.5));
        let changes = run(&doc, &[cube, sphere, floor], &options).unwrap();
        let copies = placed(&changes);
        assert_eq!(copies.len(), 20);
        // A group, 20 copies, and the cube's child under each cube copy.
        let cubes = copies.iter().filter(|(n, _)| n == "Cube").count();
        assert!(cubes > 0 && cubes < 20, "both objects picked: {cubes}");
        assert_eq!(changes.list().len(), 1 + 20 + cubes);
        assert!(matches!(&changes.list()[0], Change::Insert { parent: Parent::Root, data } if data.name == "Scatter"));
        for (_, t) in &copies {
            assert!(t.position.x.abs() <= 100.0 && t.position.z.abs() <= 100.0);
            assert!((t.position.y - 10.0).abs() < 1e-3, "on the floor");
            assert!((0.5 - 1e-3..=1.5 + 1e-3).contains(&t.scale.x));
        }
        assert_eq!(
            run(&doc, &[cube, sphere, floor], &options).unwrap(),
            changes,
            "same seed"
        );
        options.set("Seed", PluginValue::Int(2));
        assert_ne!(placed(&run(&doc, &[cube, sphere, floor], &options).unwrap()), copies);
    }

    #[test]
    fn it_says_what_to_select() {
        let (doc, floor, cube, _) = scene();
        let options = Registry::new_object(&Scatter);
        assert_eq!(run(&doc, &[], &options), Err(SELECT.to_owned()));
        assert_eq!(
            run(&doc, &[floor], &options),
            Err(SELECT.to_owned()),
            "only the surface"
        );
        assert!(run(&doc, &[floor, cube], &options).unwrap_err().contains("no polygons"));
    }

    #[test]
    fn the_random_numbers_stay_in_range() {
        let mut random = Random(7);
        for _ in 0..1000 {
            let u = random.unit();
            assert!((0.0..1.0).contains(&u));
            assert!(random.below(3) < 3);
        }
    }
}
