//! Objects moving between a document and another format: read from a file to add
//! to a document (File ▸ Merge) or to make a new one from (File ▸ Open), or taken
//! from a document to write (File ▸ Export). A flat list in Object Manager
//! pre-order, each node naming its parent by index.

use std::{collections::HashMap, path::Path};

use grid4d_scene::{Document, Layers, MaterialId, Materials, Node, NodeData, NodeId, ObjectKind, Tag};

use crate::{IoError, OPEN_EXTENSIONS, gltf_export, gltf_import, load, obj};

/// Extensions of the formats [`import`] reads besides Grid4D documents.
pub const IMPORT_EXTENSIONS: [&str; 3] = ["gltf", "glb", "obj"];
/// Extensions of the formats [`export`] writes.
pub const EXPORT_EXTENSIONS: [&str; 3] = ["glb", "gltf", "obj"];

/// Objects read from a file, or to write to one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Objects {
    /// In Object Manager pre-order: every parent comes before its children.
    pub nodes: Vec<ObjectEntry>,
    /// The materials the nodes' Material tags refer to (their ids are this
    /// table's, not a document's).
    pub materials: Materials,
    /// The layers the nodes are on (their ids are this table's).
    pub layers: Layers,
    /// For objects taken from a document: each node's id there (parallel to
    /// `nodes`), so object references in tags can be carried over. Empty
    /// otherwise.
    pub sources: Vec<NodeId>,
}

/// One object of an [`Objects`] list.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectEntry {
    /// Index of the parent in [`Objects::nodes`] (always an earlier one); `None`
    /// for a top-level object.
    pub parent: Option<usize>,
    pub data: NodeData,
}

impl Objects {
    /// Every object of `doc`, in Object Manager order, with its materials.
    #[must_use]
    pub fn from_document(doc: &Document) -> Self {
        let mut nodes = Vec::new();
        let mut sources = Vec::new();
        let mut index_of: HashMap<NodeId, usize> = HashMap::new();
        let mut stack: Vec<NodeId> = doc.scene.roots().iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            let Some(node) = doc.scene.get(id) else { continue };
            let parent = node.parent().and_then(|p| index_of.get(&p).copied());
            index_of.insert(id, nodes.len());
            nodes.push(ObjectEntry {
                parent,
                data: node.data.clone(),
            });
            sources.push(id);
            stack.extend(node.children().iter().rev());
        }
        Self {
            nodes,
            materials: doc.scene.materials().clone(),
            layers: doc.scene.layers().clone(),
            sources,
        }
    }

    /// A new document named `name` holding these objects and materials.
    #[must_use]
    pub fn into_document(self, name: &str) -> Document {
        let mut doc = Document::new(name);
        // A new document: the table moves over with its ids, so tags stay valid.
        *doc.scene.materials_mut() = self.materials;
        *doc.scene.layers_mut() = self.layers;
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
        let objects: HashMap<NodeId, NodeId> = self.sources.iter().copied().zip(ids.iter().copied()).collect();
        for &id in &ids {
            if let Some(node) = doc.scene.get_mut(id) {
                remap_objects(&mut node.data, |old| objects.get(&old).copied());
            }
        }
        doc
    }

    /// Each node's effective material (its own Material tag's, else its nearest
    /// ancestor's), for exporters.
    #[must_use]
    pub fn effective_materials(&self) -> Vec<Option<MaterialId>> {
        let mut effective: Vec<Option<MaterialId>> = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            let own = grid4d_scene::Tags::material(node.data.tags.as_slice()).filter(|&m| self.materials.contains(m));
            let inherited = node.parent.and_then(|p| effective.get(p).copied().flatten());
            effective.push(own.or(inherited));
        }
        effective
    }
}

/// What an exported mesh looks like: a material, or a plain display colour (as
/// bits, so it can be a key).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Look {
    Material(MaterialId),
    Colour([u32; 3]),
}

/// Rewrites the object references in `data` (tags, a Cloner's object) through `map` (a reference
/// `map` does not know is dropped).
pub fn remap_objects(data: &mut NodeData, map: impl Fn(NodeId) -> Option<NodeId>) {
    for tag in &mut data.tags {
        if let Tag::Target { target, up, .. } = tag {
            *target = target.and_then(&map);
            *up = up.and_then(&map);
        }
    }
    if let ObjectKind::Cloner { object, effectors, .. } = &mut data.kind {
        *object = object.and_then(&map);
        effectors.retain_mut(|id| map(*id).map(|new| *id = new).is_some());
    }
}

/// Rewrites the material references in `data`'s tags through `map` (a reference
/// `map` does not know is dropped).
pub fn remap_materials(data: &mut NodeData, map: impl Fn(MaterialId) -> Option<MaterialId>) {
    for tag in &mut data.tags {
        if let Tag::Material { material } = tag {
            *material = material.and_then(&map);
        }
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

/// The objects of a glTF (`.gltf`, `.glb`) or Wavefront OBJ file, or of a Grid4D
/// document.
///
/// # Errors
///
/// [`IoError`] if the file cannot be read, is not in a supported format or is
/// broken (see `docs/reference/interchange-gltf.md`).
pub fn import(path: &Path) -> Result<Objects, IoError> {
    if has_extension(path, &OPEN_EXTENSIONS) {
        return load(path).map(|doc| Objects::from_document(&doc));
    }
    if !is_importable(path) {
        return Err(IoError::Invalid(format!(
            "unsupported file type (Grid4D reads .{}, .gltf, .glb and .obj)",
            crate::EXTENSION
        )));
    }
    let bytes = crate::read_file(path)?;
    if has_extension(path, &["obj"]) {
        let name = path
            .file_stem()
            .map_or_else(|| "Object".into(), |s| s.to_string_lossy());
        return obj::import_obj(&bytes, path.parent(), &name);
    }
    gltf_import::import_gltf(&bytes, path.parent())
}

/// Writes `objects` to `path` in the format its extension names: `.glb`, `.gltf`
/// (+ `.bin`) or `.obj` (+ `.mtl`).
///
/// # Errors
///
/// [`IoError::Invalid`] for an unknown extension or non-finite numbers,
/// [`IoError::Io`] if a file cannot be written.
pub fn export(objects: &Objects, path: &Path) -> Result<(), IoError> {
    if has_extension(path, &["obj"]) {
        obj::export_obj(objects, path)
    } else if has_extension(path, &["glb", "gltf"]) {
        gltf_export::export_gltf(objects, path)
    } else {
        Err(IoError::Invalid("export as .glb, .gltf or .obj".to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_round_trips_through_its_object_list() {
        let doc = Document::with_demo_scene();
        let imported = Objects::from_document(&doc);
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
        assert_eq!(Objects::from_document(&back), imported);
        assert_eq!(back.name, "Copy");
    }

    #[test]
    fn materials_are_inherited_and_carried_into_new_documents() {
        use grid4d_scene::{Material, Tag};
        let mut doc = Document::with_demo_scene();
        let red = doc.scene.materials_mut().insert(Material::new("Red"));
        let id = |doc: &Document, name: &str| doc.scene.iter().find(|(_, n)| n.data.name == name).unwrap().0;
        let (group, cube, boxed) = (id(&doc, "Group"), id(&doc, "Cube"), id(&doc, "Box"));
        doc.scene.get_mut(group).unwrap().data.tags = vec![Tag::Material { material: Some(red) }];
        // The Cube aims at the Box: an object reference.
        doc.scene.get_mut(cube).unwrap().data.tags = vec![Tag::Target {
            target: Some(boxed),
            up: None,
            pitch: true,
        }];
        let objects = Objects::from_document(&doc);
        let effective = objects.effective_materials();
        for (node, material) in objects.nodes.iter().zip(&effective) {
            let expected = matches!(node.data.name.as_str(), "Group" | "Cylinder" | "Box").then_some(red);
            assert_eq!(*material, expected, "{}", node.data.name);
        }

        let copy = objects.into_document("Copy");
        let copy_group = id(&copy, "Group");
        assert_eq!(copy.scene.effective_material(id(&copy, "Box")), Some(red));
        assert_eq!(copy.scene.materials().get(red).unwrap().name, "Red");
        assert_eq!(copy.scene.get(copy_group).unwrap().data.tags.len(), 1);
        let aim = grid4d_scene::Tags::aim(copy.scene.get(id(&copy, "Cube")).unwrap().data.tags.as_slice());
        assert_eq!(
            aim.map(|a| a.target),
            Some(id(&copy, "Box")),
            "the reference follows its object"
        );
    }

    #[test]
    fn importable_paths_and_unsupported_ones() {
        assert!(is_importable(Path::new("model.GLB")));
        assert!(is_importable(Path::new("dir/model.gltf")));
        assert!(is_importable(Path::new("model.OBJ")));
        assert!(!is_importable(Path::new("model.fbx")));
        let err = import(Path::new("model.fbx")).unwrap_err();
        assert!(err.to_string().contains("unsupported"), "{err}");
    }
}
