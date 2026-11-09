//! Format version 4 (Grid4D 1.1), frozen: read only, through version 5 (and
//! from there through version 6, `v5.rs`).
//!
//! Version 5 keeps version 4's layout and only adds to it, so a version 4
//! `document.json` is read by turning it into a version 5 one ([`upgrade`], a
//! change of the JSON itself; the mechanism is [`crate::upgrade`]) and then
//! on. This module is the whole difference between the two: what version 5
//! adds as fields ([`ADDED`], dropped from a version 4 file) and as values
//! ([`NEW_VALUES`], refused in one).
//!
//! The fixtures `demo_v4.g4d` and `full_v4.g4d` (every thing version 4 holds)
//! pin what a version 4 file means; they are never regenerated. A version 3
//! file is first upgraded to version 4 ([`crate::v3::upgrade`]), then through
//! here.

use serde_json::Value;

use crate::{
    IoError,
    upgrade::{Added, NewValue, Place, additions},
};

pub(crate) const VERSION: u32 = 4;

/// Everything version 5 adds as fields, by step of Phase 16.
const ADDED: &[Added] = &[
    // Step 16.2: the two light-transport switches; an area light's shape.
    Added {
        place: Place::Render,
        fields: &["clamp_indirect", "emissive_lights"],
    },
    Added {
        place: Place::Kind("light"),
        fields: &["area"],
    },
    // Step 16.5: motion blur, a camera's lens and shutter.
    Added {
        place: Place::Render,
        fields: &["motion_blur", "segments"],
    },
    // Step 16.6: render passes.
    Added {
        place: Place::Render,
        fields: &["passes", "png_passes"],
    },
    // Step 16.7: adaptive sampling.
    Added {
        place: Place::Render,
        fields: &[
            "adaptive",
            "min_samples",
            "noise_threshold",
            "denoise",
            "denoise_strength",
        ],
    },
    // Step 16.8: the render region.
    Added {
        place: Place::Render,
        fields: &["region", "region_final"],
    },
    // Step 16.10: subsurface scattering.
    Added {
        place: Place::Material,
        fields: &[
            "subsurface",
            "subsurface_color",
            "subsurface_radius",
            "subsurface_scale",
            "displace",
            "displacement_mid",
            "displacement_level",
        ],
    },
    Added {
        place: Place::Render,
        fields: &["displacement_budget"],
    },
    Added {
        place: Place::Kind("camera"),
        fields: &["lens"],
    },
    // Step 16.4: which objects a light lights, and its light group.
    Added {
        place: Place::Kind("light"),
        fields: &["linking", "group"],
    },
];

/// Every value version 5 adds to a closed set, by step of Phase 16.
const NEW_VALUES: &[NewValue] = &[
    // Step 16.2: the Area light type.
    NewValue {
        field: "light_type",
        value: "area",
    },
    // Step 16.3: the Sky and Environment objects.
    NewValue {
        field: "type",
        value: "sky",
    },
    NewValue {
        field: "type",
        value: "environment",
    },
];

/// A version 4 `document.json`, already parsed, as version 5 (see the module
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

    #[test]
    fn version_5_s_additions_are_dropped_and_its_values_refused() {
        let v4 = json!({
            "version": 4,
            "document": {
                "render": { "samples": 64, "adaptive": true, "region": [0, 0, 1, 1] },
                "materials": [{ "name": "Wax", "subsurface": 0.5 }],
                "objects": [{ "name": "Lamp", "kind": { "type": "light", "group": "Key" } }]
            }
        });
        let v5 = upgrade(v4).unwrap();
        assert_eq!(
            v5["document"],
            json!({
                "render": { "samples": 64 },
                "materials": [{ "name": "Wax" }],
                "objects": [{ "name": "Lamp", "kind": { "type": "light" } }]
            })
        );
        let sky = json!({ "version": 4, "document": { "objects": [{ "kind": { "type": "sky" } }] } });
        let err = upgrade(sky).unwrap_err();
        assert!(err.to_string().contains("not part of format version 4"), "{err}");
    }
}
