//! Logic shared by every format version: assembling a document from objects
//! stored in pre-order with parent indices, number validation, and reading
//! archive entries.

use std::io::{Cursor, Read as _};

use grid4d_scene::{Document, Node, NodeData, NodeId, TimeSettings};
use zip::{ZipArchive, result::ZipError};

use crate::IoError;

/// A document archive (format version 3 and later) being read.
pub(crate) struct Archive<'a>(ZipArchive<Cursor<&'a [u8]>>);

impl<'a> Archive<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Result<Self, IoError> {
        Ok(Self(ZipArchive::new(Cursor::new(bytes))?))
    }

    /// The decompressed contents of entry `name`, which may be at most `max`
    /// bytes: what the file *claims* about sizes is never trusted for memory.
    pub(crate) fn read(&mut self, name: &str, max: u64) -> Result<Vec<u8>, IoError> {
        let entry = self.0.by_name(name).map_err(|err| match err {
            ZipError::FileNotFound => IoError::Invalid(format!("the file has no \"{name}\"")),
            other => other.into(),
        })?;
        let mut data = Vec::with_capacity(usize::try_from(entry.size().min(max)).unwrap_or(0));
        entry.take(max.saturating_add(1)).read_to_end(&mut data)?;
        if u64::try_from(data.len()).unwrap_or(u64::MAX) > max {
            return Err(IoError::Invalid(format!("\"{name}\" is larger than expected")));
        }
        Ok(data)
    }
}

/// Builds a document from `objects` in Object Manager pre-order, each naming an
/// earlier object as its parent (`None` for top-level objects).
pub(crate) fn assemble(
    name: String,
    time: TimeSettings,
    objects: impl IntoIterator<Item = (Option<usize>, NodeData)>,
) -> Result<Document, IoError> {
    assemble_with_ids(name, time, objects).map(|(doc, _)| doc)
}

/// [`assemble`], also returning each object's id (in the order given).
pub(crate) fn assemble_with_ids(
    name: String,
    time: TimeSettings,
    objects: impl IntoIterator<Item = (Option<usize>, NodeData)>,
) -> Result<(Document, Vec<NodeId>), IoError> {
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
    Ok((doc, ids))
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
