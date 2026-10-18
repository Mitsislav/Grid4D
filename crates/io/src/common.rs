//! Logic shared by every format version: assembling a document from objects
//! stored in pre-order with parent indices, and number validation.

use grid4d_scene::{Document, Node, NodeData, NodeId, TimeSettings};

use crate::IoError;

/// Builds a document from `objects` in Object Manager pre-order, each naming an
/// earlier object as its parent (`None` for top-level objects).
pub(crate) fn assemble(
    name: String,
    time: TimeSettings,
    objects: impl IntoIterator<Item = (Option<usize>, NodeData)>,
) -> Result<Document, IoError> {
    let mut doc = Document::new(name);
    // Validated (fps and range clamped) exactly like an edit in the UI.
    doc.time.set_settings(time);
    let mut ids: Vec<NodeId> = Vec::new();
    for (index, (parent, data)) in objects.into_iter().enumerate() {
        let parent = match parent {
            None => None,
            // `ids` holds exactly the objects before this one.
            Some(p) => Some(*ids.get(p).ok_or_else(|| {
                IoError::Invalid(format!(
                    "object {index} (\"{}\") names object {p} as its parent, which does not come before it",
                    data.name
                ))
            })?),
        };
        let id = doc
            .scene
            .insert_at(parent, usize::MAX, Node::from_data(data))
            .map_err(|err| IoError::Invalid(err.to_string()))?;
        ids.push(id);
    }
    Ok(doc)
}

/// JSON has no NaN or infinity (serde_json would write `null`, which then fails
/// to load), and huge literals overflow `f32` on load: refuse both.
pub(crate) fn check_finite(object: &str, groups: &[(&str, &[f32])]) -> Result<(), IoError> {
    for (what, values) in groups {
        if values.iter().any(|v| !v.is_finite()) {
            return Err(IoError::Invalid(format!(
                "object \"{object}\" has a {what} value that is not a finite number"
            )));
        }
    }
    Ok(())
}
