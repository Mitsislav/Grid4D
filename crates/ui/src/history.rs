//! Undo/redo history: groups of reversible [`SceneEdit`]s plus the selection
//! around them. Editor state, not document state — a saved file has no history.
//!
//! `commands::apply` is the only writer: it applies a command's edits with
//! [`Document::apply_all`] and records the returned inverses as one
//! [`UndoGroup`]. Undo applies a group's edits and stores what `apply_all`
//! returns as the redo edits, and vice versa.

use grid4d_scene::{Document, SceneEdit, SceneError};

use crate::Selection;

/// One undo step ("Undo Delete").
#[derive(Clone, Debug)]
pub struct UndoGroup {
    /// Identifies the document state *after* this group (see [`History::state_id`]).
    id: u64,
    label: &'static str,
    /// Edits that move the document out of the state this group represents:
    /// the inverses while on the undo stack, the redo edits while on the redo
    /// stack. Always in application order.
    edits: Vec<SceneEdit>,
    selection_before: Selection,
    selection_after: Selection,
}

impl UndoGroup {
    #[must_use]
    pub fn label(&self) -> &'static str {
        self.label
    }
}

/// Undo and redo stacks with a size limit.
///
/// Deleted nodes stay in the scene as detached subtrees (so undo can restore
/// them with their `NodeId`s). They are purged as soon as no remaining group
/// can bring them back: when the redo stack is discarded by a new edit, when the
/// oldest group falls off the limit, and on [`History::clear`].
#[derive(Debug)]
pub struct History {
    undo: Vec<UndoGroup>,
    redo: Vec<UndoGroup>,
    limit: usize,
    next_id: u64,
}

impl Default for History {
    fn default() -> Self {
        Self::with_limit(Self::DEFAULT_LIMIT)
    }
}

impl History {
    /// Number of undo steps kept by default.
    pub const DEFAULT_LIMIT: usize = 100;

    /// A history keeping at most `limit` undo steps (at least one).
    #[must_use]
    pub fn with_limit(limit: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            limit: limit.max(1),
            next_id: 1,
        }
    }

    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Label of the step [`History::undo`] would revert.
    #[must_use]
    pub fn undo_label(&self) -> Option<&'static str> {
        self.undo.last().map(UndoGroup::label)
    }

    /// Label of the step [`History::redo`] would re-apply.
    #[must_use]
    pub fn redo_label(&self) -> Option<&'static str> {
        self.redo.last().map(UndoGroup::label)
    }

    #[must_use]
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    #[must_use]
    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    /// Identifies the current document state: equal ids mean equal documents
    /// (as far as recorded edits go). `0` is the state before any recorded edit.
    /// Saving stores this to tell whether the document has unsaved changes.
    #[must_use]
    pub fn state_id(&self) -> u64 {
        self.undo.last().map_or(0, |g| g.id)
    }

    /// Records an already-applied step. `inverses` are the edits returned by
    /// [`Document::apply_all`] (undo order). An empty step is ignored.
    ///
    /// Discards the redo stack and trims the oldest steps beyond the limit,
    /// purging the detached nodes only those steps could have restored.
    pub fn record(
        &mut self,
        doc: &mut Document,
        label: &'static str,
        inverses: Vec<SceneEdit>,
        selection_before: Selection,
        selection_after: Selection,
    ) {
        if inverses.is_empty() {
            return;
        }
        for group in std::mem::take(&mut self.redo) {
            purge_unreachable(doc, &group);
        }
        self.undo.push(UndoGroup {
            id: self.next_id,
            label,
            edits: inverses,
            selection_before,
            selection_after,
        });
        self.next_id += 1;
        while self.undo.len() > self.limit {
            let oldest = self.undo.remove(0);
            purge_unreachable(doc, &oldest);
        }
    }

    /// Reverts the newest step and restores the selection it started from.
    /// Returns the step's label, or `None` if nothing was undone.
    pub fn undo(&mut self, doc: &mut Document, selection: &mut Selection) -> Option<&'static str> {
        let mut group = self.undo.pop()?;
        match doc.apply_all(std::mem::take(&mut group.edits)) {
            Ok(redo) => {
                group.edits = redo;
                selection.clone_from(&group.selection_before);
                let label = group.label;
                self.redo.push(group);
                Some(label)
            }
            Err(err) => {
                self.recover(doc, group.label, err);
                None
            }
        }
    }

    /// Re-applies the newest undone step and restores the selection it ended with.
    /// Returns the step's label, or `None` if nothing was redone.
    pub fn redo(&mut self, doc: &mut Document, selection: &mut Selection) -> Option<&'static str> {
        let mut group = self.redo.pop()?;
        match doc.apply_all(std::mem::take(&mut group.edits)) {
            Ok(undo) => {
                group.edits = undo;
                selection.clone_from(&group.selection_after);
                let label = group.label;
                self.undo.push(group);
                Some(label)
            }
            Err(err) => {
                self.recover(doc, group.label, err);
                None
            }
        }
    }

    /// Drops every step and purges all detached nodes (new or loaded document).
    pub fn clear(&mut self, doc: &mut Document) {
        self.undo.clear();
        self.redo.clear();
        doc.scene.purge_all_detached();
    }

    /// A step that no longer applies means the document was changed behind the
    /// history's back. The batch was rolled back, so the document is consistent,
    /// but the remaining steps cannot be trusted: start over.
    fn recover(&mut self, doc: &mut Document, label: &'static str, err: SceneError) {
        log::error!("Undo history out of sync at \"{label}\" ({err}); clearing the history");
        self.clear(doc);
    }
}

/// Purges the detached subtrees that `group` alone could re-attach.
///
/// A group that is dropped can only be the last reference to a detached node
/// through an `Attach` edit: any other step that could restore the node would
/// have to be older than the delete (dropped first by the limit) or newer
/// (on the redo stack, discarded together).
fn purge_unreachable(doc: &mut Document, group: &UndoGroup) {
    for edit in &group.edits {
        if let SceneEdit::Attach { root, .. } = *edit
            && doc.scene.detached_roots().contains(&root)
        {
            // Cannot fail: `root` was just checked to be a detached root.
            let _ = doc.scene.purge(root);
        }
    }
}
