use glam::Vec3;

use crate::{Node, ObjectKind, ObjectKindTag, RenderSettings, Scene, TimeState, Transform};

/// A complete Grid4D document: the scene graph plus document-level settings.
#[derive(Clone, Debug)]
pub struct Document {
    pub name: String,
    pub scene: Scene,
    pub time: TimeState,
    /// How the document is rendered.
    pub render: RenderSettings,
    /// How its rigid bodies are simulated.
    pub simulation: crate::SimulationSettings,
    /// Its takes; the document holds the active one's values.
    pub takes: crate::Takes,
}

impl Default for Document {
    fn default() -> Self {
        Self::new("Untitled 1")
    }
}

impl Document {
    /// An empty document.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            scene: Scene::new(),
            time: TimeState::default(),
            render: RenderSettings::default(),
            simulation: crate::SimulationSettings::default(),
            takes: crate::Takes::default(),
        }
    }

    /// Makes `to` the active take (`None`: Main): Main's values come back
    /// over the active take's (which already holds them: edits in a take are
    /// recorded as it goes), then `to`'s values are written with Main's kept
    /// beside them. Returns the take that was active, so switching back is
    /// the exact inverse. An unknown take counts as Main.
    pub fn switch_take(&mut self, to: Option<crate::TakeId>) -> Option<crate::TakeId> {
        use crate::take::{Override, set_override};
        let to = to.filter(|&t| self.takes.contains(t));
        let from = self.takes.active();
        if from == to {
            return from;
        }
        for (id, values) in std::mem::take(self.takes.main_mut()) {
            let Some(data) = self.scene.data_mut_any(id) else {
                continue;
            };
            for value in values {
                value.write(data);
            }
        }
        if let Some(main) = self.takes.main_render_mut().take() {
            self.render = main;
        }
        if let Some(to) = to
            && let Some(take) = self.takes.get(to).cloned()
        {
            let mut main = crate::ObjectOverrides::new();
            for (id, values) in &take.objects {
                let Some(data) = self.scene.data_mut_any(*id) else {
                    continue;
                };
                for value in values {
                    set_override(&mut main, *id, Override::of(value.group(), data));
                    value.write(data);
                }
            }
            *self.takes.main_mut() = main;
            *self.takes.main_render_mut() = take.render.map(|render| std::mem::replace(&mut self.render, render));
        }
        self.takes.set_active(to);
        from
    }

    /// Auto Take: while a take other than Main is active, what the edits whose
    /// inverses are `inverses` changed (object values, render settings)
    /// becomes that take's overrides, Main's values kept beside them. Returns
    /// the inverse of that change of the takes, if there was one.
    pub fn record_in_take(&mut self, inverses: &[crate::SceneEdit]) -> Option<crate::SceneEdit> {
        use crate::take::{Override, OverrideGroup, get_override, overridable, remove_override, set_override};
        let active = self.takes.active()?;
        let mut takes = self.takes.clone();
        for inverse in inverses {
            match inverse {
                crate::SceneEdit::SetData { id, data: before } => {
                    let Some(node) = self.scene.get(*id) else { continue };
                    let now = &node.data;
                    // Made editable (another kind of object): its parameters
                    // are no take's any more.
                    if before.kind.tag() != now.kind.tag() {
                        remove_override(takes.main_mut(), *id, OverrideGroup::Parameters);
                        if let Some(take) = takes.get_mut(active) {
                            remove_override(&mut take.objects, *id, OverrideGroup::Parameters);
                        }
                    }
                    for group in OverrideGroup::ALL {
                        if !overridable(group, &now.kind) || !overridable(group, &before.kind) {
                            continue;
                        }
                        let (old, new) = (Override::of(group, before), Override::of(group, now));
                        if old == new {
                            continue;
                        }
                        if get_override(takes.main(), *id, group).is_none() {
                            set_override(takes.main_mut(), *id, old);
                        }
                        if let Some(take) = takes.get_mut(active) {
                            set_override(&mut take.objects, *id, new);
                        }
                    }
                }
                crate::SceneEdit::Move {
                    id, transform: before, ..
                } => {
                    let Some(now) = self.scene.get(*id).map(|n| n.data.transform) else {
                        continue;
                    };
                    if *before != now {
                        if get_override(takes.main(), *id, OverrideGroup::Transform).is_none() {
                            set_override(takes.main_mut(), *id, Override::Transform(*before));
                        }
                        if let Some(take) = takes.get_mut(active) {
                            set_override(&mut take.objects, *id, Override::Transform(now));
                        }
                    }
                }
                crate::SceneEdit::SetRender(before) if *before != self.render => {
                    if takes.main_render().is_none() {
                        *takes.main_render_mut() = Some(before.clone());
                    }
                    if let Some(take) = takes.get_mut(active) {
                        take.render = Some(self.render.clone());
                    }
                }
                _ => {}
            }
        }
        if takes == self.takes {
            return None;
        }
        Some(crate::SceneEdit::SetTakes(std::mem::replace(&mut self.takes, takes)))
    }

    /// A small starter scene that exercises hierarchy, transforms and every
    /// renderable object type.
    #[must_use]
    pub fn with_demo_scene() -> Self {
        let mut doc = Self::new("Untitled 1");
        let scene = &mut doc.scene;

        scene.insert(
            Node::new("Cube", ObjectKindTag::Cube.default_kind())
                .with_position(Vec3::new(0.0, 100.0, 0.0))
                .with_color([0.62, 0.62, 0.64]),
        );
        scene.insert(
            Node::new(
                "Sphere",
                ObjectKind::Sphere {
                    radius: 80.0,
                    segments: 24,
                },
            )
            .with_position(Vec3::new(-320.0, 80.0, 140.0))
            .with_color([0.18, 0.36, 0.62]),
        );

        let group = scene.insert(Node::new("Group", ObjectKind::Null).with_position(Vec3::new(320.0, 0.0, -80.0)));
        // `insert_child` only fails for unknown parents; `group` was just inserted.
        let _ = scene.insert_child(
            group,
            Node::new("Cylinder", ObjectKindTag::Cylinder.default_kind())
                .with_position(Vec3::new(0.0, 100.0, 0.0))
                .with_color([0.78, 0.47, 0.16]),
        );
        let _ = scene.insert_child(
            group,
            Node::new(
                "Box",
                ObjectKind::Cube {
                    size: Vec3::splat(80.0),
                    segments: [1, 1, 1],
                },
            )
            .with_transform(
                Transform::from_position(Vec3::new(130.0, 40.0, 110.0)).with_rotation_deg(Vec3::new(30.0, 0.0, 0.0)),
            )
            .with_color([0.48, 0.5, 0.53]),
        );

        scene.insert(
            Node::new("Light", ObjectKindTag::Light.default_kind())
                .with_position(Vec3::new(-400.0, 650.0, 450.0))
                .with_color([1.0, 0.92, 0.75]),
        );
        scene.insert(
            Node::new("Camera", ObjectKindTag::Camera.default_kind())
                .with_transform(
                    Transform::from_position(Vec3::new(0.0, 260.0, 950.0))
                        .with_rotation_deg(Vec3::new(0.0, -12.0, 0.0)),
                )
                .with_color([0.3, 0.3, 0.32]),
        );
        doc
    }
}
