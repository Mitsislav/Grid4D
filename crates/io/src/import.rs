//! Objects read from a file to add to a document (File ▸ Merge) or to make a new
//! one from (File ▸ Open of another format): a flat list in Object Manager
//! pre-order, each node naming its parent by index.

use std::{collections::HashMap, fs, path::Path};

use grid4d_scene::{Document, Node, NodeData, NodeId};

use crate::{IoError, OPEN_EXTENSIONS, gltf_import, load};

/// Extensions of the formats [`import`] reads besides Grid4D documents.
pub const IMPORT_EXTENSIONS: [&str; 2] = ["gltf", "glb"];

/// Objects read from a file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Imported {
    /// In Object Manager pre-order: every parent comes before its children.
    pub nodes: Vec<ImportedNode>,
}

/// One object of an [`Imported`] set.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportedNode {
    /// Index of the parent in [`Imported::nodes`] (always an earlier one); `None`
    /// for a top-level object.
    pub parent: Option<usize>,
    pub data: NodeData,
}

impl Imported {
    /// Every object of `doc`, in Object Manager order.
    #[must_use]
    pub fn from_document(doc: &Document) -> Self {
        let mut nodes = Vec::new();
        let mut index_of: HashMap<NodeId, usize> = HashMap::new();
        let mut stack: Vec<NodeId> = doc.scene.roots().iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            let Some(node) = doc.scene.get(id) else { continue };
            let parent = node.parent().and_then(|p| index_of.get(&p).copied());
            index_of.insert(id, nodes.len());
            nodes.push(ImportedNode {
                parent,
                data: node.data.clone(),
            });
            stack.extend(node.children().iter().rev());
        }
        Self { nodes }
    }

    /// A new document named `name` holding these objects.
    #[must_use]
    pub fn into_document(self, name: &str) -> Document {
        let mut doc = Document::new(name);
        let mut ids: Vec<NodeId> = Vec::with_capacity(self.nodes.len());
        for node in self.nodes {
            let parent = node.parent.and_then(|p| ids.get(p).copied());
            let id = match parent {
                // The parent was just inserted, so this cannot fail; fall back to the top level.
                Some(parent) => doc
                    .scene
                    .insert_child(parent, Node::from_data(node.data.clone()))
                    .unwrap_or_else(|_| doc.scene.insert(Node::from_data(node.data))),
                None => doc.scene.insert(Node::from_data(node.data)),
            };
            ids.push(id);
        }
        doc
    }
}

/// `true` if `path` has an extension of [`IMPORT_EXTENSIONS`] (any letter case).
#[must_use]
pub fn is_importable(path: &Path) -> bool {
    has_extension(path, &IMPORT_EXTENSIONS)
}

fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .is_some_and(|ext| extensions.iter().any(|known| ext.eq_ignore_ascii_case(known)))
}

/// The objects of a glTF (`.gltf`, `.glb`) file or a Grid4D document.
///
/// # Errors
///
/// [`IoError`] if the file cannot be read, is not in a supported format or is
/// broken (see `docs/reference/interchange-gltf.md`).
pub fn import(path: &Path) -> Result<Imported, IoError> {
    if has_extension(path, &OPEN_EXTENSIONS) {
        return load(path).map(|doc| Imported::from_document(&doc));
    }
    if !is_importable(path) {
        return Err(IoError::Invalid(format!(
            "unsupported file type (Grid4D reads .{}, .gltf and .glb)",
            crate::EXTENSION
        )));
    }
    let bytes = fs::read(path).map_err(IoError::Io)?;
    gltf_import::import_gltf(&bytes, path.parent())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_round_trips_through_its_object_list() {
        let doc = Document::with_demo_scene();
        let imported = Imported::from_document(&doc);
        assert_eq!(imported.nodes.len(), doc.scene.len());
        let group = imported.nodes.iter().position(|n| n.data.name == "Group").unwrap();
        let children: Vec<&str> = imported
            .nodes
            .iter()
            .filter(|n| n.parent == Some(group))
            .map(|n| n.data.name.as_str())
            .collect();
        assert_eq!(children, ["Cylinder", "Box"]);
        assert!(
            imported
                .nodes
                .iter()
                .enumerate()
                .all(|(i, n)| n.parent.is_none_or(|p| p < i))
        );

        let back = imported.clone().into_document("Copy");
        assert_eq!(Imported::from_document(&back), imported);
        assert_eq!(back.name, "Copy");
    }

    #[test]
    fn importable_paths_and_unsupported_ones() {
        assert!(is_importable(Path::new("model.GLB")));
        assert!(is_importable(Path::new("dir/model.gltf")));
        assert!(!is_importable(Path::new("model.obj")));
        let err = import(Path::new("model.obj")).unwrap_err();
        assert!(err.to_string().contains("unsupported"), "{err}");
    }
}
