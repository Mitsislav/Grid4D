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
const V4_DOCUMENT_FIELDS: &[&str] = &[];

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
    fn what_is_not_an_object_is_not_a_document() {
        for json in [json!([]), json!(3), json!("grid4d")] {
            assert!(matches!(upgrade(json), Err(IoError::NotADocument)));
        }
    }
}
