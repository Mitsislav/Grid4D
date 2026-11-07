use std::{
    fmt,
    sync::atomic::{AtomicU64, Ordering},
};

use glam::Mat4;
use slotmap::SlotMap;

use crate::{Aabb, Node, NodeId, Transform};

/// Errors returned by structural scene edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneError {
    NodeNotFound,
    /// The edit would make a node its own ancestor.
    WouldCreateCycle,
    /// The node is not the root of a subtree removed with [`Scene::detach`].
    NotDetached,
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NodeNotFound => f.write_str("node not found in scene"),
            Self::WouldCreateCycle => f.write_str("a node cannot be parented under itself or its descendants"),
            Self::NotDetached => f.write_str("node is not the root of a detached subtree"),
        }
    }
}

impl std::error::Error for SceneError {}

/// Where a subtree sat in the hierarchy before [`Scene::detach`] — exactly what
/// [`Scene::attach`] needs to put it back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Detached {
    pub root: NodeId,
    pub parent: Option<NodeId>,
    /// Position among the parent's children (or the roots).
    pub index: usize,
}

/// Source of every revision number: process-wide, so revisions of different
/// scenes (a replaced document, a loaded file) never collide.
static REVISIONS: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_revision() -> u64 {
    REVISIONS.fetch_add(1, Ordering::Relaxed)
}

/// One entry of the explicit traversal stack used by [`Scene::walk`].
///
/// The stack is owned by the caller so that per-frame traversals (render
/// extraction, picking, framing) reuse its capacity and never allocate.
#[derive(Clone, Copy, Debug)]
pub struct WalkItem {
    id: NodeId,
    parent_world: Mat4,
    depth: u32,
}

/// The scene graph: an arena of [`Node`]s plus the ordered list of root nodes.
///
/// ## Detached subtrees
///
/// Deleting with [`Scene::detach`] unlinks a subtree but keeps its nodes in the
/// arena, so that [`Scene::attach`] can restore it with the **same** `NodeId`s —
/// a slotmap never re-issues a key, so re-inserting would break every handle that
/// undo history and the selection hold. Detached nodes are invisible to all
/// public queries (`get`, `contains`, `len`, `iter`, `walk`, `children`, …) until
/// re-attached, and are freed for good by [`Scene::purge`].
///
/// Invariants: every attached node is reachable from `roots` exactly once with
/// consistent parent links; every detached node belongs to exactly one subtree
/// listed in `detached_roots`, whose root has no parent.
///
/// ## Revisions
///
/// [`Scene::revision`] changes whenever anything in the scene may have changed:
/// every [`Scene::get_mut`] and every structural edit. Read-only access never
/// changes it, so code that only *looks* must use [`Scene::get`] — this is what
/// lets the evaluation skip frames in which nothing changed.
#[derive(Clone, Debug)]
pub struct Scene {
    nodes: SlotMap<NodeId, Node>,
    roots: Vec<NodeId>,
    /// Roots of the subtrees removed with `detach`, kept alive until `purge`.
    detached_roots: Vec<NodeId>,
    /// Number of nodes, across all detached subtrees, hidden from public queries.
    detached_count: usize,
    /// The document's materials (here, so evaluation sees changes through the
    /// revision).
    materials: crate::Materials,
    /// The document's layers (here for the same reason).
    layers: crate::Layers,
    /// The document's textures (here for the same reason).
    textures: crate::Textures,
    revision: u64,
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            nodes: SlotMap::with_key(),
            roots: Vec::new(),
            detached_roots: Vec::new(),
            detached_count: 0,
            materials: crate::Materials::default(),
            layers: crate::Layers::default(),
            textures: crate::Textures::default(),
            revision: next_revision(),
        }
    }
}

impl Scene {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Changes whenever anything in the scene may have changed (see the type docs).
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The document's materials.
    #[must_use]
    pub fn materials(&self) -> &crate::Materials {
        &self.materials
    }

    /// Write access to the materials (marks the scene as changed).
    pub fn materials_mut(&mut self) -> &mut crate::Materials {
        self.touch();
        &mut self.materials
    }

    /// The document's textures.
    #[must_use]
    pub fn textures(&self) -> &crate::Textures {
        &self.textures
    }

    /// Write access to the textures (marks the scene as changed).
    pub fn textures_mut(&mut self) -> &mut crate::Textures {
        self.touch();
        &mut self.textures
    }

    /// The document's layers.
    #[must_use]
    pub fn layers(&self) -> &crate::Layers {
        &self.layers
    }

    /// Write access to the layers (marks the scene as changed).
    pub fn layers_mut(&mut self) -> &mut crate::Layers {
        self.touch();
        &mut self.layers
    }

    /// The layer an object is on: its own, else the nearest ancestor's (links
    /// to missing layers count as none).
    #[must_use]
    pub fn effective_layer(&self, id: NodeId) -> Option<crate::LayerId> {
        let mut current = Some(id);
        while let Some(c) = current {
            let node = self.get(c)?;
            if let Some(layer) = node.data.layer.filter(|&l| self.layers.contains(l)) {
                return Some(layer);
            }
            current = node.parent;
        }
        None
    }

    /// The switches of the layer `id` is on (the defaults without one).
    #[must_use]
    pub fn layer_switches(&self, id: NodeId) -> crate::LayerSwitches {
        if self.layers.is_empty() {
            return crate::LayerSwitches::default();
        }
        self.layers.switches(self.effective_layer(id))
    }

    /// Whether `id` follows its animation (its layer's Animation switch).
    #[must_use]
    pub fn animates(&self, id: NodeId) -> bool {
        self.layers.all_animate() || self.layer_switches(id).animation
    }

    /// Whether `id` is on a locked layer (not pickable or movable in the views).
    #[must_use]
    pub fn is_locked(&self, id: NodeId) -> bool {
        self.layer_switches(id).locked
    }

    /// What the views may not change of `id`: its Protection tag's channels,
    /// every channel on a locked layer.
    #[must_use]
    pub fn protection(&self, id: NodeId) -> crate::Protection {
        if self.is_locked(id) {
            return crate::Protection {
                position: true,
                scale: true,
                rotation: true,
            };
        }
        self.get(id).map_or_else(crate::Protection::default, |n| {
            crate::Tags::protection(n.data.tags.as_slice())
        })
    }

    /// The material an object shows: its own Material tag's, else the nearest
    /// ancestor's (links to missing materials count as none).
    #[must_use]
    pub fn effective_material(&self, id: NodeId) -> Option<crate::MaterialId> {
        let mut current = Some(id);
        while let Some(c) = current {
            let node = self.get(c)?;
            if let Some(material) = crate::Tags::material_in(node.data.tags.as_slice(), |m| self.materials.contains(m))
            {
                return Some(material);
            }
            current = node.parent;
        }
        None
    }

    /// Marks the scene as changed.
    fn touch(&mut self) -> u64 {
        self.revision = next_revision();
        self.revision
    }

    /// Number of attached nodes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len() - self.detached_count
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `true` if `id` is an attached node of this scene.
    #[must_use]
    pub fn contains(&self, id: NodeId) -> bool {
        self.get(id).is_some()
    }

    #[must_use]
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id).filter(|n| !n.detached)
    }

    /// Mutable access to an attached node. Marks the node and the scene as
    /// changed — use [`Scene::get`] to only read.
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        if !self.contains(id) {
            return None;
        }
        let revision = self.touch();
        let node = self.nodes.get_mut(id)?;
        node.revision = revision;
        Some(node)
    }

    /// A node's data, detached nodes included (a take's values reach objects
    /// deleted for now, so undo brings them back as they were).
    pub(crate) fn data_mut_any(&mut self, id: NodeId) -> Option<&mut crate::NodeData> {
        if !self.nodes.contains_key(id) {
            return None;
        }
        let revision = self.touch();
        let node = self.nodes.get_mut(id)?;
        node.revision = revision;
        Some(&mut node.data)
    }

    /// Top-level nodes, in Object Manager order.
    #[must_use]
    pub fn roots(&self) -> &[NodeId] {
        &self.roots
    }

    /// Children of `id` in order (empty if `id` is unknown or detached).
    #[must_use]
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.get(id).map_or(&[], |n| n.children.as_slice())
    }

    /// Position of `id` among its parent's children (or the roots).
    #[must_use]
    pub fn index_in_parent(&self, id: NodeId) -> Option<usize> {
        let siblings = match self.get(id)?.parent {
            Some(p) => self.children(p),
            None => self.roots.as_slice(),
        };
        siblings.iter().position(|&c| c == id)
    }

    /// All attached nodes in storage order (not hierarchy order — use
    /// [`Scene::walk`] for that).
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &Node)> + '_ {
        self.nodes.iter().filter(|(_, n)| !n.detached)
    }

    /// Roots of the detached subtrees, oldest first.
    #[must_use]
    pub fn detached_roots(&self) -> &[NodeId] {
        &self.detached_roots
    }

    /// Inserts `node` as the last root.
    pub fn insert(&mut self, node: Node) -> NodeId {
        let id = self.nodes.insert(fresh(node));
        self.roots.push(id);
        self.touch();
        id
    }

    /// Inserts `node` as the last child of `parent`.
    pub fn insert_child(&mut self, parent: NodeId, node: Node) -> Result<NodeId, SceneError> {
        self.insert_at(Some(parent), usize::MAX, node)
    }

    /// Inserts `node` at `index` (clamped) among the children of `parent`
    /// (`None` = the roots).
    pub fn insert_at(&mut self, parent: Option<NodeId>, index: usize, node: Node) -> Result<NodeId, SceneError> {
        if let Some(p) = parent
            && !self.contains(p)
        {
            return Err(SceneError::NodeNotFound);
        }
        let id = self.nodes.insert(fresh(node));
        self.link(id, parent, index);
        self.touch();
        Ok(id)
    }

    /// Permanently removes `id` and its whole subtree. Returns the number of
    /// nodes removed (0 if `id` is unknown or detached).
    ///
    /// Use [`Scene::detach`] instead when the removal must be undoable.
    pub fn remove(&mut self, id: NodeId) -> usize {
        if self.detach(id).is_err() {
            return 0;
        }
        self.purge(id).unwrap_or(0)
    }

    /// Unlinks `id` and its subtree from the hierarchy but keeps every node
    /// stored, so [`Scene::attach`] can restore it with the same `NodeId`s.
    pub fn detach(&mut self, id: NodeId) -> Result<Detached, SceneError> {
        if !self.contains(id) {
            return Err(SceneError::NodeNotFound);
        }
        let (parent, index) = self.unlink(id).ok_or(SceneError::NodeNotFound)?;
        self.detached_count += self.set_subtree_detached(id, true);
        self.detached_roots.push(id);
        self.touch();
        Ok(Detached {
            root: id,
            parent,
            index,
        })
    }

    /// Re-links a subtree removed with [`Scene::detach`] at `index` (clamped)
    /// among the children of `parent` (`None` = the roots).
    ///
    /// `parent` must be attached, so a subtree can never be attached into itself.
    pub fn attach(&mut self, root: NodeId, parent: Option<NodeId>, index: usize) -> Result<(), SceneError> {
        self.check_detached_root(root)?;
        if let Some(p) = parent
            && !self.contains(p)
        {
            return Err(SceneError::NodeNotFound);
        }
        self.detached_roots.retain(|&r| r != root);
        self.detached_count -= self.set_subtree_detached(root, false);
        self.link(root, parent, index);
        let revision = self.touch();
        if let Some(node) = self.nodes.get_mut(root) {
            node.revision = revision;
        }
        Ok(())
    }

    /// Frees a detached subtree for good. Returns the number of nodes freed.
    pub fn purge(&mut self, root: NodeId) -> Result<usize, SceneError> {
        self.check_detached_root(root)?;
        self.detached_roots.retain(|&r| r != root);
        let mut removed = 0;
        let mut pending = vec![root];
        while let Some(current) = pending.pop() {
            if let Some(node) = self.nodes.remove(current) {
                removed += 1;
                pending.extend(node.children);
            }
        }
        self.detached_count -= removed;
        self.touch();
        Ok(removed)
    }

    /// Frees every detached subtree (e.g. when the undo history is cleared).
    /// Returns the number of nodes freed.
    pub fn purge_all_detached(&mut self) -> usize {
        let roots = std::mem::take(&mut self.detached_roots);
        roots.into_iter().map(|root| self.purge(root).unwrap_or(0)).sum()
    }

    /// `true` if `ancestor` is a (direct or indirect) parent of `node`.
    #[must_use]
    pub fn is_ancestor_of(&self, ancestor: NodeId, node: NodeId) -> bool {
        let mut current = self.get(node).and_then(|n| n.parent);
        while let Some(c) = current {
            if c == ancestor {
                return true;
            }
            current = self.get(c).and_then(|n| n.parent);
        }
        false
    }

    /// Moves `id` (with its subtree) under `new_parent` (`None` = make it a root),
    /// as the last child. Does nothing if `new_parent` already is the parent.
    ///
    /// With `keep_world = true` the node keeps its world-space placement, like
    /// dragging objects in Cinema 4D's Object Manager.
    pub fn reparent(&mut self, id: NodeId, new_parent: Option<NodeId>, keep_world: bool) -> Result<(), SceneError> {
        if self.check_move(id, new_parent)? == new_parent {
            return Ok(());
        }
        self.reparent_at(id, new_parent, usize::MAX, keep_world)
    }

    /// Moves `id` (with its subtree) to `index` (clamped, counted once `id` has
    /// been taken out of its old place) among the children of `new_parent`
    /// (`None` = the roots). Also reorders within the same parent.
    pub fn reparent_at(
        &mut self,
        id: NodeId,
        new_parent: Option<NodeId>,
        index: usize,
        keep_world: bool,
    ) -> Result<(), SceneError> {
        self.check_move(id, new_parent)?;
        // Computed before the move; valid after it, because `check_move` ruled out
        // `new_parent` being inside the moved subtree.
        let kept = if keep_world {
            self.transform_keeping_world(id, new_parent)
        } else {
            None
        };
        self.unlink(id).ok_or(SceneError::NodeNotFound)?;
        self.link(id, new_parent, index);
        let revision = self.touch();
        if let Some(node) = self.nodes.get_mut(id) {
            node.revision = revision;
        }
        if let Some(transform) = kept
            && let Some(node) = self.nodes.get_mut(id)
        {
            node.data.transform = transform;
        }
        Ok(())
    }

    /// The local transform that keeps `id` where it is in world space once it is
    /// a child of `new_parent` (`None` = a root). `None` if `id` is not attached.
    ///
    /// Used to build world-preserving [`SceneEdit::Move`](crate::SceneEdit::Move)s.
    #[must_use]
    pub fn transform_keeping_world(&self, id: NodeId, new_parent: Option<NodeId>) -> Option<Transform> {
        self.get(id)?;
        let parent_world = new_parent.map_or(Mat4::IDENTITY, |p| self.world_matrix(p));
        Some(Transform::from_matrix(
            &(parent_world.inverse() * self.world_matrix(id)),
        ))
    }

    /// World matrix of `id` (identity if unknown or detached).
    #[must_use]
    pub fn world_matrix(&self, id: NodeId) -> Mat4 {
        let mut m = Mat4::IDENTITY;
        let mut current = Some(id);
        while let Some(c) = current {
            let Some(node) = self.get(c) else { break };
            m = node.data.transform.matrix() * m;
            current = node.parent;
        }
        m
    }

    /// World matrix of the parent of `id` (identity for roots).
    #[must_use]
    pub fn parent_world_matrix(&self, id: NodeId) -> Mat4 {
        self.get(id)
            .and_then(|n| n.parent)
            .map_or(Mat4::IDENTITY, |p| self.world_matrix(p))
    }

    /// World matrix of `id` with the transforms shown at `frame` (animation and
    /// temporary values; see [`NodeData::transform_at`](crate::NodeData::transform_at)).
    #[must_use]
    pub fn world_matrix_at(&self, id: NodeId, frame: f64) -> Mat4 {
        let mut m = Mat4::IDENTITY;
        let mut current = Some(id);
        while let Some(c) = current {
            let Some(node) = self.get(c) else { break };
            // A layer with Animation off shows its objects unanimated.
            let local = if self.animates(c) {
                node.data.transform_at(frame)
            } else {
                node.data.transform
            };
            m = local.matrix() * m;
            current = node.parent;
        }
        m
    }

    /// World matrix of the parent of `id` at `frame` (identity for roots).
    #[must_use]
    pub fn parent_world_matrix_at(&self, id: NodeId, frame: f64) -> Mat4 {
        self.get(id)
            .and_then(|n| n.parent)
            .map_or(Mat4::IDENTITY, |p| self.world_matrix_at(p, frame))
    }

    /// Drops temporary values made at another frame than `frame` (the time
    /// moved on, so the curves show again). Returns whether any were dropped;
    /// touches nothing otherwise.
    pub fn discard_unkeyed(&mut self, frame: f64) -> bool {
        let stale = |node: &Node| !node.detached && node.data.unkeyed.is_some_and(|u| u.frame != frame);
        if !self.nodes.values().any(stale) {
            return false;
        }
        let revision = self.touch();
        for node in self.nodes.values_mut() {
            if stale(node) {
                node.data.unkeyed = None;
                node.revision = revision;
            }
        }
        true
    }

    /// Depth-first, pre-order traversal in Object Manager order, computing world
    /// matrices on the way down.
    ///
    /// `visit(id, node, world_matrix, depth)` returns whether to descend into the
    /// node's children. The traversal is allocation-free once `stack` has grown to
    /// the scene's peak breadth, which makes it safe for the per-frame hot path.
    /// It starts from the roots, so detached subtrees are never visited.
    pub fn walk<F>(&self, stack: &mut Vec<WalkItem>, mut visit: F)
    where
        F: FnMut(NodeId, &Node, &Mat4, u32) -> bool,
    {
        stack.clear();
        stack.extend(self.roots.iter().rev().map(|&id| WalkItem {
            id,
            parent_world: Mat4::IDENTITY,
            depth: 0,
        }));
        while let Some(item) = stack.pop() {
            let Some(node) = self.nodes.get(item.id) else { continue };
            let world = item.parent_world * node.data.transform.matrix();
            if visit(item.id, node, &world, item.depth) {
                stack.extend(node.children.iter().rev().map(|&id| WalkItem {
                    id,
                    parent_world: world,
                    depth: item.depth + 1,
                }));
            }
        }
    }

    /// World-space bounds of every node for which `include` returns `true`.
    /// Nodes without extent (Nulls) contribute their origin.
    pub fn world_bounds(&self, stack: &mut Vec<WalkItem>, mut include: impl FnMut(NodeId) -> bool) -> Aabb {
        let mut out = Aabb::EMPTY;
        self.walk(stack, |id, node, world, _| {
            if include(id) {
                out = out.union(node.world_bounds(world));
            }
            true
        });
        out
    }

    /// Returns `base` if unused, otherwise the first free `base.N` (C4D naming).
    /// Names of detached nodes count as free.
    #[must_use]
    pub fn unique_name(&self, base: &str) -> String {
        let taken = |name: &str| self.iter().any(|(_, n)| n.data.name == name);
        if !taken(base) {
            return base.to_owned();
        }
        let mut i = 1u32;
        loop {
            let candidate = format!("{base}.{i}");
            if !taken(&candidate) {
                return candidate;
            }
            i += 1;
        }
    }

    /// Validates a move of `id` under `new_parent`; returns the current parent.
    fn check_move(&self, id: NodeId, new_parent: Option<NodeId>) -> Result<Option<NodeId>, SceneError> {
        let old_parent = self.get(id).ok_or(SceneError::NodeNotFound)?.parent;
        if let Some(p) = new_parent {
            if !self.contains(p) {
                return Err(SceneError::NodeNotFound);
            }
            if p == id || self.is_ancestor_of(id, p) {
                return Err(SceneError::WouldCreateCycle);
            }
        }
        Ok(old_parent)
    }

    fn check_detached_root(&self, root: NodeId) -> Result<(), SceneError> {
        let node = self.nodes.get(root).ok_or(SceneError::NodeNotFound)?;
        if node.detached && node.parent.is_none() {
            Ok(())
        } else {
            Err(SceneError::NotDetached)
        }
    }

    /// Inserts `id` into the sibling list of `parent` at `index` (clamped) and
    /// sets its parent link. `parent` must be attached (checked by callers).
    fn link(&mut self, id: NodeId, parent: Option<NodeId>, index: usize) {
        let siblings = match parent {
            Some(p) => match self.nodes.get_mut(p) {
                Some(node) => &mut node.children,
                None => return,
            },
            None => &mut self.roots,
        };
        siblings.insert(index.min(siblings.len()), id);
        if let Some(node) = self.nodes.get_mut(id) {
            node.parent = parent;
        }
    }

    /// Takes `id` out of its sibling list and clears its parent link.
    /// Returns where it was, or `None` if it was not linked.
    fn unlink(&mut self, id: NodeId) -> Option<(Option<NodeId>, usize)> {
        let parent = self.nodes.get(id)?.parent;
        let siblings = match parent {
            Some(p) => &mut self.nodes.get_mut(p)?.children,
            None => &mut self.roots,
        };
        let index = siblings.iter().position(|&c| c == id)?;
        siblings.remove(index);
        if let Some(node) = self.nodes.get_mut(id) {
            node.parent = None;
        }
        Some((parent, index))
    }

    /// Sets the detached flag on `root` and its descendants; returns how many.
    fn set_subtree_detached(&mut self, root: NodeId, detached: bool) -> usize {
        let mut count = 0;
        let mut pending = vec![root];
        while let Some(current) = pending.pop() {
            if let Some(node) = self.nodes.get_mut(current) {
                node.detached = detached;
                count += 1;
                pending.extend_from_slice(&node.children);
            }
        }
        count
    }
}

/// A node about to be stored: hierarchy links are the scene's business.
fn fresh(mut node: Node) -> Node {
    node.parent = None;
    node.children.clear();
    node.detached = false;
    node.revision = next_revision();
    node
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use glam::Vec3;

    use super::*;
    use crate::{ObjectKind, test_util::Rng};

    fn null(name: &str) -> Node {
        Node::new(name, ObjectKind::Null)
    }

    /// Checks every structural invariant documented on [`Scene`].
    fn assert_invariants(s: &Scene) {
        let mut attached = HashSet::new();
        let mut stack: Vec<(NodeId, Option<NodeId>)> = s.roots.iter().map(|&r| (r, None)).collect();
        while let Some((id, parent)) = stack.pop() {
            let node = s.nodes.get(id).expect("dangling link in the hierarchy");
            assert!(!node.detached, "the hierarchy reaches a detached node");
            assert_eq!(node.parent, parent, "parent link does not match the children list");
            assert!(attached.insert(id), "node reached twice");
            stack.extend(node.children.iter().map(|&c| (c, Some(id))));
        }
        assert_eq!(attached.len(), s.len(), "len() disagrees with the hierarchy");
        assert_eq!(attached.len(), s.iter().count(), "iter() disagrees with the hierarchy");

        let mut detached = HashSet::new();
        for &root in &s.detached_roots {
            assert!(s.nodes.get(root).expect("dangling detached root").parent.is_none());
            stack.push((root, None));
            while let Some((id, parent)) = stack.pop() {
                let node = s.nodes.get(id).expect("dangling link in a detached subtree");
                assert!(node.detached, "detached subtree contains an attached node");
                assert!(!s.contains(id), "detached node visible to queries");
                assert_eq!(node.parent, parent);
                assert!(detached.insert(id), "detached node reached twice");
                stack.extend(node.children.iter().map(|&c| (c, Some(id))));
            }
        }
        assert_eq!(detached.len(), s.detached_count, "detached_count is off");
        assert_eq!(
            attached.len() + detached.len(),
            s.nodes.len(),
            "orphaned nodes in the arena"
        );
    }

    #[test]
    fn remove_deletes_subtree() {
        let mut s = Scene::new();
        let a = s.insert(null("a"));
        let b = s.insert_child(a, null("b")).unwrap();
        let _c = s.insert_child(b, null("c")).unwrap();
        let d = s.insert(null("d"));
        assert_eq!(s.remove(a), 3);
        assert_eq!(s.len(), 1);
        assert_eq!(s.roots(), &[d]);
        assert!(s.detached_roots().is_empty(), "remove must not leave detached nodes");
        assert_invariants(&s);
    }

    #[test]
    fn reparent_rejects_cycles() {
        let mut s = Scene::new();
        let a = s.insert(null("a"));
        let b = s.insert_child(a, null("b")).unwrap();
        assert_eq!(s.reparent(a, Some(b), false), Err(SceneError::WouldCreateCycle));
        assert_eq!(s.reparent(a, Some(a), false), Err(SceneError::WouldCreateCycle));
        assert_eq!(s.reparent(b, None, false), Ok(()));
        assert_eq!(s.roots(), &[a, b]);
        assert_invariants(&s);
    }

    #[test]
    fn reparent_keeps_world_position() {
        let mut s = Scene::new();
        let parent = s.insert(null("p").with_position(Vec3::new(100.0, 0.0, 0.0)));
        let child = s.insert(null("c").with_position(Vec3::new(150.0, 20.0, 0.0)));
        s.reparent(child, Some(parent), true).unwrap();
        let local = s.get(child).unwrap().data.transform.position;
        assert!(local.abs_diff_eq(Vec3::new(50.0, 20.0, 0.0), 1e-3), "{local:?}");
        let world = s.world_matrix(child).w_axis.truncate();
        assert!(world.abs_diff_eq(Vec3::new(150.0, 20.0, 0.0), 1e-3));
    }

    #[test]
    fn walk_is_preorder_in_manager_order() {
        let mut s = Scene::new();
        let a = s.insert(null("a"));
        let a1 = s.insert_child(a, null("a1")).unwrap();
        let a2 = s.insert_child(a, null("a2")).unwrap();
        let b = s.insert(null("b"));
        let mut order = Vec::new();
        let mut stack = Vec::new();
        s.walk(&mut stack, |id, _, _, depth| {
            order.push((id, depth));
            true
        });
        assert_eq!(order, vec![(a, 0), (a1, 1), (a2, 1), (b, 0)]);
    }

    #[test]
    fn unique_names_follow_c4d_style() {
        let mut s = Scene::new();
        assert_eq!(s.unique_name("Cube"), "Cube");
        s.insert(null("Cube"));
        assert_eq!(s.unique_name("Cube"), "Cube.1");
        s.insert(null("Cube.1"));
        assert_eq!(s.unique_name("Cube"), "Cube.2");
    }

    #[test]
    fn detach_hides_subtree_and_attach_restores_the_same_ids() {
        let mut s = Scene::new();
        let a = s.insert(null("a"));
        let g = s.insert(null("g"));
        let g1 = s.insert_child(g, null("g1")).unwrap();
        let g2 = s.insert_child(g, null("g2")).unwrap();
        let b = s.insert(null("b"));

        let info = s.detach(g).unwrap();
        assert_eq!(
            info,
            Detached {
                root: g,
                parent: None,
                index: 1
            }
        );
        assert_eq!(s.len(), 2);
        assert_eq!(s.roots(), &[a, b]);
        for id in [g, g1, g2] {
            assert!(!s.contains(id) && s.get(id).is_none());
        }
        assert!(s.children(g).is_empty());
        assert_eq!(s.world_matrix(g1), Mat4::IDENTITY);
        let mut visited = 0;
        s.walk(&mut Vec::new(), |_, _, _, _| {
            visited += 1;
            true
        });
        assert_eq!(visited, 2, "walk must skip detached subtrees");
        assert_invariants(&s);

        s.attach(info.root, info.parent, info.index).unwrap();
        assert_eq!(s.roots(), &[a, g, b], "restored at its old position");
        assert_eq!(s.children(g), &[g1, g2], "child order survives");
        assert_eq!(s.len(), 5);
        assert!(s.detached_roots().is_empty());
        assert_invariants(&s);
    }

    #[test]
    fn detach_of_a_child_restores_under_its_parent() {
        let mut s = Scene::new();
        let p = s.insert(null("p"));
        let c0 = s.insert_child(p, null("c0")).unwrap();
        let c1 = s.insert_child(p, null("c1")).unwrap();
        let c2 = s.insert_child(p, null("c2")).unwrap();
        let info = s.detach(c1).unwrap();
        assert_eq!((info.parent, info.index), (Some(p), 1));
        assert_eq!(s.children(p), &[c0, c2]);
        assert!(s.get(c1).is_none());
        s.attach(c1, info.parent, info.index).unwrap();
        assert_eq!(s.children(p), &[c0, c1, c2]);
        assert_eq!(s.get(c1).unwrap().parent(), Some(p));
        assert_invariants(&s);
    }

    #[test]
    fn attach_and_purge_validate_their_arguments() {
        let mut s = Scene::new();
        let a = s.insert(null("a"));
        let g = s.insert(null("g"));
        let g1 = s.insert_child(g, null("g1")).unwrap();
        s.detach(g).unwrap();

        assert_eq!(s.attach(a, None, 0), Err(SceneError::NotDetached), "attached node");
        assert_eq!(
            s.attach(g1, None, 0),
            Err(SceneError::NotDetached),
            "inside a detached subtree"
        );
        assert_eq!(s.attach(g, Some(g1), 0), Err(SceneError::NodeNotFound), "into itself");
        assert_eq!(s.purge(a), Err(SceneError::NotDetached));
        assert_eq!(s.detach(g), Err(SceneError::NodeNotFound), "already detached");
        assert_eq!(s.detach(g1), Err(SceneError::NodeNotFound));
        assert_eq!(s.reparent(g1, None, false), Err(SceneError::NodeNotFound));
        assert_eq!(s.reparent(a, Some(g), false), Err(SceneError::NodeNotFound));
        assert_eq!(s.insert_at(Some(g), 0, null("x")), Err(SceneError::NodeNotFound));
        assert_invariants(&s);

        // Attaching under another node, at a clamped index.
        s.attach(g, Some(a), 99).unwrap();
        assert_eq!(s.children(a), &[g]);
        assert_invariants(&s);
    }

    #[test]
    fn purge_frees_detached_subtrees() {
        let mut s = Scene::new();
        let g = s.insert(null("g"));
        let g1 = s.insert_child(g, null("g1")).unwrap();
        let h = s.insert(null("h"));
        s.detach(g).unwrap();
        s.detach(h).unwrap();
        assert_eq!(s.detached_roots(), &[g, h]);
        assert_eq!(s.purge(g), Ok(2));
        assert_eq!(s.attach(g1, None, 0), Err(SceneError::NodeNotFound), "freed");
        assert_invariants(&s);
        assert_eq!(s.purge_all_detached(), 1);
        assert!(s.is_empty());
        assert_eq!(s.nodes.len(), 0, "nothing left in the arena");
        assert_invariants(&s);
    }

    #[test]
    fn world_matrices_at_a_frame_and_dropping_temporary_values() {
        use crate::{Axis, Curve, Key, Param, Track};
        let mut s = Scene::new();
        let mut parent = null("parent");
        parent.data.tracks.push(Track {
            param: Param::Position(Axis::X),
            curve: Curve::from_keys([Key::new(0.0, 0.0), Key::new(10.0, 100.0)]),
        });
        let parent = s.insert(parent);
        let child = s.insert_child(parent, null("child").with_position(Vec3::Y)).unwrap();
        let at = |s: &Scene, frame| s.world_matrix_at(child, frame).w_axis.truncate();
        assert_eq!(at(&s, 10.0), Vec3::new(100.0, 1.0, 0.0), "the child follows the curve");
        assert_eq!(s.parent_world_matrix_at(child, 10.0).w_axis.x, 100.0);
        assert_eq!(s.world_matrix(child).w_axis.x, 0.0, "the document's own values");

        let data = &mut s.get_mut(parent).unwrap().data;
        let mut shown = data.transform_at(3.0);
        shown.position.x = -50.0;
        data.set_transform_at(&shown, 3.0, false);
        assert_eq!(at(&s, 3.0).x, -50.0);
        let revision = s.revision();
        assert!(!s.discard_unkeyed(3.0), "still the frame it was made at");
        assert_eq!(s.revision(), revision, "nothing touched");
        assert!(s.discard_unkeyed(4.0));
        assert_eq!(s.get(parent).unwrap().data.unkeyed, None);
        let curve = s.get(parent).unwrap().data.tracks[0].curve.value_at(3.0).unwrap();
        assert_eq!(at(&s, 3.0).x, curve, "the curve again");
    }

    #[test]
    fn materials_are_inherited_and_changes_bump_the_revision() {
        use crate::{Material, Tag};
        let mut s = Scene::new();
        let revision = s.revision();
        let red = s.materials_mut().insert(Material::new("Red"));
        assert_ne!(s.revision(), revision, "a material change is a scene change");
        let group = s.insert(null("group"));
        let child = s.insert_child(group, null("child")).unwrap();
        let grandchild = s.insert_child(child, null("grandchild")).unwrap();
        assert_eq!(s.effective_material(grandchild), None);
        s.get_mut(group)
            .unwrap()
            .data
            .tags
            .push(Tag::Material(crate::MaterialTag::new(Some(red))));
        assert_eq!(s.effective_material(grandchild), Some(red), "inherited");
        let blue = s.materials_mut().insert(Material::new("Blue"));
        s.get_mut(child)
            .unwrap()
            .data
            .tags
            .push(Tag::Material(crate::MaterialTag::new(Some(blue))));
        assert_eq!(s.effective_material(grandchild), Some(blue), "the nearest wins");
        assert_eq!(s.effective_material(group), Some(red));
        s.materials_mut().remove(blue);
        assert_eq!(
            s.effective_material(grandchild),
            Some(red),
            "a missing material counts as none"
        );
    }

    #[test]
    fn detached_names_are_free_again() {
        let mut s = Scene::new();
        let cube = s.insert(null("Cube"));
        s.detach(cube).unwrap();
        assert_eq!(s.unique_name("Cube"), "Cube");
    }

    #[test]
    fn insert_at_and_reparent_at_respect_order() {
        let mut s = Scene::new();
        let a = s.insert(null("a"));
        let c = s.insert(null("c"));
        let b = s.insert_at(None, 1, null("b")).unwrap();
        let first = s.insert_at(None, 0, null("first")).unwrap();
        assert_eq!(s.roots(), &[first, a, b, c]);
        assert_eq!(s.index_in_parent(b), Some(2));

        // Reorder within the same parent: `first` moves to the end.
        s.reparent_at(first, None, usize::MAX, false).unwrap();
        assert_eq!(s.roots(), &[a, b, c, first]);
        s.reparent_at(c, None, 0, false).unwrap();
        assert_eq!(s.roots(), &[c, a, b, first]);

        // Into another parent at a given position.
        let k0 = s.insert_child(a, null("k0")).unwrap();
        s.reparent_at(b, Some(a), 0, false).unwrap();
        assert_eq!(s.children(a), &[b, k0]);
        assert_eq!(s.index_in_parent(k0), Some(1));
        assert_eq!(s.roots(), &[c, a, first]);
        assert_invariants(&s);
    }

    #[test]
    fn reparent_to_the_current_parent_is_a_no_op() {
        let mut s = Scene::new();
        let p = s.insert(null("p"));
        let x = s.insert_child(p, null("x")).unwrap();
        let y = s.insert_child(p, null("y")).unwrap();
        s.reparent(x, Some(p), false).unwrap();
        assert_eq!(s.children(p), &[x, y], "reparent does not reorder");
    }

    #[test]
    fn transform_keeping_world_preserves_placement() {
        let mut s = Scene::new();
        let parent = s.insert(null("p").with_transform(
            Transform::from_position(Vec3::new(10.0, 0.0, 0.0)).with_rotation_deg(Vec3::new(90.0, 0.0, 0.0)),
        ));
        let child = s.insert(null("c").with_position(Vec3::new(0.0, 5.0, -30.0)));
        let world_before = s.world_matrix(child);
        let local = s.transform_keeping_world(child, Some(parent)).unwrap();
        s.reparent(child, Some(parent), false).unwrap();
        s.get_mut(child).unwrap().data.transform = local;
        assert!(s.world_matrix(child).abs_diff_eq(world_before, 1e-3));
        s.detach(child).unwrap();
        assert_eq!(s.transform_keeping_world(child, None), None, "detached");
    }

    #[test]
    fn reading_never_changes_the_revision_and_writing_always_does() {
        let mut s = Scene::new();
        let a = s.insert(null("a"));
        let b = s.insert_child(a, null("b")).unwrap();

        let r = s.revision();
        let _ = (s.get(a), s.children(a), s.iter().count(), s.world_matrix(b), s.len());
        s.walk(&mut Vec::new(), |_, _, _, _| true);
        assert_eq!(s.revision(), r, "read-only access");

        let node_before = s.get(b).unwrap().revision();
        s.get_mut(b).unwrap().data.name = "renamed".to_owned();
        assert!(s.revision() > r);
        assert!(s.get(b).unwrap().revision() > node_before, "the node is marked too");
        assert_eq!(s.get_mut(NodeId::default()).map(|_| ()), None);

        let mut last = s.revision();
        let mut changed = |s: &Scene, what: &str| {
            assert!(s.revision() > last, "{what} must change the revision");
            last = s.revision();
        };
        s.insert(null("c"));
        changed(&s, "insert");
        let d = s.detach(b).unwrap();
        changed(&s, "detach");
        s.attach(b, d.parent, d.index).unwrap();
        changed(&s, "attach");
        s.reparent_at(b, None, 0, true).unwrap();
        changed(&s, "reparent");
        s.detach(b).unwrap();
        s.purge(b).unwrap();
        changed(&s, "purge");
    }

    #[test]
    fn revisions_are_unique_across_scenes() {
        let first = Scene::new();
        let second = Scene::new();
        assert_ne!(
            first.revision(),
            second.revision(),
            "a replaced document never looks unchanged"
        );
        assert_eq!(first.clone().revision(), first.revision(), "a copy is the same content");
    }

    #[test]
    fn random_edit_sequences_keep_every_invariant() {
        for seed in 1..=8u64 {
            let mut rng = Rng::new(seed);
            let mut s = Scene::new();
            for step in 0..400 {
                let attached: Vec<NodeId> = s.iter().map(|(id, _)| id).collect();
                let detached = s.detached_roots().to_vec();
                let parent = if rng.chance(4) { None } else { rng.pick(&attached) };
                let index = rng.below(6);
                match rng.below(7) {
                    0 | 1 => {
                        s.insert_at(parent, index, null("n")).unwrap();
                    }
                    2 => {
                        if let Some(id) = rng.pick(&attached) {
                            let info = s.detach(id).unwrap();
                            assert_eq!(info.root, id);
                        }
                    }
                    3 => {
                        if let Some(root) = rng.pick(&detached) {
                            let size_before = s.len();
                            s.attach(root, parent, index).unwrap();
                            assert!(s.contains(root));
                            assert!(s.len() > size_before);
                        }
                    }
                    4 => {
                        if let Some(root) = rng.pick(&detached) {
                            assert!(s.purge(root).unwrap() >= 1);
                        }
                    }
                    5 => {
                        if let Some(id) = rng.pick(&attached) {
                            match s.reparent_at(id, parent, index, rng.chance(2)) {
                                Ok(()) => assert_eq!(s.get(id).unwrap().parent(), parent),
                                Err(e) => assert_eq!(e, SceneError::WouldCreateCycle),
                            }
                        }
                    }
                    _ => {
                        if let Some(id) = rng.pick(&attached) {
                            s.remove(id);
                            assert!(!s.contains(id));
                        }
                    }
                }
                assert_invariants(&s);
                if step % 100 == 99 {
                    s.purge_all_detached();
                    assert!(s.detached_roots().is_empty());
                    assert_invariants(&s);
                }
            }
        }
    }
}
