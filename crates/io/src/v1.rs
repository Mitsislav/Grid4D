//! Format version 1 — read-only. Files written by Grid4D before primitives had
//! segment counts; loading migrates them to the current scene types, filling in
//! each primitive's default segments. These types are frozen.

use glam::Vec3;
use grid4d_scene::{Document, NodeData, ObjectKind, TimeSettings, Transform};
use serde::Deserialize;

use crate::{
    IoError,
    common::{assemble, check_finite},
};

#[derive(Debug, Deserialize)]
pub(crate) struct File {
    document: DocumentDto,
}

#[derive(Debug, Deserialize)]
struct DocumentDto {
    name: String,
    time: TimeDto,
    objects: Vec<ObjectDto>,
}

#[derive(Debug, Deserialize)]
struct TimeDto {
    fps: u32,
    start: i32,
    end: i32,
    looping: bool,
}

#[derive(Debug, Deserialize)]
struct ObjectDto {
    name: String,
    #[serde(default)]
    parent: Option<usize>,
    kind: KindDto,
    position: [f32; 3],
    rotation_hpb: [f32; 3],
    scale: [f32; 3],
    display_color: [f32; 3],
    visible_in_editor: bool,
    visible_in_render: bool,
    enabled: bool,
}

#[derive(Debug, Deserialize)]
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

    pub(crate) fn into_document(self) -> Result<Document, IoError> {
        let DocumentDto { name, time, objects } = self.document;
        let objects = objects
            .into_iter()
            .map(|object| {
                object.check_finite()?;
                Ok((object.parent, object.into_data()))
            })
            .collect::<Result<Vec<_>, IoError>>()?;
        let time = TimeSettings {
            fps: time.fps,
            start: time.start,
            end: time.end,
            looping: time.looping,
        };
        assemble(name, time, objects)
    }
}

impl ObjectDto {
    fn into_data(self) -> NodeData {
        NodeData {
            name: self.name,
            kind: self.kind.migrate(),
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

    fn check_finite(&self) -> Result<(), IoError> {
        let parameters: Vec<f32> = match self.kind {
            KindDto::Null => Vec::new(),
            KindDto::Cube { size } => size.to_vec(),
            KindDto::Sphere { radius } => vec![radius],
            KindDto::Cylinder { radius, height } => vec![radius, height],
            KindDto::Plane { width, depth } => vec![width, depth],
            KindDto::Camera { fov_y_deg } => vec![fov_y_deg],
            KindDto::Light { color, intensity } => vec![color[0], color[1], color[2], intensity],
        };
        check_finite(
            &self.name,
            &[
                ("position", &self.position),
                ("rotation", &self.rotation_hpb),
                ("scale", &self.scale),
                ("display colour", &self.display_color),
                ("parameters", &parameters),
            ],
        )
    }
}

impl KindDto {
    /// Version 1 sizes with the segment counts version 2 introduced. Hard-coded
    /// on purpose: a migration must give the same result even if the editor's
    /// defaults change later.
    fn migrate(self) -> ObjectKind {
        match self {
            Self::Null => ObjectKind::Null,
            Self::Cube { size } => ObjectKind::Cube {
                size: Vec3::from_array(size),
                segments: [1, 1, 1],
            },
            Self::Sphere { radius } => ObjectKind::Sphere { radius, segments: 24 },
            Self::Cylinder { radius, height } => ObjectKind::Cylinder {
                radius,
                height,
                rotation_segments: 36,
                height_segments: 4,
            },
            Self::Plane { width, depth } => ObjectKind::Plane {
                width,
                depth,
                width_segments: 20,
                depth_segments: 20,
            },
            Self::Camera { fov_y_deg } => ObjectKind::Camera { fov_y_deg },
            Self::Light { color, intensity } => ObjectKind::Light { color, intensity },
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::from_json as from_str;

    use super::*;

    const ONE_OF_EACH: &str = r#"{ "format": "oc4d", "version": 1, "document": {
        "name": "old", "time": { "fps": 25, "start": 0, "end": 50, "looping": false },
        "objects": [
          { "name": "box", "kind": { "type": "cube", "size": [10, 20, 30] },
            "position": [1,2,3], "rotation_hpb": [0,0,0], "scale": [1,1,1], "display_color": [1,0,0],
            "visible_in_editor": true, "visible_in_render": true, "enabled": true },
          { "name": "ball", "parent": 0, "kind": { "type": "sphere", "radius": 5 },
            "position": [0,0,0], "rotation_hpb": [0,0,0], "scale": [1,1,1], "display_color": [1,1,1],
            "visible_in_editor": true, "visible_in_render": false, "enabled": true },
          { "name": "tube", "kind": { "type": "cylinder", "radius": 2, "height": 8 },
            "position": [0,0,0], "rotation_hpb": [0,0,0], "scale": [1,1,1], "display_color": [1,1,1],
            "visible_in_editor": true, "visible_in_render": true, "enabled": false },
          { "name": "floor", "kind": { "type": "plane", "width": 100, "depth": 50 },
            "position": [0,0,0], "rotation_hpb": [0,0,0], "scale": [1,1,1], "display_color": [1,1,1],
            "visible_in_editor": true, "visible_in_render": true, "enabled": true }
        ] } }"#;

    fn kinds(doc: &Document) -> Vec<(String, ObjectKind)> {
        let mut out = Vec::new();
        doc.scene.walk(&mut Vec::new(), |_, node, _, _| {
            out.push((node.data.name.clone(), node.data.kind.clone()));
            true
        });
        out
    }

    #[test]
    fn version_1_files_get_default_segments() {
        let doc = from_str(ONE_OF_EACH).unwrap();
        assert_eq!(doc.name, "old");
        assert_eq!(doc.time.fps(), 25);
        let migrated = kinds(&doc);
        let expected = [
            (
                "box",
                ObjectKind::Cube {
                    size: Vec3::new(10.0, 20.0, 30.0),
                    segments: [1, 1, 1],
                },
            ),
            (
                "ball",
                ObjectKind::Sphere {
                    radius: 5.0,
                    segments: 24,
                },
            ),
            (
                "tube",
                ObjectKind::Cylinder {
                    radius: 2.0,
                    height: 8.0,
                    rotation_segments: 36,
                    height_segments: 4,
                },
            ),
            (
                "floor",
                ObjectKind::Plane {
                    width: 100.0,
                    depth: 50.0,
                    width_segments: 20,
                    depth_segments: 20,
                },
            ),
        ];
        assert_eq!(migrated, expected.map(|(n, k)| (n.to_owned(), k)));
        let ball = doc.scene.iter().find(|(_, n)| n.data.name == "ball").unwrap().1;
        assert!(ball.parent().is_some(), "hierarchy survives the migration");
    }

    #[test]
    fn parents_must_come_first() {
        let text = ONE_OF_EACH.replacen("\"parent\": 0", "\"parent\": 3", 1);
        let err = from_str(&text).unwrap_err();
        assert!(matches!(err, IoError::Invalid(_)), "{err:?}");
        assert!(err.to_string().contains("\"ball\""), "{err}");
        // A self-reference is caught the same way, so cycles are impossible.
        let own = ONE_OF_EACH.replacen("\"parent\": 0", "\"parent\": 1", 1);
        assert!(matches!(from_str(&own), Err(IoError::Invalid(_))));
    }
}
