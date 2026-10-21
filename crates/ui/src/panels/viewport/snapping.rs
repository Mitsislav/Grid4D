//! Snapping during viewport drags (see `docs/reference/modeling-snapping.md`):
//! the Move handles and Tweak keep the position the element would have without
//! snapping and ask a [`Snapper`] where it goes instead. Targets are looked for
//! around the cursor, so the user points at what to snap to.

use egui::{Pos2, Rect};
use glam::{Vec2 as GVec2, Vec3};
use grid4d_eval::EvaluatedScene;
use grid4d_geom::BitSet;
use grid4d_render::{ScreenView, SnapCache, SnapHit, SnapQuery, ViewFilter, find_snap, snapped_position};
use grid4d_scene::{Node, NodeId, Scene};

use crate::SnapSettings;

/// Snapping for this frame of a drag: the settings and what the viewport shows.
pub(super) struct Snapper<'a> {
    pub settings: SnapSettings,
    pub scene: &'a EvaluatedScene,
    pub view: ScreenView,
    pub rect: Rect,
    pub cache: &'a mut SnapCache,
    /// The floor grid's current spacing.
    pub grid_spacing: f32,
    /// The object types the view shows: hidden ones offer no targets.
    pub shown: ViewFilter,
    /// The target of the last snap, for the viewport to show.
    pub hit: Option<SnapHit>,
}

impl Snapper<'_> {
    /// Where an element that would go to `wanted` (world) goes: onto the target
    /// near the `cursor` (screen) — along `line` (start, unit direction) for a
    /// constrained move — or to `wanted` when snapping is off or nothing is near.
    /// `skip` and `moving` are what moves with the drag (see [`SnapQuery`]).
    pub(super) fn snap(
        &mut self,
        cursor: Pos2,
        wanted: Vec3,
        line: Option<(Vec3, Vec3)>,
        skip: &[NodeId],
        moving: Option<(NodeId, &BitSet)>,
    ) -> Vec3 {
        self.hit = None;
        if !self.settings.enabled {
            return wanted;
        }
        let query = SnapQuery {
            at: GVec2::new(cursor.x - self.rect.left(), cursor.y - self.rect.top()),
            radius: self.settings.radius,
            targets: self.settings.targets,
            grid_spacing: self.grid_spacing,
            skip,
            moving,
            shown: self.shown,
        };
        let Some(hit) = find_snap(self.scene, &self.view, self.cache, &query) else {
            return wanted;
        };
        self.hit = Some(hit);
        snapped_position(&self.view, &hit, line, wanted, self.settings.kind.in_space())
    }
}

/// The generators above `id`: their result changes when anything below them
/// moves, so they cannot be snap targets during the drag.
pub(super) fn generators_above(scene: &Scene, id: NodeId) -> Vec<NodeId> {
    let mut found = Vec::new();
    let mut at = scene.get(id).and_then(Node::parent);
    while let Some(parent) = at {
        let Some(node) = scene.get(parent) else { break };
        if node.data.kind.is_generator() {
            found.push(parent);
        }
        at = node.parent();
    }
    found
}

#[cfg(test)]
mod tests {
    use grid4d_scene::{Document, Node, ObjectKind, ObjectKindTag};

    use super::*;

    #[test]
    fn generators_above_an_object_are_found_up_the_hierarchy() {
        let mut doc = Document::new("snap");
        let sds = doc.scene.insert(Node::new(
            "Subdivision Surface",
            ObjectKindTag::SubdivisionSurface.default_kind(),
        ));
        let group = doc
            .scene
            .insert_child(sds, Node::new("Group", ObjectKind::Null))
            .unwrap();
        let cube = doc
            .scene
            .insert_child(group, Node::new("Cube", ObjectKindTag::Cube.default_kind()))
            .unwrap();
        assert_eq!(generators_above(&doc.scene, cube), [sds]);
        assert_eq!(generators_above(&doc.scene, group), [sds]);
        assert!(generators_above(&doc.scene, sds).is_empty());
    }
}
