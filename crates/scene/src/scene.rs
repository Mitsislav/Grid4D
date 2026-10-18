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
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NodeNotFound => f.write_str("node not found in scene"),
            Self::WouldCreateCycle => f.write_str("a node cannot be parented under itself or its descendants"),
        }
    }
}

impl std::error::Error for SceneError {}

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
#[derive(Clone, Debug, Default)]
pub struct Scene {
    nodes: SlotMap<NodeId, Node>,
    roots: Vec<NodeId>,
}

impl Scene {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    #[must_use]
    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(id)
    }

    #[must_use]
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id)
    }

    /// Top-level nodes, in Object Manager order.
    #[must_use]
    pub fn roots(&self) -> &[NodeId] {
        &self.roots
    }

    /// Children of `id` in order (empty if `id` is unknown).
    #[must_use]
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.nodes.get(id).map_or(&[], |n| n.children.as_slice())
    }

    /// All nodes in storage order (not hierarchy order — use [`Scene::walk`] for that).
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &Node)> + '_ {
        self.nodes.iter()
    }

    /// Inserts `node` as the last root.
    pub fn insert(&mut self, mut node: Node) -> NodeId {
        node.parent = None;
        node.children.clear();
        let id = self.nodes.insert(node);
        self.roots.push(id);
        id
    }

    /// Inserts `node` as the last child of `parent`.
    pub fn insert_child(&mut self, parent: NodeId, mut node: Node) -> Result<NodeId, SceneError> {
        if !self.nodes.contains_key(parent) {
            return Err(SceneError::NodeNotFound);
        }
        node.parent = Some(parent);
        node.children.clear();
        let id = self.nodes.insert(node);
        if let Some(p) = self.nodes.get_mut(parent) {
            p.children.push(id);
        }
        Ok(id)
    }

    /// Removes `id` and its whole subtree. Returns the number of nodes removed.
    pub fn remove(&mut self, id: NodeId) -> usize {
        let Some(node) = self.nodes.get(id) else {
            return 0;
        };
        let parent = node.parent;
        self.detach(id, parent);

        let mut removed = 0;
        let mut pending = vec![id];
        while let Some(current) = pending.pop() {
            if let Some(node) = self.nodes.remove(current) {
                removed += 1;
                pending.extend(node.children);
            }
        }
        removed
    }

    /// `true` if `ancestor` is a (direct or indirect) parent of `node`.
    #[must_use]
    pub fn is_ancestor_of(&self, ancestor: NodeId, node: NodeId) -> bool {
        let mut current = self.nodes.get(node).and_then(|n| n.parent);
        while let Some(c) = current {
            if c == ancestor {
                return true;
            }
            current = self.nodes.get(c).and_then(|n| n.parent);
        }
        false
    }

    /// Moves `id` (with its subtree) under `new_parent` (`None` = make it a root).
    ///
    /// With `keep_world = true` the node keeps its world-space placement, like
    /// dragging objects in Cinema 4D's Object Manager.
    pub fn reparent(&mut self, id: NodeId, new_parent: Option<NodeId>, keep_world: bool) -> Result<(), SceneError> {
        let old_parent = self.nodes.get(id).ok_or(SceneError::NodeNotFound)?.parent;
        if let Some(p) = new_parent {
            if !self.nodes.contains_key(p) {
                return Err(SceneError::NodeNotFound);
            }
            if p == id || self.is_ancestor_of(id, p) {
                return Err(SceneError::WouldCreateCycle);
            }
        }
        if old_parent == new_parent {
            return Ok(());
        }

        let world = keep_world.then(|| self.world_matrix(id));
        self.detach(id, old_parent);
        match new_parent {
            Some(p) => {
                if let Some(parent) = self.nodes.get_mut(p) {
                    parent.children.push(id);
                }
            }
            None => self.roots.push(id),
        }
        if let Some(node) = self.nodes.get_mut(id) {
            node.parent = new_parent;
        }

        if let Some(world) = world {
            let parent_world = new_parent.map_or(Mat4::IDENTITY, |p| self.world_matrix(p));
            let local = parent_world.inverse() * world;
            if let Some(node) = self.nodes.get_mut(id) {
                node.transform = Transform::from_matrix(&local);
            }
        }
        Ok(())
    }

    /// World matrix of `id` (identity if unknown).
    #[must_use]
    pub fn world_matrix(&self, id: NodeId) -> Mat4 {
        let mut m = Mat4::IDENTITY;
        let mut current = Some(id);
        while let Some(c) = current {
            let Some(node) = self.nodes.get(c) else { break };
            m = node.transform.matrix() * m;
            current = node.parent;
        }
        m
    }

    /// World matrix of the parent of `id` (identity for roots).
    #[must_use]
    pub fn parent_world_matrix(&self, id: NodeId) -> Mat4 {
        self.nodes
            .get(id)
            .and_then(|n| n.parent)
            .map_or(Mat4::IDENTITY, |p| self.world_matrix(p))
    }

    /// Depth-first, pre-order traversal in Object Manager order, computing world
    /// matrices on the way down.
    ///
    /// `visit(id, node, world_matrix, depth)` returns whether to descend into the
    /// node's children. The traversal is allocation-free once `stack` has grown to
    /// the scene's peak breadth, which makes it safe for the per-frame hot path.
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
            let world = item.parent_world * node.transform.matrix();
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
    #[must_use]
    pub fn unique_name(&self, base: &str) -> String {
        let taken = |name: &str| self.nodes.values().any(|n| n.name == name);
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

    fn detach(&mut self, id: NodeId, parent: Option<NodeId>) {
        let siblings = match parent {
            Some(p) => match self.nodes.get_mut(p) {
                Some(node) => &mut node.children,
                None => return,
            },
            None => &mut self.roots,
        };
        siblings.retain(|&c| c != id);
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::ObjectKind;

    fn null(name: &str) -> Node {
        Node::new(name, ObjectKind::Null)
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
    }

    #[test]
    fn reparent_keeps_world_position() {
        let mut s = Scene::new();
        let parent = s.insert(null("p").with_position(Vec3::new(100.0, 0.0, 0.0)));
        let child = s.insert(null("c").with_position(Vec3::new(150.0, 20.0, 0.0)));
        s.reparent(child, Some(parent), true).unwrap();
        let local = s.get(child).unwrap().transform.position;
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
}
