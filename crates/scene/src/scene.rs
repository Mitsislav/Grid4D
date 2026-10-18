use std::fmt;

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
#[derive(Clone, Debug, Default)]
pub struct Scene {
    nodes: SlotMap<NodeId, Node>,
    roots: Vec<NodeId>,
    /// Roots of the subtrees removed with `detach`, kept alive until `purge`.
    detached_roots: Vec<NodeId>,
    /// Number of nodes, across all detached subtrees, hidden from public queries.
    detached_count: usize,
}

impl Scene {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id).filter(|n| !n.detached)
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
        let world = keep_world.then(|| self.world_matrix(id));
        self.unlink(id).ok_or(SceneError::NodeNotFound)?;
        self.link(id, new_parent, index);

        if let Some(world) = world {
            let parent_world = new_parent.map_or(Mat4::IDENTITY, |p| self.world_matrix(p));
            let local = parent_world.inverse() * world;
            if let Some(node) = self.nodes.get_mut(id) {
                node.data.transform = Transform::from_matrix(&local);
            }
        }
        Ok(())
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
    node
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use glam::Vec3;

    use super::*;
    use crate::ObjectKind;

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

    /// Minimal deterministic xorshift generator, so the test needs no dependency.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            usize::try_from(self.0 % u64::try_from(n.max(1)).unwrap()).unwrap()
        }

        fn pick<T: Copy>(&mut self, items: &[T]) -> Option<T> {
            (!items.is_empty()).then(|| items[self.below(items.len())])
        }
    }

    #[test]
    fn random_edit_sequences_keep_every_invariant() {
        for seed in 1..=8u64 {
            let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ seed);
            let mut s = Scene::new();
            for step in 0..400 {
                let attached: Vec<NodeId> = s.iter().map(|(id, _)| id).collect();
                let detached = s.detached_roots().to_vec();
                let parent = if rng.below(4) == 0 { None } else { rng.pick(&attached) };
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
                            match s.reparent_at(id, parent, index, rng.below(2) == 0) {
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
