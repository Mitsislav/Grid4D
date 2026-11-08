//! World matrices that replace what animation gives some objects — a
//! simulation's poses (see `docs/reference/simulation-cache.md`). Evaluation
//! applies them after animation and Target tags; children follow.

use std::collections::HashMap;

use glam::{Mat4, Quat};

/// The placement `t` of the way (0 – 1) from `a` to `b`: scale and position
/// straight, rotation along the shorter arc.
#[must_use]
pub fn interpolate(a: Mat4, b: Mat4, t: f32) -> Mat4 {
    let (s0, r0, p0) = a.to_scale_rotation_translation();
    let (s1, r1, p1) = b.to_scale_rotation_translation();
    let r1 = if r0.dot(r1) < 0.0 { -r1 } else { r1 };
    Mat4::from_scale_rotation_translation(s0.lerp(s1, t), Quat::slerp(r0, r1, t), p0.lerp(p1, t))
}
use grid4d_scene::{Aabb, NodeId};

use crate::{EvaluatedInstance, EvaluatedNode};

/// Objects placed by something other than their animation, by id; clones
/// moved by a matrix, by their Cloner and number.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overrides {
    nodes: HashMap<NodeId, Mat4>,
    clones: HashMap<(NodeId, u32), Mat4>,
}

impl Overrides {
    /// No objects (keeps the capacity).
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.clones.clear();
    }

    /// Moves clone `index` of `cloner` (all its parts) by `delta`, in world space.
    pub fn set_clone(&mut self, cloner: NodeId, index: u32, delta: Mat4) {
        self.clones.insert((cloner, index), delta);
    }

    /// How clone `index` of `cloner` is moved, if it is.
    #[must_use]
    pub fn clone_delta(&self, cloner: NodeId, index: u32) -> Option<Mat4> {
        self.clones.get(&(cloner, index)).copied()
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

    /// Moves every placement `t` of the way (0 – 1) towards `other`'s for
    /// the same object or clone (those `other` lacks stay): a pose between
    /// two cached frames.
    pub fn blend_towards(&mut self, other: &Self, t: f32) {
        for (id, world) in &mut self.nodes {
            if let Some(&to) = other.nodes.get(id) {
                *world = interpolate(*world, to, t);
            }
        }
        for (key, delta) in &mut self.clones {
            if let Some(&to) = other.clones.get(key) {
                *delta = interpolate(*delta, to, t);
            }
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.clones.is_empty()
    }

    /// Objects and clones placed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len() + self.clones.len()
    }
}

/// Moves the overridden clones by their deltas, then gives every Cloner with
/// a moved clone the bounds of its clones again.
pub(crate) fn apply_clones(overrides: &Overrides, instances: &mut [EvaluatedInstance], nodes: &mut [EvaluatedNode]) {
    if overrides.clones.is_empty() {
        return;
    }
    let mut start = 0;
    while let Some(first) = instances.get(start) {
        let owner = first.owner;
        let end = start + instances[start..].iter().take_while(|c| c.owner == owner).count();
        let group = &mut instances[start..end];
        start = end;
        let mut moved = false;
        for clone in group.iter_mut() {
            if let Some(delta) = overrides.clone_delta(owner, clone.index) {
                clone.world = delta * clone.world;
                moved = true;
            }
        }
        if !moved {
            continue;
        }
        let Some(node) = nodes.iter_mut().find(|n| n.id == owner) else {
            continue;
        };
        let inverse = node.world.inverse();
        node.bounds = group.iter().fold(None, |all: Option<Aabb>, clone| {
            let Some(b) = clone.bounds.filter(|b| !b.is_empty()) else {
                return all;
            };
            let b = b.transformed(&(inverse * clone.world));
            Some(all.map_or(b, |all| all.union(b)))
        });
    }
}

/// Moves the overridden nodes (pre-order) to their matrices, and their
/// subtrees with them. `stack` is scratch space.
pub(crate) fn apply(overrides: &Overrides, nodes: &mut [EvaluatedNode], stack: &mut Vec<(u32, Mat4)>) {
    if overrides.nodes.is_empty() {
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
