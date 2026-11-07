//! Format version 3 (Grid4D 1.0), frozen: read only, through version 4.
//!
//! Version 4 kept version 3's layout and only adds to it, so a version 3
//! `document.json` is read by turning it into a version 4 one ([`upgrade`], a
//! change of the JSON itself) and then through version 4's types. This module
//! is the whole difference between the two, and it keeps version 3's meaning
//! fixed while version 4 grows:
//!
//! * a version 3 reader ignored fields it did not know, so every field version
//!   4 adds is **removed** here from a version 3 file ([`V4_DOCUMENT_FIELDS`])
//!   — a version 3 file that happens to carry one reads as Grid4D 1.0 read it;
//! * where version 4 gives an old field a new form, the conversion is written
//!   here, from the version 3 form.
//!
//! The fixtures `demo_v3.g4d`, `demo_v3_initial.g4d` and `full_v3.g4d` (every
//! thing version 3 holds) pin what a version 3 file means; they are never
//! regenerated.

use serde_json::Value;

use crate::IoError;

pub(crate) const VERSION: u32 = 3;

/// Fields of `document` that version 4 adds: absent from version 3, and
/// dropped from a version 3 file (Grid4D 1.0 ignored them). Each step of
/// Phase 15 that adds one lists it here.
const V4_DOCUMENT_FIELDS: &[&str] = &[
    // Step 15.3: the document's textures (embedded ones' entries are never
    // read without it).
    "textures",
];

/// Fields of each material that version 4 adds (step 15.4: the channels
/// beyond colour, roughness, metallic and emission, and their textures).
const V4_MATERIAL_FIELDS: &[&str] = &[
    "specular",
    "normal_strength",
    "bump_height",
    "emission_strength",
    "opacity",
    "opacity_threshold",
    "transmission",
    "ior",
    "clearcoat",
    "clearcoat_roughness",
    "two_sided",
    "maps",
];

/// Fields of each Material tag that version 4 adds (step 15.4: the
/// projection), on objects and in takes' overrides.
const V4_MATERIAL_TAG_FIELDS: &[&str] = &["projection", "offset", "tiling", "rotation", "placement", "camera"];

/// Removes [`V4_MATERIAL_TAG_FIELDS`] from every Material tag in `value`
/// (any `tags` list, wherever it is).
fn strip_material_tags(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(Value::Array(tags)) = map.get_mut("tags") {
                for tag in tags.iter_mut().filter_map(Value::as_object_mut) {
                    if tag.get("type").and_then(Value::as_str) == Some("material") {
                        for field in V4_MATERIAL_TAG_FIELDS {
                            tag.remove(*field);
                        }
                    }
                }
            }
            map.values_mut().for_each(strip_material_tags);
        }
        Value::Array(items) => items.iter_mut().for_each(strip_material_tags),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

/// A version 3 `document.json`, already parsed, as version 4 (see the module
/// documentation).
pub(crate) fn upgrade(mut json: Value) -> Result<Value, IoError> {
    let Some(file) = json.as_object_mut() else {
        return Err(IoError::NotADocument);
    };
    file.insert("version".to_owned(), Value::from(crate::v4::VERSION));
    if let Some(document) = file.get_mut("document").and_then(Value::as_object_mut) {
        for field in V4_DOCUMENT_FIELDS {
            document.remove(*field);
        }
        for material in document
            .get_mut("materials")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
            .filter_map(Value::as_object_mut)
        {
            for field in V4_MATERIAL_FIELDS {
                material.remove(*field);
            }
        }
        for field in ["objects", "takes"] {
            if let Some(value) = document.get_mut(field) {
                strip_material_tags(value);
            }
        }
    }
    Ok(json)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test code")]

    use serde_json::json;

    use super::*;

    #[test]
    fn upgrading_names_version_4_and_keeps_the_document() {
        let v3 = json!({
            "format": "grid4d",
            "version": 3,
            "document": { "name": "Shot", "objects": [] },
            "meshes": []
        });
        let v4 = upgrade(v3.clone()).unwrap();
        assert_eq!(v4["version"], 4);
        assert_eq!(v4["document"], v3["document"]);
        assert_eq!(v4["format"], "grid4d");
    }

    #[test]
    fn what_version_4_adds_is_dropped_from_a_version_3_file() {
        let v3 = json!({
            "version": 3,
            "document": {
                "name": "Shot",
                "textures": [{ "name": "Wood", "file": "wood.png" }],
                "materials": [{ "name": "Glass", "color": [1, 1, 1], "transmission": 1, "maps": {} }]
            }
        });
        let v4 = upgrade(v3).unwrap();
        assert_eq!(
            v4["document"],
            json!({ "name": "Shot", "materials": [{ "name": "Glass", "color": [1, 1, 1] }] })
        );
    }

    #[test]
    fn a_material_tags_projection_is_dropped_from_a_version_3_file() {
        let v3 = json!({
            "version": 3,
            "document": {
                "objects": [{ "name": "Cube", "tags": [
                    { "type": "material", "material": 0, "projection": "box", "tiling": [2, 2] },
                    { "type": "phong", "angle_limit": true, "angle": 1, "rotation": 5 }
                ] }],
                "takes": { "takes": [{ "overrides": [{ "object": 0, "tags": [
                    { "type": "material", "camera": 0, "placement": {} }
                ] }] }] }
            }
        });
        let v4 = upgrade(v3).unwrap();
        assert_eq!(
            v4["document"]["objects"][0]["tags"],
            json!([
                { "type": "material", "material": 0 },
                { "type": "phong", "angle_limit": true, "angle": 1, "rotation": 5 }
            ]),
            "only Material tags lose them"
        );
        assert_eq!(
            v4["document"]["takes"]["takes"][0]["overrides"][0]["tags"],
            json!([{ "type": "material" }])
        );
    }

    #[test]
    fn what_is_not_an_object_is_not_a_document() {
        for json in [json!([]), json!(3), json!("grid4d")] {
            assert!(matches!(upgrade(json), Err(IoError::NotADocument)));
        }
    }
}
