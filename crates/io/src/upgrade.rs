//! The upgrade of a frozen format version to the next one, when the next one
//! only adds: a change of the parsed `document.json` before the newer
//! version's types read it. Versions 4 and 5 (`v4.rs`, `v5.rs`) each hold
//! only the lists of what the version after them adds; this module applies
//! them:
//!
//! * a reader ignored fields it did not know, so every field the next version
//!   adds is **removed** from an older file ([`Added`]) — an older file that
//!   happens to carry one reads as the Grid4D that wrote it read it;
//! * a reader refused types and values it did not know, so the ones the next
//!   version adds ([`NewValue`]: a new kind of object, a new light type) stay
//!   errors in an older file.

use serde_json::{Map, Value};

use crate::IoError;

/// Where in `document.json` a version adds fields.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "a frozen version's list may not use every place")
)]
pub(crate) enum Place {
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

/// Fields a version adds at a place: absent from the version before, and
/// dropped from a file of that version.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Added {
    pub(crate) place: Place,
    pub(crate) fields: &'static [&'static str],
}

/// A value a version adds to a field that held one of a closed set (a `type`
/// of object or tag, a light type): the version before refused it, so it is
/// an error in a file of that version still.
#[derive(Clone, Copy, Debug)]
pub(crate) struct NewValue {
    pub(crate) field: &'static str,
    pub(crate) value: &'static str,
}

/// A `document.json` of format version `from`, already parsed, as version
/// `from + 1`, which adds `added` and `new_values` to it (see the module
/// documentation).
pub(crate) fn additions(
    mut json: Value,
    from: u32,
    added: &[Added],
    new_values: &[NewValue],
) -> Result<Value, IoError> {
    let Some(file) = json.as_object_mut() else {
        return Err(IoError::NotADocument);
    };
    if let Some(document) = file.get("document") {
        refuse_new_values(document, from, new_values)?;
    }
    file.insert("version".to_owned(), Value::from(from + 1));
    if let Some(document) = file.get_mut("document").and_then(Value::as_object_mut) {
        for addition in added {
            strip(document, *addition);
        }
    }
    Ok(json)
}

/// An error if `value` uses any of `new_values` anywhere.
fn refuse_new_values(value: &Value, from: u32, new_values: &[NewValue]) -> Result<(), IoError> {
    match value {
        Value::Object(map) => {
            for new in new_values {
                if map.get(new.field).and_then(Value::as_str) == Some(new.value) {
                    return Err(IoError::Invalid(format!(
                        "\"{}\": \"{}\" is not part of format version {from}",
                        new.field, new.value
                    )));
                }
            }
            map.values()
                .try_for_each(|value| refuse_new_values(value, from, new_values))
        }
        Value::Array(items) => items
            .iter()
            .try_for_each(|value| refuse_new_values(value, from, new_values)),
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

    /// Additions at every kind of place, to try the mechanism whatever the
    /// frozen versions' own lists hold.
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
    fn what_a_version_adds_is_dropped_from_an_older_file() {
        let older = json!({
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
        let newer = additions(older, 4, TRIAL, &[]).unwrap();
        assert_eq!(newer["version"], 5);
        assert_eq!(
            newer["document"],
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
    fn what_a_version_adds_as_a_value_is_refused_in_an_older_file() {
        let new = [NewValue {
            field: "light_type",
            value: "area",
        }];
        let older = |light_type: &str| {
            json!({
                "version": 4,
                "document": { "takes": { "takes": [{ "objects": [{ "object": 0,
                    "parameters": { "type": "light", "light_type": light_type } }] }] } }
            })
        };
        let err = additions(older("area"), 4, &[], &new).unwrap_err();
        assert!(
            matches!(&err, IoError::Invalid(message)
                if message == "\"light_type\": \"area\" is not part of format version 4"),
            "{err:?}"
        );
        assert_eq!(additions(older("spot"), 4, &[], &new).unwrap()["version"], 5);
    }

    #[test]
    fn what_is_not_an_object_is_not_a_document() {
        for json in [json!([]), json!(4), json!("grid4d")] {
            assert!(matches!(additions(json, 5, TRIAL, &[]), Err(IoError::NotADocument)));
        }
    }
}
