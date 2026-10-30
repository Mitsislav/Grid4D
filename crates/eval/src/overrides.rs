//! World matrices that replace what animation gives some objects — a
//! simulation's poses (see `docs/reference/simulation-cache.md`). Evaluation
//! applies them after animation and Target tags; children follow.

use std::collections::HashMap;

use glam::Mat4;
use grid4d_scene::NodeId;

use crate::EvaluatedNode;

/// Objects placed by something other than their animation, by id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overrides {
    nodes: HashMap<NodeId, Mat4>,
}

impl Overrides {
    /// No objects (keeps the capacity).
    pub fn clear(&mut self) {
        self.nodes.clear();
    }

    /// Places `id` at `world`.
    pub fn set(&mut self, id: NodeId, world: Mat4) {
        self.nodes.insert(id, world);
    }

    /// Where `id` is placed, if it is.
    #[must_use]
    pub fn get(&self, id: NodeId) -> Option<Mat4> {
        self.nodes.get(&id).copied()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
}

/// Moves the overridden nodes (pre-order) to their matrices, and their
/// subtrees with them. `stack` is scratch space.
pub(crate) fn apply(overrides: &Overrides, nodes: &mut [EvaluatedNode], stack: &mut Vec<(u32, Mat4)>) {
    if overrides.is_empty() {
        return;
    }
    stack.clear();
    for node in nodes {
        while stack.last().is_some_and(|&(d, _)| d >= node.depth) {
            stack.pop();
        }
        // How the nearest moved ancestor moved.
        let inherited = stack.last().map(|&(_, delta)| delta);
        let own = overrides.get(node.id);
        if own.is_none() && inherited.is_none() {
            continue;
        }
        let before = node.world;
        node.world = own.unwrap_or_else(|| inherited.unwrap_or(Mat4::IDENTITY) * before);
        let delta = if before.determinant().abs() > f32::EPSILON {
            node.world * before.inverse()
        } else {
            inherited.unwrap_or(Mat4::IDENTITY)
        };
        stack.push((node.depth, delta));
    }
}
