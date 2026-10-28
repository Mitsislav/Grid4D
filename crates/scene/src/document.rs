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
        }
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
