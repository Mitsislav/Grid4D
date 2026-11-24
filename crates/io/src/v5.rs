//! Format version 5 (Grid4D 1.2), frozen: read only, through version 6.
//!
//! Version 6 keeps version 5's layout and only adds to it, so a version 5
//! `document.json` is read by turning it into a version 6 one ([`upgrade`], a
//! change of the JSON itself; the mechanism is [`crate::upgrade`]) and then
//! through version 6's types. This module is the whole difference between the
//! two, and it keeps version 5's meaning fixed while version 6 grows:
//!
//! * every field version 6 adds is listed in [`ADDED`] and **removed** from a
//!   version 5 file (Grid4D 1.2 ignored fields it did not know);
//! * every value version 6 adds to a closed set (a new kind of object, a new
//!   deformer) is listed in [`NEW_VALUES`] and stays an **error** in a version
//!   5 file, as it was for 1.2;
//! * where version 6 gives an old field a new form, the conversion is written
//!   here, from the version 5 form.
//!
//! Each step of Phase 17 that adds data lists it here and in
//! `docs/FILE_FORMAT.md`. The fixtures `demo_v5.g4d` and `full_v5.g4d` (every
//! thing version 5 holds) pin what a version 5 file means; they are never
//! regenerated. Versions 3 and 4 are first upgraded to version 5
//! ([`crate::v3::upgrade`], [`crate::v4::upgrade`]), then through here.

use serde_json::Value;

use crate::{
    IoError,
    upgrade::{Added, NewValue, additions},
};

pub(crate) const VERSION: u32 = 5;

/// Everything version 6 adds as fields, by step of Phase 17 (none yet: step
/// 17.1 makes the version, later steps fill it).
const ADDED: &[Added] = &[];

/// Every value version 6 adds to a closed set, by step of Phase 17.
const NEW_VALUES: &[NewValue] = &[
    // Step 17.3: the Boolean generator.
    NewValue {
        field: "type",
        value: "boolean",
    },
    // Step 17.4: the Symmetry, Instance and Join objects.
    NewValue {
        field: "type",
        value: "symmetry",
    },
    NewValue {
        field: "type",
        value: "instance",
    },
    NewValue {
        field: "type",
        value: "join",
    },
];

/// A version 5 `document.json`, already parsed, as version 6 (see the module
/// documentation).
pub(crate) fn upgrade(json: Value) -> Result<Value, IoError> {
    additions(json, VERSION, ADDED, NEW_VALUES)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test code")]

    use serde_json::json;

    use super::*;

    #[test]
    fn upgrading_names_version_6_and_keeps_the_document() {
        let v5 = json!({
            "format": "grid4d",
            "version": 5,
            "document": {
                "name": "Shot",
                "objects": [{ "name": "Sky", "kind": { "type": "sky" } }],
                "render": { "samples": 64, "adaptive": true }
            },
            "meshes": []
        });
        let v6 = upgrade(v5.clone()).unwrap();
        assert_eq!(v6["version"], 6);
        assert_eq!(v6["document"], v5["document"], "nothing version 6 adds is in it");
        assert_eq!(v6["format"], "grid4d");
        assert_eq!(v6["meshes"], v5["meshes"]);
    }

    #[test]
    fn what_is_not_an_object_is_not_a_document() {
        assert!(matches!(upgrade(json!([5])), Err(IoError::NotADocument)));
    }
}
