//! Format version 1. Never change these types once released: add a `v2` module
//! and migrate instead.

use std::collections::HashMap;

use glam::Vec3;
use oc4d_scene::{Document, Node, NodeData, NodeId, ObjectKind, TimeSettings, Transform};
use serde::{Deserialize, Serialize};

use crate::{FORMAT_TAG, IoError};

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct File {
    format: String,
    version: u32,
    document: DocumentDto,
}

#[derive(Debug, Serialize, Deserialize)]
struct DocumentDto {
    name: String,
    time: TimeDto,
    objects: Vec<ObjectDto>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TimeDto {
    fps: u32,
    start: i32,
    end: i32,
    looping: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct ObjectDto {
    name: String,
    /// Index of the parent in `objects`, always smaller than this object's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent: Option<usize>,
    kind: KindDto,
    position: [f32; 3],
    /// Heading, pitch, bank in radians.
    rotation_hpb: [f32; 3],
    scale: [f32; 3],
    /// Linear RGB.
    display_color: [f32; 3],
    visible_in_editor: bool,
    visible_in_render: bool,
    enabled: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum KindDto {
    Null,
    Cube { size: [f32; 3] },
    Sphere { radius: f32 },
    Cylinder { radius: f32, height: f32 },
    Plane { width: f32, depth: f32 },
    Camera { fov_y_deg: f32 },
    Light { color: [f32; 3], intensity: f32 },
}

impl File {
    pub(crate) fn parse(text: &str) -> Result<Self, IoError> {
        Ok(serde_json::from_str(text)?)
    }

    pub(crate) fn from_document(doc: &Document) -> Result<Self, IoError> {
        let mut index_of: HashMap<NodeId, usize> = HashMap::with_capacity(doc.scene.len());
        let mut objects = Vec::with_capacity(doc.scene.len());
        // Pre-order: a parent is always written (and indexed) before its children.
        doc.scene.walk(&mut Vec::new(), |id, node, _, _| {
            index_of.insert(id, objects.len());
            let parent = node.parent().and_then(|p| index_of.get(&p).copied());
            objects.push(ObjectDto::new(&node.data, parent));
            true
        });
        for object in &objects {
            object.check_finite()?;
        }
        let settings = doc.time.settings();
        Ok(Self {
            format: FORMAT_TAG.to_owned(),
            version: 1,
            document: DocumentDto {
                name: doc.name.clone(),
                time: TimeDto {
                    fps: settings.fps,
                    start: settings.start,
                    end: settings.end,
                    looping: settings.looping,
                },
                objects,
            },
        })
    }

    pub(crate) fn into_document(self) -> Result<Document, IoError> {
        let DocumentDto { name, time, objects } = self.document;
        let mut doc = Document::new(name);
        // Validated (fps and range clamped) exactly like an edit in the UI.
        doc.time.set_settings(TimeSettings {
            fps: time.fps,
            start: time.start,
            end: time.end,
            looping: time.looping,
        });

        let mut ids: Vec<NodeId> = Vec::with_capacity(objects.len());
        for (index, object) in objects.into_iter().enumerate() {
            object.check_finite()?;
            let parent = match object.parent {
                None => None,
                // `ids` holds exactly the objects before this one.
                Some(p) => Some(*ids.get(p).ok_or_else(|| {
                    IoError::Invalid(format!(
                        "object {index} (\"{}\") names object {p} as its parent, which does not come before it",
                        object.name
                    ))
                })?),
            };
            let id = doc
                .scene
                .insert_at(parent, usize::MAX, Node::from_data(object.into_data()))
                .map_err(|err| IoError::Invalid(err.to_string()))?;
            ids.push(id);
        }
        Ok(doc)
    }
}

impl ObjectDto {
    fn new(data: &NodeData, parent: Option<usize>) -> Self {
        let t = &data.transform;
        Self {
            name: data.name.clone(),
            parent,
            kind: KindDto::from(&data.kind),
            position: t.position.to_array(),
            rotation_hpb: t.rotation.to_array(),
            scale: t.scale.to_array(),
            display_color: data.display_color,
            visible_in_editor: data.visible_in_editor,
            visible_in_render: data.visible_in_render,
            enabled: data.enabled,
        }
    }

    fn into_data(self) -> NodeData {
        NodeData {
            name: self.name,
            kind: self.kind.into(),
            transform: Transform {
                position: Vec3::from_array(self.position),
                rotation: Vec3::from_array(self.rotation_hpb),
                scale: Vec3::from_array(self.scale),
            },
            display_color: self.display_color,
            visible_in_editor: self.visible_in_editor,
            visible_in_render: self.visible_in_render,
            enabled: self.enabled,
        }
    }

    /// JSON has no NaN or infinity (serde_json would write `null`, which then
    /// fails to load), and huge literals overflow `f32` on load.
    fn check_finite(&self) -> Result<(), IoError> {
        let groups: [(&str, &[f32]); 5] = [
            ("position", &self.position),
            ("rotation", &self.rotation_hpb),
            ("scale", &self.scale),
            ("display colour", &self.display_color),
            ("parameters", &self.kind.parameters()),
        ];
        for (what, values) in groups {
            if values.iter().any(|v| !v.is_finite()) {
                return Err(IoError::Invalid(format!(
                    "object \"{}\" has a {what} value that is not a finite number",
                    self.name
                )));
            }
        }
        Ok(())
    }
}

impl KindDto {
    /// Every numeric parameter, for validation.
    fn parameters(&self) -> Vec<f32> {
        match *self {
            Self::Null => Vec::new(),
            Self::Cube { size } => size.to_vec(),
            Self::Sphere { radius } => vec![radius],
            Self::Cylinder { radius, height } => vec![radius, height],
            Self::Plane { width, depth } => vec![width, depth],
            Self::Camera { fov_y_deg } => vec![fov_y_deg],
            Self::Light { color, intensity } => vec![color[0], color[1], color[2], intensity],
        }
    }
}

impl From<&ObjectKind> for KindDto {
    fn from(kind: &ObjectKind) -> Self {
        match *kind {
            ObjectKind::Null => Self::Null,
            ObjectKind::Cube { size } => Self::Cube { size: size.to_array() },
            ObjectKind::Sphere { radius } => Self::Sphere { radius },
            ObjectKind::Cylinder { radius, height } => Self::Cylinder { radius, height },
            ObjectKind::Plane { width, depth } => Self::Plane { width, depth },
            ObjectKind::Camera { fov_y_deg } => Self::Camera { fov_y_deg },
            ObjectKind::Light { color, intensity } => Self::Light { color, intensity },
        }
    }
}

impl From<KindDto> for ObjectKind {
    fn from(kind: KindDto) -> Self {
        match kind {
            KindDto::Null => Self::Null,
            KindDto::Cube { size } => Self::Cube {
                size: Vec3::from_array(size),
            },
            KindDto::Sphere { radius } => Self::Sphere { radius },
            KindDto::Cylinder { radius, height } => Self::Cylinder { radius, height },
            KindDto::Plane { width, depth } => Self::Plane { width, depth },
            KindDto::Camera { fov_y_deg } => Self::Camera { fov_y_deg },
            KindDto::Light { color, intensity } => Self::Light { color, intensity },
        }
    }
}

#[cfg(test)]
mod tests {
    use oc4d_scene::ObjectKindTag;

    use crate::{from_str, to_string};

    use super::*;

    /// What a file must preserve: document name, time settings, and the
    /// hierarchy in pre-order (depth + data). `NodeId`s are not part of a file.
    fn snapshot(doc: &Document) -> (String, TimeSettings, Vec<(u32, NodeData)>) {
        let mut nodes = Vec::new();
        doc.scene.walk(&mut Vec::new(), |_, node, _, depth| {
            nodes.push((depth, node.data.clone()));
            true
        });
        (doc.name.clone(), doc.time.settings(), nodes)
    }

    fn round_trip(doc: &Document) -> Document {
        from_str(&to_string(doc).unwrap()).unwrap()
    }

    #[test]
    fn the_demo_scene_round_trips_exactly() {
        let mut doc = Document::with_demo_scene();
        doc.name = "Shot 010 — ünïcode".to_owned();
        doc.time.set_settings(TimeSettings {
            fps: 25,
            start: -12,
            end: 250,
            looping: false,
        });
        assert_eq!(snapshot(&round_trip(&doc)), snapshot(&doc));
    }

    #[test]
    fn every_kind_and_awkward_values_round_trip() {
        let mut doc = Document::new("kinds");
        let mut parent = None;
        for (i, tag) in ObjectKindTag::ALL.into_iter().enumerate() {
            let mut data = NodeData::new(format!("{} \"quoted\" {i}", tag.label()), tag.default_kind());
            data.transform = Transform {
                position: Vec3::new(0.1, -1e-7, 123_456.79),
                rotation: Vec3::new(std::f32::consts::PI, -0.3, 1.0e-3),
                scale: Vec3::new(1.0 / 3.0, 2.0, 0.001),
            };
            data.display_color = [0.1, 0.2, 0.3];
            data.visible_in_editor = i % 2 == 0;
            data.visible_in_render = i % 3 == 0;
            data.enabled = i % 4 != 0;
            let id = doc.scene.insert_at(parent, usize::MAX, Node::from_data(data)).unwrap();
            parent = Some(id);
        }
        assert_eq!(snapshot(&round_trip(&doc)), snapshot(&doc));
    }

    #[test]
    fn deep_and_wide_hierarchies_round_trip() {
        let mut doc = Document::new("deep");
        let mut parent = None;
        for depth in 0..500 {
            let id = doc
                .scene
                .insert_at(
                    parent,
                    usize::MAX,
                    Node::new(format!("level {depth}"), ObjectKind::Null),
                )
                .unwrap();
            for sibling in 0..3 {
                doc.scene
                    .insert_at(
                        Some(id),
                        0,
                        Node::new(format!("leaf {depth}.{sibling}"), ObjectKind::Null),
                    )
                    .unwrap();
            }
            parent = Some(id);
        }
        let loaded = round_trip(&doc);
        assert_eq!(loaded.scene.len(), 2000);
        assert_eq!(snapshot(&loaded), snapshot(&doc));
    }

    #[test]
    fn detached_nodes_are_not_saved() {
        let mut doc = Document::with_demo_scene();
        let group = doc.scene.iter().find(|(_, n)| n.data.name == "Group").unwrap().0;
        doc.scene.detach(group).unwrap();
        let loaded = round_trip(&doc);
        assert_eq!(loaded.scene.len(), doc.scene.len());
        assert!(loaded.scene.detached_roots().is_empty());
        assert_eq!(snapshot(&loaded), snapshot(&doc));
    }

    #[test]
    fn non_finite_values_refuse_to_save() {
        let mut doc = Document::with_demo_scene();
        let cube = doc.scene.roots()[0];
        doc.scene.get_mut(cube).unwrap().data.transform.scale.y = f32::NAN;
        let err = to_string(&doc).unwrap_err();
        assert!(matches!(err, IoError::Invalid(_)), "{err:?}");
        assert!(err.to_string().contains("\"Cube\""), "{err}");
    }

    #[test]
    fn malformed_files_give_clear_errors() {
        let ok = to_string(&Document::with_demo_scene()).unwrap();

        let err = from_str("{ \"format\": \"oc4d\", ").unwrap_err();
        assert!(matches!(err, IoError::Parse { line: 1, .. }), "{err:?}");

        for not_ours in ["{}", "{\"format\": \"gltf\", \"version\": 1}", "{\"format\": \"oc4d\"}"] {
            assert!(matches!(from_str(not_ours), Err(IoError::NotOc4d)), "{not_ours}");
        }

        let newer = ok.replacen("\"version\": 1", "\"version\": 7", 1);
        assert!(matches!(
            from_str(&newer),
            Err(IoError::UnsupportedVersion { found: 7, newest: 1 })
        ));

        let missing_field = ok.replacen("\"enabled\": true", "\"enabeld\": true", 1);
        assert!(matches!(from_str(&missing_field), Err(IoError::Parse { .. })));

        let unknown_kind = ok.replacen("\"type\": \"cube\"", "\"type\": \"teapot\"", 1);
        assert!(matches!(from_str(&unknown_kind), Err(IoError::Parse { .. })));

        let overflow = ok.replacen("\"radius\": 80.0", "\"radius\": 1e300", 1);
        assert!(matches!(from_str(&overflow), Err(IoError::Invalid(_))), "f32 overflow");
    }

    #[test]
    fn parents_must_come_first() {
        let text = r#"{ "format": "oc4d", "version": 1, "document": {
            "name": "x", "time": { "fps": 30, "start": 0, "end": 90, "looping": true },
            "objects": [
              { "name": "child", "parent": 1, "kind": { "type": "null" },
                "position": [0,0,0], "rotation_hpb": [0,0,0], "scale": [1,1,1], "display_color": [1,1,1],
                "visible_in_editor": true, "visible_in_render": true, "enabled": true },
              { "name": "parent", "kind": { "type": "null" },
                "position": [0,0,0], "rotation_hpb": [0,0,0], "scale": [1,1,1], "display_color": [1,1,1],
                "visible_in_editor": true, "visible_in_render": true, "enabled": true }
            ] } }"#;
        let err = from_str(text).unwrap_err();
        assert!(matches!(err, IoError::Invalid(_)), "{err:?}");
        assert!(err.to_string().contains("\"child\""), "{err}");
        // A self-reference is caught the same way, so cycles are impossible.
        assert!(matches!(
            from_str(&text.replacen("\"parent\": 1", "\"parent\": 0", 1)),
            Err(IoError::Invalid(_))
        ));
    }

    #[test]
    fn time_settings_are_validated_on_load() {
        let text = to_string(&Document::new("t"))
            .unwrap()
            .replacen("\"fps\": 30", "\"fps\": 0", 1);
        assert_eq!(from_str(&text).unwrap().time.fps(), 1);
    }
}
