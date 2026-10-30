//! The editor's simulation (see `docs/reference/simulation-cache.md`): the
//! document's rigid bodies simulated a little each refresh into a cache that
//! the viewport, renders and exports read; dropped when the document changes
//! in a way that alters the motion.

use std::time::{Duration, Instant};

use glam::Mat4;
use grid4d_eval::Overrides;
use grid4d_scene::{Document, NodeData, NodeId, Scene, SimulationCache, SimulationSettings, Tag, Track};
use grid4d_sim::{Replay, SimCache, Simulation};

/// Time spent simulating per refresh.
const BUDGET: Duration = Duration::from_millis(8);
/// … and while baking.
const BAKE_BUDGET: Duration = Duration::from_millis(40);

/// The document's simulation and its cache.
#[derive(Debug, Default)]
pub struct Sim {
    simulation: Option<Simulation>,
    /// What the simulation was made from.
    key: Option<Key>,
    /// The scene revision the key was last checked at.
    checked: Option<u64>,
    /// Bake: simulate up to this frame now.
    baking: Option<i32>,
    /// Frames saved with the document (shown until the simulation passes them).
    saved: Option<SimCache>,
    /// The cache reached the memory limit: no more frames are simulated.
    full: bool,
}

/// Everything a simulation depends on.
#[derive(Debug, PartialEq)]
struct Key {
    settings: SimulationSettings,
    fps: u32,
    start: i32,
    bodies: Vec<BodyInputs>,
}

/// One object with a Dynamics tag: its own data and everything below it, where
/// it starts, what above it can move or hide it, and — for a Cloner — its
/// effectors and Object mode object (where they are, what they are).
#[derive(Debug, PartialEq)]
struct BodyInputs {
    id: NodeId,
    start: Mat4,
    subtree: Vec<NodeData>,
    above: Vec<(Vec<Track>, bool, bool)>,
    links: Vec<(Mat4, NodeData)>,
}

impl Sim {
    /// Drops the simulation if `doc` changed in a way that alters it
    /// (selecting, renaming or editing other objects keeps it).
    pub fn sync(&mut self, doc: &Document) {
        let revision = doc.scene.revision();
        if self.checked == Some(revision) && self.key.as_ref().is_some_and(|k| k.matches_document(doc)) {
            return;
        }
        self.checked = Some(revision);
        let key = Key::of(doc);
        if self.key.as_ref() != Some(&key) {
            self.simulation = None;
            self.saved = None;
            self.full = false;
            self.key = Some(key);
        }
    }

    /// Simulates towards `frame` (or, baking, the end) for at most a refresh's
    /// budget. Returns `true` while there is more to do (ask for a repaint).
    pub fn advance(&mut self, doc: &Document, frame: i32) -> bool {
        self.sync(doc);
        let simulation = self.simulation.get_or_insert_with(|| Simulation::of(doc));
        if simulation.cache().bodies().is_empty() {
            self.baking = None;
            return false;
        }
        let target = self.baking.map_or(frame, |end| end.max(frame));
        // Saved frames cover it: nothing to simulate.
        if self.saved.as_ref().is_some_and(|s| s.last() >= target) {
            self.baking = None;
            return false;
        }
        let limit = usize::try_from(doc.simulation.memory_limit_mb)
            .unwrap_or(usize::MAX)
            .saturating_mul(1 << 20);
        let budget = if self.baking.is_some() { BAKE_BUDGET } else { BUDGET };
        let began = Instant::now();
        while simulation.cache().last() < target && began.elapsed() < budget {
            if simulation.cache().memory() >= limit {
                self.full = true;
                break;
            }
            simulation.step();
        }
        let more = simulation.cache().last() < target && !self.full;
        if !more {
            self.baking = None;
        }
        more
    }

    /// The cache reached the Project settings' memory limit.
    #[must_use]
    pub const fn is_full(&self) -> bool {
        self.full
    }

    /// The fuller of the simulation's and the saved cache.
    fn best(&self) -> Option<&SimCache> {
        let running = self.simulation.as_ref().map(Simulation::cache);
        match (running, self.saved.as_ref()) {
            (Some(a), Some(b)) => Some(if b.last() > a.last() { b } else { a }),
            (a, b) => a.or(b),
        }
    }

    /// The frames to save with the document; `None` when nothing is simulated.
    #[must_use]
    pub fn to_save(&mut self, doc: &Document) -> Option<SimulationCache> {
        self.sync(doc);
        self.best()
            .filter(|cache| !cache.bodies().is_empty())
            .map(SimCache::to_saved)
    }

    /// Takes frames saved with a just-opened `doc`, if they still agree with
    /// it (the same bodies starting the same way); returns whether they did.
    pub fn adopt(&mut self, doc: &Document, saved: SimulationCache) -> bool {
        self.sync(doc);
        let fresh = Simulation::of(doc);
        match SimCache::from_saved(saved).filter(|cache| cache.agrees(fresh.cache())) {
            Some(cache) => {
                self.saved = Some(cache);
                self.simulation = Some(fresh);
                true
            }
            None => false,
        }
    }

    /// The simulated objects' places at `frame` (the latest simulated frame
    /// until `frame` is), into `out`; nothing without a simulation.
    pub fn overrides(&self, frame: i32, out: &mut Overrides) {
        match self.best() {
            Some(cache) => cache.overrides(frame, out),
            None => out.clear(),
        }
    }

    /// The cached frames (first, last), if anything is simulated.
    #[must_use]
    pub fn cached(&self) -> Option<(i32, i32)> {
        let cache = self.best()?;
        (!cache.bodies().is_empty()).then(|| (cache.start(), cache.last()))
    }

    /// Simulate the whole range now (Simulate ▸ Bake Simulation).
    pub fn bake(&mut self, doc: &Document) {
        self.baking = Some(doc.time.end());
    }

    /// Stops baking (Esc); what is done stays cached. `false` if not baking.
    pub fn stop_baking(&mut self) -> bool {
        self.baking.take().is_some()
    }

    /// Baking: (the last frame simulated, the frame it goes to).
    #[must_use]
    pub fn baking(&self) -> Option<(i32, i32)> {
        let target = self.baking?;
        Some((self.cached().map_or(target, |(_, last)| last), target))
    }

    /// Empties the cache (Simulate ▸ Clear Simulation Cache); it refills as
    /// frames are visited.
    pub fn clear(&mut self) {
        self.simulation = None;
        self.saved = None;
        self.baking = None;
        self.full = false;
    }

    /// A copy of the cache for a render, which simulates what it lacks; `None`
    /// when nothing is simulated.
    #[must_use]
    pub fn replay(&mut self, doc: &Document) -> Option<Replay> {
        self.sync(doc);
        self.simulation.get_or_insert_with(|| Simulation::of(doc));
        let cache = self.best()?;
        (!cache.bodies().is_empty()).then(|| Replay::new(doc.clone(), cache.clone()))
    }
}

impl Key {
    fn of(doc: &Document) -> Self {
        let scene = &doc.scene;
        let start = doc.time.start();
        let mut bodies = Vec::new();
        scene.walk(&mut Vec::new(), |id, node, _, _| {
            if node.data.tags.iter().any(|t| matches!(t, Tag::Dynamics(_))) {
                bodies.push(BodyInputs {
                    id,
                    start: scene.world_matrix_at(id, f64::from(start)),
                    subtree: subtree(scene, id),
                    above: above(scene, id),
                    links: links(scene, &node.data, start),
                });
            }
            true
        });
        Self {
            settings: motion_settings(&doc.simulation),
            fps: doc.time.fps(),
            start,
            bodies,
        }
    }

    /// The settings and time the simulation was made with still hold.
    fn matches_document(&self, doc: &Document) -> bool {
        self.settings == motion_settings(&doc.simulation)
            && self.fps == doc.time.fps()
            && self.start == doc.time.start()
    }
}

/// The settings that change the motion (the memory limit does not).
fn motion_settings(settings: &SimulationSettings) -> SimulationSettings {
    SimulationSettings {
        memory_limit_mb: 0,
        ..*settings
    }
}

/// The data of `id` and everything below it, in pre-order (names left out:
/// renaming changes no motion).
fn subtree(scene: &Scene, id: NodeId) -> Vec<NodeData> {
    let mut found = Vec::new();
    let mut stack = vec![id];
    while let Some(at) = stack.pop() {
        let Some(node) = scene.get(at) else { continue };
        let mut data = node.data.clone();
        data.name.clear();
        found.push(data);
        stack.extend(node.children().iter().rev());
    }
    found
}

/// A Cloner's effectors and Object mode object: where each is at `start` and
/// its data; nothing for other objects.
fn links(scene: &Scene, data: &NodeData, start: i32) -> Vec<(Mat4, NodeData)> {
    let grid4d_scene::ObjectKind::Cloner { object, effectors, .. } = &data.kind else {
        return Vec::new();
    };
    effectors
        .iter()
        .chain(object)
        .filter_map(|&id| {
            let node = scene.get(id)?;
            Some((scene.world_matrix_at(id, f64::from(start)), node.data.clone()))
        })
        .collect()
}

/// What above `id` can move or hide it: animation, visibility, enabled.
fn above(scene: &Scene, id: NodeId) -> Vec<(Vec<Track>, bool, bool)> {
    let mut found = Vec::new();
    let mut at = scene.get(id).and_then(grid4d_scene::Node::parent);
    while let Some(current) = at {
        let Some(node) = scene.get(current) else { break };
        found.push((node.data.tracks.clone(), node.data.visible_in_editor, node.data.enabled));
        at = node.parent();
    }
    found
}

#[cfg(test)]
mod tests {
    use glam::Vec3;
    use grid4d_eval::EvaluatedScene;
    use grid4d_scene::{Body, Node, ObjectKind, ObjectKindTag, SceneEdit};

    use super::*;

    fn falling_cube() -> (Document, NodeId, NodeId) {
        let mut doc = Document::new("fall");
        let mut floor = NodeData::new("Floor", ObjectKindTag::Plane.default_kind());
        floor.tags.push(Tag::Dynamics(Body::collider()));
        doc.scene.insert(Node::from_data(floor));
        let mut cube = NodeData::new(
            "Cube",
            ObjectKind::Cube {
                size: Vec3::splat(100.0),
                segments: [1, 1, 1],
            },
        );
        cube.transform.position.y = 400.0;
        cube.tags.push(Tag::Dynamics(Body::rigid()));
        let cube = doc.scene.insert(Node::from_data(cube));
        let other = doc
            .scene
            .insert(Node::new("Other", ObjectKindTag::Sphere.default_kind()));
        (doc, cube, other)
    }

    fn fill(sim: &mut Sim, doc: &Document, frame: i32) {
        while sim.advance(doc, frame) {}
    }

    #[test]
    fn the_cache_fills_as_frames_are_visited_and_moves_the_viewport() {
        let (doc, cube, _) = falling_cube();
        let mut sim = Sim::default();
        fill(&mut sim, &doc, 20);
        assert_eq!(sim.cached(), Some((0, 20)));
        let mut overrides = Overrides::default();
        sim.overrides(20, &mut overrides);
        let mut eval = EvaluatedScene::new();
        eval.set_frame(20.0);
        eval.set_overrides(&overrides);
        eval.update(&doc.scene);
        let y = eval.nodes().iter().find(|n| n.id == cube).unwrap().world.w_axis.y;
        assert!(y < 300.0, "fallen: {y}");
        assert_eq!(
            doc.scene.get(cube).unwrap().data.transform.position.y,
            400.0,
            "the document is untouched"
        );
        // Going back reads the cache; nothing more is simulated.
        assert!(!sim.advance(&doc, 5));
        assert_eq!(sim.cached(), Some((0, 20)));
        sim.overrides(0, &mut overrides);
        assert_eq!(overrides.get(cube).unwrap().w_axis.y, 400.0, "the start state");
    }

    #[test]
    fn only_changes_that_alter_the_motion_drop_the_cache() {
        let (mut doc, cube, other) = falling_cube();
        let mut sim = Sim::default();
        fill(&mut sim, &doc, 15);
        // Renaming the document or a body, or moving an object that is not a
        // body, keeps it.
        doc.apply(SceneEdit::SetName("Shot".to_owned())).unwrap();
        doc.scene.get_mut(other).unwrap().data.transform.position.x = 300.0;
        "Box".clone_into(&mut doc.scene.get_mut(cube).unwrap().data.name);
        sim.sync(&doc);
        assert_eq!(sim.cached(), Some((0, 15)));
        // Moving the body, its tag or the settings drop it.
        doc.scene.get_mut(cube).unwrap().data.transform.position.y = 500.0;
        sim.sync(&doc);
        assert_eq!(sim.cached(), None);
        fill(&mut sim, &doc, 15);
        doc.simulation.gravity.y = -100.0;
        sim.sync(&doc);
        assert_eq!(sim.cached(), None);
    }

    #[test]
    fn baking_fills_the_range_esc_stops_it_and_clear_empties_it() {
        let (mut doc, _, _) = falling_cube();
        doc.time.set_range(0, 60);
        let mut sim = Sim::default();
        sim.bake(&doc);
        assert!(sim.baking().is_some());
        fill(&mut sim, &doc, 0);
        assert_eq!(sim.cached(), Some((0, 60)), "baked to the end");
        assert!(sim.baking().is_none() && !sim.stop_baking());
        sim.clear();
        assert_eq!(sim.cached(), None);
        // A render's copy simulates what the cache lacks: the same motion.
        fill(&mut sim, &doc, 10);
        let mut replay = sim.replay(&doc).unwrap();
        let (mut from_replay, mut from_sim) = (Overrides::default(), Overrides::default());
        replay.overrides(10, &mut from_replay);
        sim.overrides(10, &mut from_sim);
        assert_eq!(from_replay, from_sim);
        replay.overrides(40, &mut from_replay);
        fill(&mut sim, &doc, 40);
        sim.overrides(40, &mut from_sim);
        assert_eq!(from_replay, from_sim, "deterministic");
    }

    #[test]
    fn a_tagged_cloners_effectors_are_part_of_the_inputs() {
        use grid4d_geom::cloner::Cloner;
        let (mut doc, _, _) = falling_cube();
        let effector = doc
            .scene
            .insert(Node::new("Plain", ObjectKindTag::PlainEffector.default_kind()));
        let mut cloner = NodeData::new(
            "Cloner",
            ObjectKind::Cloner {
                cloner: Cloner::default(),
                object: None,
                effectors: vec![effector],
            },
        );
        cloner.tags.push(Tag::Dynamics(Body::rigid()));
        let cloner = doc.scene.insert(Node::from_data(cloner));
        doc.scene
            .insert_child(cloner, Node::new("Ball", ObjectKindTag::Sphere.default_kind()))
            .unwrap();
        let mut sim = Sim::default();
        fill(&mut sim, &doc, 5);
        assert!(sim.cached().is_some());
        doc.scene.get_mut(effector).unwrap().data.transform.position.x = 100.0;
        sim.sync(&doc);
        assert_eq!(sim.cached(), None, "moving the effector changes where the clones start");
    }

    #[test]
    fn saved_frames_reopen_ready_unless_the_document_disagrees() {
        let (mut doc, cube, _) = falling_cube();
        doc.time.set_range(0, 40);
        let mut sim = Sim::default();
        sim.bake(&doc);
        fill(&mut sim, &doc, 0);
        let saved = sim.to_save(&doc).unwrap();
        let bytes = grid4d_io::to_bytes_with_cache(&doc, Some(&saved)).unwrap();
        let (opened, cache) = grid4d_io::from_bytes_with_cache(&bytes).unwrap();
        let mut reopened = Sim::default();
        assert!(reopened.adopt(&opened, cache.clone().unwrap()));
        assert_eq!(reopened.cached(), Some((0, 40)), "all frames at once");
        assert!(!reopened.advance(&opened, 40), "nothing to simulate");
        let (mut a, mut b) = (Overrides::default(), Overrides::default());
        sim.overrides(33, &mut a);
        let opened_cube = opened.scene.roots()[1];
        reopened.overrides(33, &mut b);
        assert_eq!(a.get(cube), b.get(opened_cube));
        // Past the saved frames the simulation goes on as it would have.
        doc.time.set_range(0, 60);
        let mut longer = opened.clone();
        longer.time.set_range(0, 60);
        fill(&mut reopened, &longer, 50);
        fill(&mut sim, &doc, 50);
        sim.overrides(50, &mut a);
        reopened.overrides(50, &mut b);
        assert_eq!(a.get(cube), b.get(opened_cube));

        // A body moved behind the file's back: the saved frames are dropped.
        let mut moved = opened.clone();
        moved.scene.get_mut(opened_cube).unwrap().data.transform.position.y = 900.0;
        assert!(!Sim::default().adopt(&moved, cache.unwrap()));
    }

    #[test]
    fn the_memory_limit_stops_the_cache() {
        let (mut doc, _, _) = falling_cube();
        doc.time.set_range(0, 100_000);
        doc.simulation.memory_limit_mb = 1;
        let mut sim = Sim::default();
        sim.bake(&doc);
        fill(&mut sim, &doc, 0);
        assert!(sim.is_full());
        let (_, last) = sim.cached().unwrap();
        assert!(last > 1000 && last < 100_000, "stopped at about 1 MB: {last}");
        // Raising the limit keeps what is done (it changes no motion).
        doc.simulation.memory_limit_mb = 2;
        sim.sync(&doc);
        assert_eq!(sim.cached().unwrap().1, last);
    }
}
