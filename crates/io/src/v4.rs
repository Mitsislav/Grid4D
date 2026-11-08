//! Format version 4 (Grid4D 1.1), frozen: read only, through version 5.
//!
//! Version 5 keeps version 4's layout and only adds to it, so a version 4
//! `document.json` is read by turning it into a version 5 one ([`upgrade`], a
//! change of the JSON itself) and then through version 5's types. This module
//! is the whole difference between the two, and it keeps version 4's meaning
//! fixed while version 5 grows:
//!
//! * a version 4 reader ignored fields it did not know, so every field version
//!   5 adds is **removed** here from a version 4 file ([`ADDED`]) — a version 4
//!   file that happens to carry one reads as Grid4D 1.1 read it;
//! * a version 4 reader refused types and values it did not know, so the ones
//!   version 5 adds ([`NEW_VALUES`]: a new kind of object, a new light type)
//!   stay errors in a version 4 file;
//! * where version 5 gives an old field a new form, the conversion is written
//!   here, from the version 4 form.
//!
//! Each step of Phase 16 that adds data lists it here and in
//! `docs/FILE_FORMAT.md`. The fixtures `demo_v4.g4d` and `full_v4.g4d` (every
//! thing version 4 holds) pin what a version 4 file means; they are never
//! regenerated. A version 3 file is first upgraded to version 4
//! ([`crate::v3::upgrade`]), then through here.

use serde_json::{Map, Value};

use crate::IoError;

pub(crate) const VERSION: u32 = 4;

/// Where in `document.json` version 5 adds fields.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(test), allow(dead_code, reason = "each Phase 16 step lists its additions"))]
enum Place {
    /// `document` itself.
    Document,
    /// The render settings: `document.render`, and a take's own and Main's
    /// (`settings` of `takes.takes[n].render` and `takes.main_render`).
    Render,
    /// Each of `document.materials`.
    Material,
    /// An object's kind of this `type` — on objects (`kind`) and in takes'
    /// overrides (`parameters`).
    Kind(&'static str),
    /// A tag of this `type`, in any `tags` list (objects', takes' overrides').
    Tag(&'static str),
}

/// Fields version 5 adds at a place: absent from version 4, and dropped from
/// a version 4 file (Grid4D 1.1 ignored them).
#[derive(Clone, Copy, Debug)]
struct Added {
    place: Place,
    fields: &'static [&'static str],
}

/// Everything version 5 adds as fields, by step.
const ADDED: &[Added] = &[
    // Step 16.2: the two light-transport switches.
    Added {
        place: Place::Render,
        fields: &["clamp_indirect", "emissive_lights"],
    },
];

/// A value version 5 adds to a field that held one of a closed set (a `type`
/// of object or tag, a light type): Grid4D 1.1 refused it, so it is an error
/// in a version 4 file still.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(test), allow(dead_code, reason = "each Phase 16 step lists its new values"))]
struct NewValue {
    field: &'static str,
    value: &'static str,
}

/// Every such value, by step. Empty in 16.1.
const NEW_VALUES: &[NewValue] = &[];

/// A version 4 `document.json`, already parsed, as version 5 (see the module
/// documentation).
pub(crate) fn upgrade(json: Value) -> Result<Value, IoError> {
    upgrade_with(json, ADDED, NEW_VALUES)
}

fn upgrade_with(mut json: Value, added: &[Added], new_values: &[NewValue]) -> Result<Value, IoError> {
    let Some(file) = json.as_object_mut() else {
        return Err(IoError::NotADocument);
    };
    if let Some(document) = file.get("document") {
        refuse_new_values(document, new_values)?;
    }
    file.insert("version".to_owned(), Value::from(crate::v5::VERSION));
    if let Some(document) = file.get_mut("document").and_then(Value::as_object_mut) {
        for addition in added {
            strip(document, *addition);
        }
    }
    Ok(json)
}

/// An error if `value` uses any of `new_values` anywhere.
fn refuse_new_values(value: &Value, new_values: &[NewValue]) -> Result<(), IoError> {
    match value {
        Value::Object(map) => {
            for new in new_values {
                if map.get(new.field).and_then(Value::as_str) == Some(new.value) {
                    return Err(IoError::Invalid(format!(
                        "\"{}\": \"{}\" is not part of format version 4",
                        new.field, new.value
                    )));
                }
            }
            map.values().try_for_each(|value| refuse_new_values(value, new_values))
        }
        Value::Array(items) => items.iter().try_for_each(|value| refuse_new_values(value, new_values)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Ok(()),
    }
}

/// Removes `addition`'s fields at its place in `document`.
fn strip(document: &mut Map<String, Value>, addition: Added) {
    let remove = |map: &mut Map<String, Value>| {
        for field in addition.fields {
            map.remove(*field);
        }
    };
    match addition.place {
        Place::Document => remove(document),
        Place::Render => {
            if let Some(render) = document.get_mut("render").and_then(Value::as_object_mut) {
                remove(render);
            }
            let Some(takes) = document.get_mut("takes").and_then(Value::as_object_mut) else {
                return;
            };
            if let Some(settings) = own_render(takes.get_mut("main_render")) {
                remove(settings);
            }
            for take in takes
                .get_mut("takes")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
                .filter_map(Value::as_object_mut)
            {
                if let Some(settings) = own_render(take.get_mut("render")) {
                    remove(settings);
                }
            }
        }
        Place::Material => {
            for material in document
                .get_mut("materials")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
                .filter_map(Value::as_object_mut)
            {
                remove(material);
            }
        }
        Place::Kind(kind) => {
            for field in ["objects", "takes"] {
                if let Some(value) = document.get_mut(field) {
                    strip_kinds(value, kind, addition.fields);
                }
            }
        }
        Place::Tag(tag) => {
            for field in ["objects", "takes"] {
                if let Some(value) = document.get_mut(field) {
                    strip_tags(value, tag, addition.fields);
                }
            }
        }
    }
}

/// The `settings` of a take's own render settings, if any.
fn own_render(value: Option<&mut Value>) -> Option<&mut Map<String, Value>> {
    value?.as_object_mut()?.get_mut("settings")?.as_object_mut()
}

fn has_type(value: &Value, kind: &str) -> bool {
    value.get("type").and_then(Value::as_str) == Some(kind)
}

/// Removes `fields` from every object kind of type `kind` in `value` (the
/// value of a `kind` or `parameters` field, wherever it is).
fn strip_kinds(value: &mut Value, kind: &str, fields: &[&str]) {
    match value {
        Value::Object(map) => {
            for (name, value) in map.iter_mut() {
                if (name == "kind" || name == "parameters")
                    && has_type(value, kind)
                    && let Some(kind_map) = value.as_object_mut()
                {
                    for field in fields {
                        kind_map.remove(*field);
                    }
                }
                strip_kinds(value, kind, fields);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| strip_kinds(item, kind, fields)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

/// Removes `fields` from every tag of type `tag` in `value` (any `tags` list,
/// wherever it is).
fn strip_tags(value: &mut Value, tag: &str, fields: &[&str]) {
    match value {
        Value::Object(map) => {
            if let Some(Value::Array(tags)) = map.get_mut("tags") {
                for found in tags
                    .iter_mut()
                    .filter(|t| has_type(t, tag))
                    .filter_map(Value::as_object_mut)
                {
                    for field in fields {
                        found.remove(*field);
                    }
                }
            }
            map.values_mut().for_each(|value| strip_tags(value, tag, fields));
        }
        Value::Array(items) => items.iter_mut().for_each(|item| strip_tags(item, tag, fields)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test code")]

    use serde_json::json;

    use super::*;

    #[test]
    fn upgrading_names_version_5_and_keeps_the_document() {
        let v4 = json!({
            "format": "grid4d",
            "version": 4,
            "document": { "name": "Shot", "objects": [], "render": { "samples": 64 } },
            "meshes": []
        });
        let v5 = upgrade(v4.clone()).unwrap();
        assert_eq!(v5["version"], 5);
        assert_eq!(v5["document"], v4["document"], "nothing version 5 adds is in it");
        assert_eq!(v5["format"], "grid4d");
        assert_eq!(v5["meshes"], v4["meshes"]);
    }

    /// The additions a later step will list, to try the mechanism now.
    const TRIAL: &[Added] = &[
        Added {
            place: Place::Document,
            fields: &["render_region"],
        },
        Added {
            place: Place::Render,
            fields: &["denoise"],
        },
        Added {
            place: Place::Material,
            fields: &["subsurface"],
        },
        Added {
            place: Place::Kind("camera"),
            fields: &["f_stop"],
        },
        Added {
            place: Place::Tag("material"),
            fields: &["displacement"],
        },
    ];

    #[test]
    fn what_version_5_adds_is_dropped_from_a_version_4_file() {
        let v4 = json!({
            "version": 4,
            "document": {
                "name": "Shot",
                "render_region": [0, 0, 1, 1],
                "render": { "samples": 64, "denoise": true },
                "materials": [{ "name": "Skin", "subsurface": 1 }],
                "objects": [
                    { "name": "Cam", "kind": { "type": "camera", "fov_y_deg": 40, "f_stop": 2.8 },
                      "tags": [{ "type": "material", "material": 0, "displacement": 2 },
                               { "type": "phong", "displacement": 2 }] },
                    { "name": "Cube", "kind": { "type": "cube", "f_stop": 1 } }
                ],
                "takes": {
                    "main_render": { "settings": { "samples": 8, "denoise": true } },
                    "takes": [{ "name": "Close", "render": { "settings": { "denoise": false } },
                        "objects": [{ "object": 0, "parameters": { "type": "camera", "f_stop": 4 },
                                      "tags": [{ "type": "material", "displacement": 1 }] }] }]
                }
            }
        });
        let v5 = upgrade_with(v4, TRIAL, &[]).unwrap();
        assert_eq!(
            v5["document"],
            json!({
                "name": "Shot",
                "render": { "samples": 64 },
                "materials": [{ "name": "Skin" }],
                "objects": [
                    { "name": "Cam", "kind": { "type": "camera", "fov_y_deg": 40 },
                      "tags": [{ "type": "material", "material": 0 },
                               { "type": "phong", "displacement": 2 }] },
                    { "name": "Cube", "kind": { "type": "cube", "f_stop": 1 } }
                ],
                "takes": {
                    "main_render": { "settings": { "samples": 8 } },
                    "takes": [{ "name": "Close", "render": { "settings": {} },
                        "objects": [{ "object": 0, "parameters": { "type": "camera" },
                                      "tags": [{ "type": "material" }] }] }]
                }
            }),
            "only the listed places lose them"
        );
    }

    #[test]
    fn what_version_5_adds_as_a_value_is_refused_in_a_version_4_file() {
        let new = [NewValue {
            field: "light_type",
            value: "area",
        }];
        let v4 = |light_type: &str| {
            json!({
                "version": 4,
                "document": { "takes": { "takes": [{ "objects": [{ "object": 0,
                    "parameters": { "type": "light", "light_type": light_type } }] }] } }
            })
        };
        let err = upgrade_with(v4("area"), &[], &new).unwrap_err();
        assert!(
            matches!(&err, IoError::Invalid(message) if message.contains("\"light_type\": \"area\"")),
            "{err:?}"
        );
        assert_eq!(upgrade_with(v4("spot"), &[], &new).unwrap()["version"], 5);
    }

    #[test]
    fn what_is_not_an_object_is_not_a_document() {
        for json in [json!([]), json!(4), json!("grid4d")] {
            assert!(matches!(upgrade(json), Err(IoError::NotADocument)));
        }
    }
}
