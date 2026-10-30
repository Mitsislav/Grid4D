//! A document's simulation: its bodies, built from the evaluated start frame,
//! stepped frame by frame into a [`SimCache`].

use glam::{Mat4, Quat, Vec3};
use grid4d_eval::{EvaluatedInstance, EvaluatedScene, MeshKey, Overrides};
use grid4d_scene::{Body, CachedBody, Document, Inherit, Motion, NodeId, ObjectKind, Scene, SimulationCache, Tag};

use crate::{BodyId, World, shape};

/// What a simulated body stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BodyKey {
    /// An object with a Dynamics tag (and, Compound, its children).
    Node(NodeId),
    /// Clone `n` of a Cloner with a Dynamics tag (all its parts).
    Clone(NodeId, u32),
}

/// Every body's pose at every simulated frame, from the start frame on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SimCache {
    start: i32,
    bodies: Vec<BodyKey>,
    /// Each body's scale at the start (the engine has none; it stays).
    scales: Vec<Vec3>,
    /// Frames × bodies: position (cm) and rotation.
    poses: Vec<(Vec3, Quat)>,
}

impl SimCache {
    /// The bodies, in the order their poses are stored.
    #[must_use]
    pub fn bodies(&self) -> &[BodyKey] {
        &self.bodies
    }

    /// The first frame (the start state).
    #[must_use]
    pub const fn start(&self) -> i32 {
        self.start
    }

    /// The last frame simulated (`start - 1` while empty).
    #[must_use]
    pub fn last(&self) -> i32 {
        let frames = self.poses.len().checked_div(self.bodies.len()).unwrap_or(0);
        self.start + i32::try_from(frames).unwrap_or(i32::MAX) - 1
    }

    /// Body `body`'s world matrix at `frame`; frames before the start show the
    /// start state; `None` past the last simulated frame.
    #[must_use]
    pub fn world(&self, frame: i32, body: usize) -> Option<Mat4> {
        let at = usize::try_from(frame.max(self.start) - self.start).ok()?;
        let (position, rotation) = *self.poses.get(at * self.bodies.len() + body)?;
        Some(Mat4::from_scale_rotation_translation(
            *self.scales.get(body)?,
            rotation,
            position,
        ))
    }

    /// Every body's place at `frame` (the last simulated one if `frame` is not
    /// yet), into `out`.
    pub fn overrides(&self, frame: i32, out: &mut Overrides) {
        out.clear();
        let frame = frame.min(self.last());
        for (index, key) in self.bodies.iter().enumerate() {
            let Some(world) = self.world(frame, index) else {
                continue;
            };
            match *key {
                BodyKey::Node(id) => out.set(id, world),
                // A clone moves by its change since the start, its parts with it.
                BodyKey::Clone(cloner, n) => {
                    if let Some(start) = self.world(self.start, index) {
                        out.set_clone(cloner, n, world * start.inverse());
                    }
                }
            }
        }
    }

    /// The cache as saved with a document.
    #[must_use]
    pub fn to_saved(&self) -> SimulationCache {
        SimulationCache {
            start: self.start,
            bodies: self
                .bodies
                .iter()
                .zip(&self.scales)
                .map(|(key, &scale)| match *key {
                    BodyKey::Node(node) => CachedBody {
                        node,
                        clone: None,
                        scale,
                    },
                    BodyKey::Clone(node, n) => CachedBody {
                        node,
                        clone: Some(n),
                        scale,
                    },
                })
                .collect(),
            poses: self.poses.clone(),
        }
    }

    /// A cache saved with a document; `None` if its poses do not fill whole
    /// frames.
    #[must_use]
    pub fn from_saved(saved: SimulationCache) -> Option<Self> {
        let count = saved.bodies.len();
        if count == 0 || !saved.poses.len().is_multiple_of(count) || saved.poses.is_empty() {
            return None;
        }
        let (bodies, scales) = saved
            .bodies
            .iter()
            .map(|b| {
                let key = b.clone.map_or(BodyKey::Node(b.node), |n| BodyKey::Clone(b.node, n));
                (key, b.scale)
            })
            .unzip();
        Some(Self {
            start: saved.start,
            bodies,
            scales,
            poses: saved.poses,
        })
    }

    /// `true` if `other` starts the same way: the same bodies (scales too) in
    /// the same order, at the same start frame and start poses. A saved cache
    /// that agrees with a fresh simulation of its document is still valid.
    #[must_use]
    pub fn agrees(&self, other: &Self) -> bool {
        let count = self.bodies.len();
        self.start == other.start
            && self.bodies == other.bodies
            && self
                .scales
                .iter()
                .zip(&other.scales)
                .all(|(a, b)| a.abs_diff_eq(*b, 1e-3))
            && self.poses.len() >= count
            && other.poses.len() >= count
            && self.poses[..count]
                .iter()
                .zip(&other.poses[..count])
                .all(|((p, q), (p2, q2))| p.abs_diff_eq(*p2, 1e-2) && q.abs_diff_eq(*q2, 1e-4))
    }

    /// Bytes the poses take.
    #[must_use]
    pub fn memory(&self) -> usize {
        self.poses.len() * size_of::<(Vec3, Quat)>()
    }
}

/// One body: where it is in the engine, and what moves a collider.
#[derive(Debug)]
struct SimBody {
    id: BodyId,
    /// An animated collider follows this node's animation.
    follows: Option<NodeId>,
}

/// A document's rigid bodies in a physics world, and the poses so far.
#[derive(Debug)]
pub struct Simulation {
    world: World,
    bodies: Vec<SimBody>,
    fps: u32,
    cache: SimCache,
    /// The scene's animation, for animated colliders.
    scene: Scene,
}

impl Simulation {
    /// The simulation of `doc`, starting from `evaluated` — the document
    /// evaluated at its start frame (what the viewport shows there). Nothing
    /// is simulated while the settings are off.
    #[must_use]
    pub fn new(doc: &Document, evaluated: &EvaluatedScene) -> Self {
        let fps = doc.time.fps();
        let start = doc.time.start();
        let mut world = World::new(&doc.simulation, fps);
        let mut bodies = Vec::new();
        let mut cache = SimCache {
            start,
            ..SimCache::default()
        };
        if doc.simulation.enabled {
            gather(doc, evaluated, &mut world, &mut bodies, &mut cache);
        }
        // The start state is the first frame.
        record(&world, &bodies, &mut cache);
        Self {
            world,
            bodies,
            fps,
            cache,
            scene: doc.scene.clone(),
        }
    }

    /// The simulation of `doc` from its start frame, evaluated as the viewport
    /// shows it.
    #[must_use]
    pub fn of(doc: &Document) -> Self {
        let mut evaluated = EvaluatedScene::new();
        evaluated.set_frame(f64::from(doc.time.start()));
        evaluated.update(&doc.scene);
        Self::new(doc, &evaluated)
    }

    /// Simulates up to `frame` (nothing if it is already cached).
    pub fn step_to(&mut self, frame: i32) {
        while self.cache.last() < frame && !self.bodies.is_empty() {
            self.step();
        }
    }

    /// Simulates one more frame.
    pub fn step(&mut self) {
        let from = f64::from(self.cache.last());
        let steps = self.world.steps_per_frame().max(1);
        for step in 1..=steps {
            // Animated colliders where their animation puts them at the
            // sub-step's end.
            let at = from + f64::from(step) / f64::from(steps);
            for body in &self.bodies {
                if let Some(node) = body.follows {
                    let (_, rotation, position) = self.scene.world_matrix_at(node, at).to_scale_rotation_translation();
                    self.world.move_kinematic(body.id, position, rotation);
                }
            }
            self.world.substep();
        }
        record(&self.world, &self.bodies, &mut self.cache);
    }

    /// The poses so far.
    #[must_use]
    pub const fn cache(&self) -> &SimCache {
        &self.cache
    }

    /// Frames per second of the document simulated.
    #[must_use]
    pub const fn fps(&self) -> u32 {
        self.fps
    }
}

/// A copy of a cache that simulates the frames it lacks itself (renders take
/// one: the motion is the same, a run being deterministic).
#[derive(Debug)]
pub struct Replay {
    doc: Document,
    cache: SimCache,
    simulation: Option<Simulation>,
}

impl Replay {
    #[must_use]
    pub const fn new(doc: Document, cache: SimCache) -> Self {
        Self {
            doc,
            cache,
            simulation: None,
        }
    }

    /// Every body's place at `frame`, simulating up to it if needed.
    pub fn overrides(&mut self, frame: i32, out: &mut Overrides) {
        if frame > self.cache.last() {
            let simulation = self.simulation.get_or_insert_with(|| Simulation::of(&self.doc));
            simulation.step_to(frame);
        }
        self.simulation
            .as_ref()
            .map_or(&self.cache, Simulation::cache)
            .overrides(frame, out);
    }
}

/// Appends every body's current pose to `cache`.
fn record(world: &World, bodies: &[SimBody], cache: &mut SimCache) {
    for body in bodies {
        cache
            .poses
            .push(world.pose(body.id).unwrap_or((Vec3::ZERO, Quat::IDENTITY)));
    }
}

/// Every enabled body of `doc` as shown in `evaluated`, into `world`.
fn gather(
    doc: &Document,
    evaluated: &EvaluatedScene,
    world: &mut World,
    bodies: &mut Vec<SimBody>,
    cache: &mut SimCache,
) {
    let scene = &doc.scene;
    let frame = f64::from(doc.time.start());
    let nodes = evaluated.nodes();
    // Inside a Compound body: (its depth) — its tags are the body's.
    let mut compound_depth: Option<u32> = None;
    for (index, node) in nodes.iter().enumerate() {
        if compound_depth.is_some_and(|d| node.depth > d) {
            continue;
        }
        compound_depth = None;
        let Some((body, data)) = scene
            .get(node.id)
            .and_then(|n| Some((dynamics(&n.data.tags)?, &n.data)))
        else {
            continue;
        };
        if !body.enabled {
            continue;
        }
        // A Cloner's tag makes each of its clones a body.
        if matches!(data.kind, ObjectKind::Cloner { .. }) {
            clone_bodies(scene, evaluated, node.id, &body, frame, world, bodies, cache);
            continue;
        }
        let (scale, rotation, position) = node.world.to_scale_rotation_translation();
        let to_body = Mat4::from_rotation_translation(rotation, position).inverse();
        // The body's parts: the object, and with Compound its visible children.
        let mut colliders = Vec::new();
        let end = if body.inherit == Inherit::Compound {
            compound_depth = Some(node.depth);
            index + 1 + nodes[index + 1..].iter().take_while(|n| n.depth > node.depth).count()
        } else {
            index + 1
        };
        for part in &nodes[index..end] {
            if let Some(mesh) = shown_mesh(scene, evaluated, part, frame) {
                colliders.extend(shape::colliders(&mesh, to_body * part.world, &body));
            }
        }
        if colliders.is_empty() {
            continue;
        }
        let id = world.insert(position, rotation, &body, colliders);
        let follows = (body.motion == Motion::Collider && animated(scene, node.id)).then_some(node.id);
        bodies.push(SimBody { id, follows });
        cache.bodies.push(BodyKey::Node(node.id));
        cache.scales.push(scale);
    }
}

/// Every clone of `cloner` as a body: its parts' shapes in the frame of its
/// first part.
#[expect(clippy::too_many_arguments, reason = "the parts of one gathering pass")]
fn clone_bodies(
    scene: &Scene,
    evaluated: &EvaluatedScene,
    cloner: NodeId,
    body: &Body,
    frame: f64,
    world: &mut World,
    bodies: &mut Vec<SimBody>,
    cache: &mut SimCache,
) {
    let clones: Vec<&EvaluatedInstance> = evaluated.instances().iter().filter(|c| c.owner == cloner).collect();
    let mut start = 0;
    while let Some(first) = clones.get(start) {
        let index = first.index;
        let end = start + clones[start..].iter().take_while(|c| c.index == index).count();
        let parts = &clones[start..end];
        start = end;
        let (scale, rotation, position) = first.world.to_scale_rotation_translation();
        let to_body = Mat4::from_rotation_translation(rotation, position).inverse();
        let mut colliders = Vec::new();
        for part in parts {
            if let Some(mesh) = clone_mesh(scene, evaluated, part, frame) {
                colliders.extend(shape::colliders(&mesh, to_body * part.world, body));
            }
        }
        if colliders.is_empty() {
            continue;
        }
        let id = world.insert(position, rotation, body, colliders);
        bodies.push(SimBody { id, follows: None });
        cache.bodies.push(BodyKey::Clone(cloner, index));
        cache.scales.push(scale);
    }
}

/// The surface a clone's part shows, in its own space.
fn clone_mesh(
    scene: &Scene,
    evaluated: &EvaluatedScene,
    part: &EvaluatedInstance,
    frame: f64,
) -> Option<grid4d_geom::PolyMesh> {
    let drawable = part.drawable.filter(|d| !d.unlit && !d.marker)?;
    match drawable.key {
        MeshKey::Polygon(id) => evaluated.mesh(id).map(|m| m.mesh().clone()),
        _ => grid4d_eval::editable_mesh(&scene.get(part.source)?.data.kind_at(frame)),
    }
}

/// The node's Dynamics tag, if it has one.
fn dynamics(tags: &[Tag]) -> Option<Body> {
    tags.iter().find_map(|tag| match tag {
        Tag::Dynamics(body) => Some(*body),
        Tag::Phong { .. } | Tag::Protection { .. } | Tag::Target { .. } | Tag::Material { .. } => None,
    })
}

/// The surface a node shows (deformed, generated or its own), in its own
/// space; `None` for curves, markers and objects without one.
fn shown_mesh(
    scene: &Scene,
    evaluated: &EvaluatedScene,
    node: &grid4d_eval::EvaluatedNode,
    frame: f64,
) -> Option<grid4d_geom::PolyMesh> {
    let drawable = node.drawable.filter(|d| !d.unlit && !d.marker)?;
    match drawable.key {
        MeshKey::Polygon(id) => evaluated.mesh(id).map(|m| m.mesh().clone()),
        _ => grid4d_eval::editable_mesh(&scene.get(node.id)?.data.kind_at(frame)),
    }
}

/// Whether `id` or an object above it is animated.
fn animated(scene: &Scene, id: NodeId) -> bool {
    let mut at = Some(id);
    while let Some(current) = at {
        let Some(node) = scene.get(current) else { break };
        if !node.data.tracks.is_empty() {
            return true;
        }
        at = node.parent();
    }
    false
}

#[cfg(test)]
mod tests {
    use grid4d_scene::{Axis, Curve, Key, Node, NodeData, ObjectKind, ObjectKindTag, Param, SimulationSettings, Track};

    use super::*;

    fn tagged(name: &str, kind: ObjectKind, position: Vec3, body: Body) -> Node {
        let mut data = NodeData::new(name, kind);
        data.transform.position = position;
        data.tags.push(Tag::Dynamics(body));
        Node::from_data(data)
    }

    fn floor() -> Node {
        tagged(
            "Floor",
            ObjectKind::Plane {
                width: 2000.0,
                depth: 2000.0,
                width_segments: 2,
                depth_segments: 2,
            },
            Vec3::ZERO,
            Body::collider(),
        )
    }

    fn cube(size: f32) -> ObjectKind {
        ObjectKind::Cube {
            size: Vec3::splat(size),
            segments: [1, 1, 1],
        }
    }

    fn simulate(doc: &Document, frames: i32) -> Simulation {
        let mut evaluated = EvaluatedScene::new();
        evaluated.set_frame(f64::from(doc.time.start()));
        evaluated.update(&doc.scene);
        let mut simulation = Simulation::new(doc, &evaluated);
        simulation.step_to(doc.time.start() + frames);
        simulation
    }

    fn height(simulation: &Simulation, frame: i32, body: usize) -> f32 {
        simulation.cache().world(frame, body).unwrap().w_axis.y
    }

    #[test]
    fn a_cube_falls_onto_a_collider_and_rests_on_it() {
        let mut doc = Document::new("fall");
        doc.scene.insert(floor());
        doc.scene
            .insert(tagged("Cube", cube(100.0), Vec3::Y * 300.0, Body::rigid()));
        let simulation = simulate(&doc, 90);
        let cache = simulation.cache();
        assert_eq!(cache.bodies().len(), 2);
        assert_eq!((cache.start(), cache.last()), (0, 90));
        assert_eq!(height(&simulation, 0, 1), 300.0, "the start state");
        assert_eq!(height(&simulation, -5, 1), 300.0, "before the start: the start state");
        assert!(height(&simulation, 10, 1) < 300.0, "falling");
        let rest = height(&simulation, 90, 1);
        assert!((rest - 50.0).abs() < 1.5, "on the floor by its half-size: {rest}");
        assert_eq!(height(&simulation, 90, 0), 0.0, "the floor stays");
        assert!(cache.world(91, 1).is_none(), "not simulated yet");
    }

    #[test]
    fn a_ball_bounces_lower_each_time() {
        let mut doc = Document::new("bounce");
        doc.scene.insert(floor());
        let ball = Body {
            bounce: 0.8,
            shape: grid4d_scene::CollisionShape::Sphere,
            ..Body::rigid()
        };
        let sphere = ObjectKind::Sphere {
            radius: 25.0,
            segments: 16,
        };
        doc.scene.insert(tagged("Ball", sphere, Vec3::Y * 300.0, ball));
        let simulation = simulate(&doc, 120);
        let heights: Vec<f32> = (0..=120).map(|f| height(&simulation, f, 1)).collect();
        // The tops of the bounces: higher than both neighbours.
        let peaks: Vec<f32> = heights
            .windows(3)
            .filter(|w| w[1] > w[0] && w[1] >= w[2] && w[1] > 30.0)
            .map(|w| w[1])
            .collect();
        assert!(peaks.len() >= 2, "{heights:?}");
        assert!(peaks.windows(2).all(|p| p[1] < p[0]), "{peaks:?}");
        assert!(peaks[0] < 300.0 && peaks[0] > 100.0, "{peaks:?}");
    }

    #[test]
    fn a_box_slides_further_on_less_friction() {
        let slide = |friction: f32| {
            let mut doc = Document::new("slope");
            let mut slope = floor();
            slope.data.transform.rotation.z = 0.35; // a 20° slope, falling towards −X
            if let Some(Tag::Dynamics(body)) = slope.data.tags.first_mut() {
                body.friction = 1.0;
            }
            doc.scene.insert(slope);
            let body = Body {
                friction,
                shape: grid4d_scene::CollisionShape::Box,
                ..Body::rigid()
            };
            let mut node = tagged("Box", cube(50.0), Vec3::Y * 30.0, body);
            node.data.transform.rotation.z = 0.35;
            doc.scene.insert(node);
            let simulation = simulate(&doc, 45);
            let start = simulation.cache().world(0, 1).unwrap().w_axis.x;
            start - simulation.cache().world(45, 1).unwrap().w_axis.x
        };
        let (icy, rough) = (slide(0.05), slide(1.0));
        assert!(icy > 100.0, "slid {icy}");
        assert!(icy > rough * 3.0, "{icy} vs {rough}");
    }

    #[test]
    fn an_animated_collider_pushes_a_dynamic_body() {
        let mut doc = Document::new("push");
        doc.scene.insert(floor());
        let mut paddle = tagged("Paddle", cube(100.0), Vec3::new(-300.0, 50.0, 0.0), Body::collider());
        paddle.data.tracks.push(Track {
            param: Param::Position(Axis::X),
            curve: Curve::from_keys([Key::new(0.0, -300.0), Key::new(30.0, 300.0)]),
        });
        doc.scene.insert(paddle);
        doc.scene
            .insert(tagged("Box", cube(50.0), Vec3::new(0.0, 25.0, 0.0), Body::rigid()));
        let simulation = simulate(&doc, 30);
        let paddle_at = simulation.cache().world(30, 1).unwrap().w_axis.x;
        assert!(
            (paddle_at - 300.0).abs() < 1.0,
            "the paddle follows its keys: {paddle_at}"
        );
        let pushed = simulation.cache().world(30, 2).unwrap().w_axis.x;
        assert!(pushed > 200.0, "pushed along: {pushed}");
    }

    #[test]
    fn identical_runs_agree_and_disabled_bodies_or_settings_simulate_nothing() {
        let mut doc = Document::new("same");
        doc.scene.insert(floor());
        for i in 0..5 {
            let at = Vec3::new(
                f32::from(u8::try_from(i).unwrap()) * 20.0,
                100.0 + 120.0 * f32::from(u8::try_from(i).unwrap()),
                0.0,
            );
            doc.scene.insert(tagged("Cube", cube(60.0), at, Body::rigid()));
        }
        assert_eq!(simulate(&doc, 60).cache(), simulate(&doc, 60).cache(), "deterministic");

        // A Compound group is one body; disabled tags take no part.
        let group = doc.scene.insert(tagged(
            "Group",
            ObjectKind::Null,
            Vec3::Y * 500.0,
            Body {
                inherit: Inherit::Compound,
                ..Body::rigid()
            },
        ));
        doc.scene
            .insert_child(group, Node::new("Part", ObjectKindTag::Sphere.default_kind()))
            .unwrap();
        doc.scene.insert(tagged(
            "Off",
            cube(10.0),
            Vec3::ZERO,
            Body {
                enabled: false,
                ..Body::rigid()
            },
        ));
        assert_eq!(simulate(&doc, 1).cache().bodies().len(), 7);
        doc.simulation = SimulationSettings {
            enabled: false,
            ..SimulationSettings::default()
        };
        let off = simulate(&doc, 30);
        assert!(off.cache().bodies().is_empty());
    }

    #[test]
    fn a_tagged_cloner_makes_every_clone_a_body() {
        use grid4d_geom::cloner::{Cloner, GridArray, Layout};
        let mut doc = Document::new("clones");
        doc.scene.insert(floor());
        let cloner = doc.scene.insert(tagged(
            "Cloner",
            ObjectKind::cloner(Cloner {
                layout: Layout::Grid(GridArray {
                    count: [3, 3, 3],
                    size: Vec3::splat(70.0),
                    ..GridArray::default()
                }),
                ..Cloner::default()
            }),
            Vec3::Y * 300.0,
            Body::rigid(),
        ));
        // A cloned group: two cubes, one above the other.
        let group = doc
            .scene
            .insert_child(cloner, Node::new("Group", ObjectKind::Null))
            .unwrap();
        for y in [0.0, 25.0] {
            doc.scene
                .insert_child(group, Node::new("Cube", cube(20.0)).with_position(Vec3::Y * y))
                .unwrap();
        }
        let simulation = simulate(&doc, 90);
        let cache = simulation.cache();
        assert_eq!(cache.bodies().len(), 1 + 27, "the floor and every clone");
        assert_eq!(cache.bodies()[1], BodyKey::Clone(cloner, 0));
        let mut overrides = Overrides::default();
        cache.overrides(90, &mut overrides);
        let mut eval = EvaluatedScene::new();
        eval.set_frame(90.0);
        eval.set_overrides(&overrides);
        eval.update(&doc.scene);
        let clones = eval.instances();
        assert_eq!(clones.len(), 54);
        // Every clone has fallen to the floor (the grid was 230 – 370 cm up) …
        assert!(
            clones.iter().all(|c| c.world.w_axis.y < 150.0),
            "{:?}",
            clones.iter().map(|c| c.world.w_axis.y).collect::<Vec<_>>()
        );
        // … and each clone's two cubes moved as one: still 25 cm apart.
        for pair in clones.chunks(2) {
            let gap = pair[0]
                .world
                .w_axis
                .truncate()
                .distance(pair[1].world.w_axis.truncate());
            assert!((gap - 25.0).abs() < 0.5, "{gap}");
        }
        // They spread out: no longer in a 70 cm grid's 140 cm square.
        let xs = clones.iter().map(|c| c.world.w_axis.x);
        let spread = xs.clone().fold(f32::MIN, f32::max) - xs.fold(f32::MAX, f32::min);
        assert!(spread > 140.0, "{spread}");
    }
}
