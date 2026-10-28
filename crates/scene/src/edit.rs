//! Reversible document edits — the foundation of undo/redo.
//!
//! [`Document::apply`] performs a [`SceneEdit`] and returns the edit that undoes
//! it. Undo is "apply the inverse"; redo is "apply the inverse of the inverse".
//! The UI translates its `EditorCommand`s into edits and keeps the inverses in
//! its history; this crate only guarantees that every inverse is exact.

use crate::{
    Detached, Document, Materials, Node, NodeData, NodeId, RenderSettings, SceneError, TimeSettings, Transform,
};

/// One reversible change to a [`Document`].
///
/// Edits are all-or-nothing: when [`Document::apply`] returns an error the
/// document is unchanged. Node identity is stable across undo/redo, because
/// deletions are soft (see [`Scene::detach`](crate::Scene::detach)): an edit
/// recorded earlier keeps referring to the same `NodeId`.
#[derive(Clone, Debug, PartialEq)]
pub enum SceneEdit {
    /// Creates a node at `index` (clamped) among the children of `parent`
    /// (`None` = the roots). Its inverse detaches the new node, so redo
    /// re-attaches that very node instead of creating another one.
    Insert {
        parent: Option<NodeId>,
        index: usize,
        data: NodeData,
    },
    /// Soft-deletes `root` and its subtree.
    Detach { root: NodeId },
    /// Re-attaches a detached subtree at `index` (clamped) among the children of
    /// `parent` (`None` = the roots).
    Attach {
        root: NodeId,
        parent: Option<NodeId>,
        index: usize,
    },
    /// Moves `id` (with its subtree) to `index` (clamped, counted without `id`
    /// itself) among the children of `parent`, and sets its local `transform`.
    /// Use [`Scene::transform_keeping_world`](crate::Scene::transform_keeping_world)
    /// for a world-preserving move.
    Move {
        id: NodeId,
        parent: Option<NodeId>,
        index: usize,
        transform: Transform,
    },
    /// Replaces the payload of `id` — every attribute edit.
    SetData { id: NodeId, data: NodeData },
    /// Replaces the document's frame rate, range and looping.
    SetTime(TimeSettings),
    /// Renames the document.
    SetName(String),
    /// Replaces the document's materials (the whole table, ids included: tags
    /// keep pointing at the same materials across undo and redo).
    SetMaterials(Materials),
    /// Replaces the document's render settings (validated).
    SetRender(RenderSettings),
}

impl SceneEdit {
    /// The node this edit is about. `None` for document-level edits and for
    /// [`SceneEdit::Insert`], whose node does not exist yet: read the new id from
    /// the inverse that [`Document::apply`] returns.
    #[must_use]
    pub fn node(&self) -> Option<NodeId> {
        match *self {
            Self::Detach { root } | Self::Attach { root, .. } => Some(root),
            Self::Move { id, .. } | Self::SetData { id, .. } => Some(id),
            Self::Insert { .. } | Self::SetTime(_) | Self::SetName(_) | Self::SetMaterials(_) | Self::SetRender(_) => {
                None
            }
        }
    }
}

impl Document {
    /// Applies `edit` and returns the edit that exactly undoes it.
    ///
    /// On error nothing has changed. The inverse records the *effective* change
    /// (indices after clamping, time settings after validation), so applying the
    /// inverse of the inverse redoes the edit exactly.
    pub fn apply(&mut self, edit: SceneEdit) -> Result<SceneEdit, SceneError> {
        match edit {
            SceneEdit::Insert { parent, index, data } => {
                let id = self.scene.insert_at(parent, index, Node::from_data(data))?;
                Ok(SceneEdit::Detach { root: id })
            }
            SceneEdit::Detach { root } => {
                let Detached { root, parent, index } = self.scene.detach(root)?;
                Ok(SceneEdit::Attach { root, parent, index })
            }
            SceneEdit::Attach { root, parent, index } => {
                self.scene.attach(root, parent, index)?;
                Ok(SceneEdit::Detach { root })
            }
            SceneEdit::Move {
                id,
                parent,
                index,
                transform,
            } => {
                let node = self.scene.get(id).ok_or(SceneError::NodeNotFound)?;
                let (old_parent, old_transform) = (node.parent(), node.data.transform);
                let old_index = self.scene.index_in_parent(id).ok_or(SceneError::NodeNotFound)?;
                self.scene.reparent_at(id, parent, index, false)?;
                if let Some(node) = self.scene.get_mut(id) {
                    node.data.transform = transform;
                }
                Ok(SceneEdit::Move {
                    id,
                    parent: old_parent,
                    index: old_index,
                    transform: old_transform,
                })
            }
            SceneEdit::SetData { id, data } => {
                let node = self.scene.get_mut(id).ok_or(SceneError::NodeNotFound)?;
                let old = std::mem::replace(&mut node.data, data);
                Ok(SceneEdit::SetData { id, data: old })
            }
            SceneEdit::SetTime(settings) => {
                let old = self.time.settings();
                self.time.set_settings(settings);
                Ok(SceneEdit::SetTime(old))
            }
            SceneEdit::SetName(name) => Ok(SceneEdit::SetName(std::mem::replace(&mut self.name, name))),
            SceneEdit::SetMaterials(materials) => Ok(SceneEdit::SetMaterials(std::mem::replace(
                self.scene.materials_mut(),
                materials,
            ))),
            SceneEdit::SetRender(settings) => Ok(SceneEdit::SetRender(std::mem::replace(
                &mut self.render,
                settings.validated(),
            ))),
        }
    }

    /// Applies `edits` in order, all or nothing.
    ///
    /// Returns the inverses **in undo order** (newest first), so passing them
    /// straight back to `apply_all` undoes the whole batch. If any edit fails,
    /// the ones already applied are rolled back and the document is unchanged.
    pub fn apply_all(&mut self, edits: impl IntoIterator<Item = SceneEdit>) -> Result<Vec<SceneEdit>, SceneError> {
        let mut inverses = Vec::new();
        for edit in edits {
            match self.apply(edit) {
                Ok(inverse) => inverses.push(inverse),
                Err(err) => {
                    while let Some(inverse) = inverses.pop() {
                        // Inverses of edits just applied, newest first, always apply.
                        let rolled_back = self.apply(inverse);
                        debug_assert!(rolled_back.is_ok(), "rollback failed: {rolled_back:?}");
                    }
                    return Err(err);
                }
            }
        }
        inverses.reverse();
        Ok(inverses)
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::{ObjectKind, ObjectKindTag, test_util::Rng};

    /// Everything a user can observe: names, settings and the attached hierarchy
    /// (pre-order with parent and depth fully determines the tree and its order).
    #[derive(Debug, PartialEq)]
    struct Snapshot {
        name: String,
        time: TimeSettings,
        materials: Materials,
        render: RenderSettings,
        nodes: Vec<(NodeId, Option<NodeId>, u32, NodeData)>,
    }

    fn snapshot(doc: &Document) -> Snapshot {
        let mut nodes = Vec::new();
        doc.scene.walk(&mut Vec::new(), |id, node, _, depth| {
            nodes.push((id, node.parent(), depth, node.data.clone()));
            true
        });
        assert_eq!(nodes.len(), doc.scene.len());
        Snapshot {
            name: doc.name.clone(),
            time: doc.time.settings(),
            materials: doc.scene.materials().clone(),
            render: doc.render.clone(),
            nodes,
        }
    }

    fn id_of(doc: &Document, name: &str) -> NodeId {
        doc.scene
            .iter()
            .find(|(_, n)| n.data.name == name)
            .map(|(id, _)| id)
            .unwrap()
    }

    /// Applies `edit`, undoes it, redoes it, and checks every state on the way.
    fn assert_round_trip(doc: &mut Document, edit: SceneEdit) -> SceneEdit {
        let before = snapshot(doc);
        let inverse = doc.apply(edit).unwrap();
        let after = snapshot(doc);
        let redo = doc.apply(inverse.clone()).unwrap();
        assert_eq!(snapshot(doc), before, "undo must restore the previous state");
        let inverse_again = doc.apply(redo).unwrap();
        assert_eq!(snapshot(doc), after, "redo must restore the edited state");
        assert_eq!(inverse_again, inverse, "redo yields the same inverse");
        inverse
    }

    #[test]
    fn insert_undo_redo_reuses_the_same_node() {
        let mut doc = Document::with_demo_scene();
        let group = id_of(&doc, "Group");
        let inverse = assert_round_trip(
            &mut doc,
            SceneEdit::Insert {
                parent: Some(group),
                index: 1,
                data: NodeData::new("Torus", ObjectKind::Null),
            },
        );
        let SceneEdit::Detach { root } = inverse else {
            panic!("insert must be undone by a detach, got {inverse:?}");
        };
        assert_eq!(inverse.node(), Some(root));
        assert_eq!(doc.scene.children(group)[1], root, "inserted at the requested index");
        assert_eq!(doc.scene.get(root).unwrap().data.name, "Torus");
    }

    #[test]
    fn delete_undo_redo_keeps_ids_and_position() {
        let mut doc = Document::with_demo_scene();
        let (group, cylinder) = (id_of(&doc, "Group"), id_of(&doc, "Cylinder"));
        let roots_before = doc.scene.roots().to_vec();
        let inverse = assert_round_trip(&mut doc, SceneEdit::Detach { root: group });
        assert_eq!(
            inverse,
            SceneEdit::Attach {
                root: group,
                parent: None,
                index: 2
            }
        );
        // After the round trip the group is deleted again; undo it once more.
        doc.apply(inverse).unwrap();
        assert_eq!(doc.scene.roots(), roots_before.as_slice());
        assert_eq!(doc.scene.get(cylinder).unwrap().parent(), Some(group));
    }

    #[test]
    fn move_round_trips_parent_index_and_transform() {
        let mut doc = Document::with_demo_scene();
        let (cube, group, box_) = (id_of(&doc, "Cube"), id_of(&doc, "Group"), id_of(&doc, "Box"));

        // Into another parent, world-preserving.
        let world_before = doc.scene.world_matrix(cube);
        let transform = doc.scene.transform_keeping_world(cube, Some(group)).unwrap();
        assert_round_trip(
            &mut doc,
            SceneEdit::Move {
                id: cube,
                parent: Some(group),
                index: 0,
                transform,
            },
        );
        assert_eq!(doc.scene.children(group)[0], cube);
        assert!(doc.scene.world_matrix(cube).abs_diff_eq(world_before, 1e-3));

        // Reorder within the same parent, with a new transform.
        assert_round_trip(
            &mut doc,
            SceneEdit::Move {
                id: box_,
                parent: Some(group),
                index: 0,
                transform: Transform::from_position(Vec3::splat(7.0)),
            },
        );
        assert_eq!(doc.scene.index_in_parent(box_), Some(0));
    }

    #[test]
    fn data_time_and_name_round_trip() {
        let mut doc = Document::with_demo_scene();
        let sphere = id_of(&doc, "Sphere");
        let mut data = doc.scene.get(sphere).unwrap().data.clone();
        data.name = "Ball".to_owned();
        data.kind = ObjectKind::Sphere {
            radius: 12.5,
            segments: 8,
        };
        data.visible_in_editor = false;
        data.transform.rotation.x = 1.25;
        assert_round_trip(&mut doc, SceneEdit::SetData { id: sphere, data });

        assert_round_trip(
            &mut doc,
            SceneEdit::SetTime(TimeSettings {
                fps: 25,
                start: 10,
                end: 250,
                looping: false,
            }),
        );
        assert_round_trip(&mut doc, SceneEdit::SetName("Shot 010".to_owned()));
        assert_round_trip(
            &mut doc,
            SceneEdit::SetRender(RenderSettings {
                width: 1920,
                height: 1080,
                samples: 64,
                output: "shot".to_owned(),
                ..RenderSettings::default()
            }),
        );
    }

    #[test]
    fn render_settings_are_validated() {
        let mut doc = Document::new("t");
        let undo = doc
            .apply(SceneEdit::SetRender(RenderSettings {
                width: 0,
                samples: 0,
                ..RenderSettings::default()
            }))
            .unwrap();
        assert_eq!((doc.render.width, doc.render.samples), (1, 1));
        assert_eq!(undo, SceneEdit::SetRender(RenderSettings::default()));
    }

    #[test]
    fn clamped_edits_record_the_effective_change() {
        let mut doc = Document::new("t");
        let inverse = doc
            .apply(SceneEdit::Insert {
                parent: None,
                index: 99,
                data: NodeData::new("a", ObjectKind::Null),
            })
            .unwrap();
        // Undoing the insert records where the node really went: index 0, not 99.
        let redo = doc.apply(inverse).unwrap();
        assert!(matches!(redo, SceneEdit::Attach { index: 0, .. }), "{redo:?}");

        let undo_time = doc
            .apply(SceneEdit::SetTime(TimeSettings {
                fps: 0,
                start: 5,
                end: 0,
                looping: true,
            }))
            .unwrap();
        let redo_time = doc.apply(undo_time).unwrap();
        assert_eq!(
            redo_time,
            SceneEdit::SetTime(TimeSettings {
                fps: 1,
                start: 5,
                end: 6,
                looping: true
            })
        );
    }

    #[test]
    fn failed_edits_change_nothing() {
        let mut doc = Document::with_demo_scene();
        let (group, cylinder, light) = (id_of(&doc, "Group"), id_of(&doc, "Cylinder"), id_of(&doc, "Light"));
        doc.apply(SceneEdit::Detach { root: light }).unwrap();
        let before = snapshot(&doc);

        let failures = [
            (
                SceneEdit::Move {
                    id: group,
                    parent: Some(cylinder),
                    index: 0,
                    transform: Transform::IDENTITY,
                },
                SceneError::WouldCreateCycle,
            ),
            (
                SceneEdit::SetData {
                    id: light,
                    data: NodeData::new("x", ObjectKind::Null),
                },
                SceneError::NodeNotFound,
            ),
            (
                SceneEdit::Attach {
                    root: group,
                    parent: None,
                    index: 0,
                },
                SceneError::NotDetached,
            ),
            (
                SceneEdit::Insert {
                    parent: Some(light),
                    index: 0,
                    data: NodeData::new("x", ObjectKind::Null),
                },
                SceneError::NodeNotFound,
            ),
            (SceneEdit::Detach { root: light }, SceneError::NodeNotFound),
        ];
        for (edit, expected) in failures {
            assert_eq!(doc.apply(edit.clone()), Err(expected), "{edit:?}");
            assert_eq!(snapshot(&doc), before, "{edit:?} must leave the document unchanged");
        }
    }

    #[test]
    fn apply_all_is_atomic_and_returns_undo_order() {
        let mut doc = Document::with_demo_scene();
        let (cube, sphere, group) = (id_of(&doc, "Cube"), id_of(&doc, "Sphere"), id_of(&doc, "Group"));
        let before = snapshot(&doc);

        let failing = vec![
            SceneEdit::SetName("changed".to_owned()),
            SceneEdit::Detach { root: cube },
            SceneEdit::Move {
                id: sphere,
                parent: Some(group),
                index: 0,
                transform: Transform::IDENTITY,
            },
            SceneEdit::Detach { root: cube }, // already detached by this batch
        ];
        assert_eq!(doc.apply_all(failing), Err(SceneError::NodeNotFound));
        assert_eq!(snapshot(&doc), before, "a failed batch is rolled back completely");

        let batch = vec![
            SceneEdit::SetName("changed".to_owned()),
            SceneEdit::Detach { root: cube },
            SceneEdit::Move {
                id: sphere,
                parent: Some(group),
                index: 0,
                transform: Transform::IDENTITY,
            },
        ];
        let undo = doc.apply_all(batch).unwrap();
        assert!(matches!(undo[0], SceneEdit::Move { .. }), "newest first: {undo:?}");
        let after = snapshot(&doc);
        let redo = doc.apply_all(undo).unwrap();
        assert_eq!(snapshot(&doc), before);
        doc.apply_all(redo).unwrap();
        assert_eq!(snapshot(&doc), after);
    }

    fn random_data(rng: &mut Rng) -> NodeData {
        let tag = ObjectKindTag::ALL[rng.below(ObjectKindTag::ALL.len())];
        let mut data = NodeData::new(format!("n{}", rng.below(1000)), tag.default_kind());
        data.transform = Transform {
            position: Vec3::new(
                rng.range_f32(-500.0, 500.0),
                rng.range_f32(-500.0, 500.0),
                rng.range_f32(-500.0, 500.0),
            ),
            rotation: Vec3::new(rng.range_f32(-3.0, 3.0), rng.range_f32(-1.5, 1.5), 0.0),
            scale: Vec3::splat(rng.range_f32(0.1, 4.0)),
        };
        data.visible_in_editor = !rng.chance(5);
        data.enabled = !rng.chance(5);
        data
    }

    fn random_edit(rng: &mut Rng, doc: &Document) -> SceneEdit {
        let attached: Vec<NodeId> = doc.scene.iter().map(|(id, _)| id).collect();
        let parent = if rng.chance(4) { None } else { rng.pick(&attached) };
        let index = rng.below(5);
        let target = rng.pick(&attached);
        match (rng.below(10), target) {
            (2, Some(root)) => SceneEdit::Detach { root },
            (3, _) => match rng.pick(doc.scene.detached_roots()) {
                Some(root) => SceneEdit::Attach { root, parent, index },
                None => SceneEdit::SetName(format!("doc {}", rng.below(100))),
            },
            (4, Some(id)) => SceneEdit::Move {
                id,
                parent,
                index,
                transform: random_data(rng).transform,
            },
            (5, Some(id)) => SceneEdit::SetData {
                id,
                data: random_data(rng),
            },
            (6, _) => {
                let start = i32::try_from(rng.below(100)).unwrap() - 50;
                SceneEdit::SetTime(TimeSettings {
                    fps: u32::try_from(rng.below(260)).unwrap(),
                    start,
                    end: start + i32::try_from(rng.below(300)).unwrap() - 20,
                    looping: rng.chance(2),
                })
            }
            (7, _) => SceneEdit::SetName(format!("doc {}", rng.below(100))),
            (8, _) => {
                // Add a material or remove one (the table is replaced whole).
                let mut materials = doc.scene.materials().clone();
                match rng.pick(materials.ids()) {
                    Some(id) if rng.chance(2) => {
                        materials.remove(id);
                    }
                    _ => {
                        let name = materials.unique_name("Mat");
                        materials.insert(crate::Material::new(name));
                    }
                }
                SceneEdit::SetMaterials(materials)
            }
            (9, _) if rng.chance(2) => SceneEdit::SetRender(RenderSettings {
                width: u32::try_from(rng.below(20_000)).unwrap(),
                samples: u32::try_from(rng.below(5000)).unwrap(),
                frames: match rng.below(3) {
                    0 => crate::FrameRange::Current,
                    1 => crate::FrameRange::All,
                    _ => crate::FrameRange::Manual {
                        from: i32::try_from(rng.below(50)).unwrap(),
                        to: i32::try_from(rng.below(50)).unwrap(),
                    },
                },
                auto_light: rng.chance(2),
                output: format!("render {}", rng.below(10)),
                format: if rng.chance(2) {
                    crate::ImageFormat::Exr
                } else {
                    crate::ImageFormat::Png
                },
                naming: crate::FrameNaming::ALL[rng.below(3)],
                ..RenderSettings::default()
            }),
            _ => SceneEdit::Insert {
                parent,
                index,
                data: random_data(rng),
            },
        }
    }

    /// The core undo/redo guarantee: for any sequence of edits, undoing all of
    /// them step by step walks back through every intermediate state to the
    /// original document, and redoing walks forward again — with the same ids.
    #[test]
    fn random_edit_sequences_undo_and_redo_exactly() {
        for seed in 1..=12u64 {
            let mut rng = Rng::new(seed);
            let mut doc = Document::with_demo_scene();
            let mut states = vec![snapshot(&doc)];
            let mut undo_stack = Vec::new();

            for _ in 0..250 {
                let edit = random_edit(&mut rng, &doc);
                match doc.apply(edit.clone()) {
                    Ok(inverse) => {
                        undo_stack.push(inverse);
                        states.push(snapshot(&doc));
                    }
                    Err(err) => {
                        // Only cycles can be generated by mistake; nothing may change.
                        assert_eq!(err, SceneError::WouldCreateCycle, "seed {seed}: {edit:?}");
                        assert_eq!(&snapshot(&doc), states.last().unwrap(), "seed {seed}: {edit:?}");
                    }
                }
            }

            assert!(undo_stack.len() > 100, "seed {seed}: too few successful edits");

            // Undo everything, checking each intermediate state.
            let mut redo_stack = Vec::new();
            for k in (0..undo_stack.len()).rev() {
                let inverse = undo_stack.pop().unwrap();
                redo_stack.push(doc.apply(inverse).unwrap());
                assert_eq!(snapshot(&doc), states[k], "seed {seed}: undo to state {k}");
            }

            // Redo everything, checking each intermediate state again.
            for (k, expected) in states.iter().enumerate().skip(1) {
                let redo = redo_stack.pop().unwrap();
                undo_stack.push(doc.apply(redo).unwrap());
                assert_eq!(&snapshot(&doc), expected, "seed {seed}: redo to state {k}");
            }
            assert!(redo_stack.is_empty());

            // And the redone history undoes cleanly once more.
            let undo_all: Vec<SceneEdit> = undo_stack.into_iter().rev().collect();
            doc.apply_all(undo_all).unwrap();
            assert_eq!(snapshot(&doc), states[0], "seed {seed}: undo after redo");
        }
    }
}
