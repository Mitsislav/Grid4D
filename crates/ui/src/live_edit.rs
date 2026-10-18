//! Turns live edits — values a panel writes every frame while the user drags
//! or types — into exactly one undo step per interaction.
//!
//! Usage in a panel, around the widgets that edit `value`:
//!
//! ```text
//! tracker.begin(key, &value);
//! …widgets write to value…
//! if let Some((key, before)) = tracker.end(key, &value, interaction_ongoing(ui)) {
//!     commands.push(EditorCommand::CommitLive { inverse: /* restore `before` */, label });
//! }
//! ```

/// `true` while the user is in the middle of an interaction: a mouse button is
/// held (dragging a value, a colour picker) or a text field has keyboard focus
/// (typing a number or a name). Clicks (checkboxes, buttons) end on release, so
/// they commit on the frame they change the value.
pub(crate) fn interaction_ongoing(ui: &egui::Ui) -> bool {
    ui.input(|i| i.pointer.any_down()) || ui.memory(|m| m.focused().is_some())
}

/// Remembers a value from before an interaction, keyed by what it belongs to
/// (a `NodeId`, or `()` for document settings).
#[derive(Debug)]
pub(crate) struct LiveEdit<K, T> {
    baseline: Option<(K, T)>,
    /// The value differs from the baseline because of an unfinished interaction.
    editing: bool,
}

impl<K, T> Default for LiveEdit<K, T> {
    fn default() -> Self {
        Self {
            baseline: None,
            editing: false,
        }
    }
}

impl<K: Copy + PartialEq, T: Clone + PartialEq> LiveEdit<K, T> {
    /// Call before the widgets that edit `value`. Outside an interaction, changes
    /// made elsewhere (the gizmo, undo, another panel) become the new baseline.
    /// Clones only when the value actually changed.
    pub(crate) fn begin(&mut self, key: K, value: &T) {
        if self.editing || self.matches(key, value) {
            return;
        }
        self.baseline = Some((key, value.clone()));
    }

    /// Call after the widgets. Once an interaction that changed the value has
    /// ended, returns the key and the value from before it, for the caller to
    /// record as an undo step; the current value becomes the new baseline.
    pub(crate) fn end(&mut self, key: K, value: &T, ongoing: bool) -> Option<(K, T)> {
        let changed = self.baseline.is_some() && !self.matches(key, value);
        if !changed {
            self.editing = false;
            return None;
        }
        if ongoing {
            self.editing = true;
            return None;
        }
        self.editing = false;
        self.baseline.replace((key, value.clone()))
    }

    fn matches(&self, key: K, value: &T) -> bool {
        self.baseline.as_ref().is_some_and(|(k, v)| *k == key && v == value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_commits_immediately() {
        let mut t = LiveEdit::<u8, i32>::default();
        t.begin(1, &10);
        assert_eq!(t.end(1, &11, false), Some((1, 10)));
        // The new value is the baseline: nothing more to commit.
        t.begin(1, &11);
        assert_eq!(t.end(1, &11, false), None);
    }

    #[test]
    fn a_drag_commits_once_on_release() {
        let mut t = LiveEdit::<u8, i32>::default();
        for value in [10, 12, 15, 20] {
            t.begin(1, &value);
            assert_eq!(t.end(1, &(value + 1), true), None, "still dragging");
        }
        t.begin(1, &21);
        assert_eq!(t.end(1, &21, false), Some((1, 10)), "one step, from the start value");
    }

    #[test]
    fn outside_changes_become_the_baseline() {
        let mut t = LiveEdit::<u8, i32>::default();
        t.begin(1, &10);
        assert_eq!(t.end(1, &10, false), None);
        // The gizmo (or an undo) changed the value between frames.
        t.begin(1, &50);
        assert_eq!(t.end(1, &50, false), None, "not this panel's edit");
        t.begin(1, &50);
        assert_eq!(t.end(1, &60, false), Some((1, 50)));
    }

    #[test]
    fn an_interaction_back_to_the_start_records_nothing() {
        let mut t = LiveEdit::<u8, i32>::default();
        t.begin(1, &10);
        assert_eq!(t.end(1, &15, true), None);
        t.begin(1, &15);
        assert_eq!(t.end(1, &10, false), None);
    }

    #[test]
    fn switching_targets_mid_edit_commits_for_the_old_target() {
        let mut t = LiveEdit::<u8, i32>::default();
        t.begin(1, &10);
        assert_eq!(t.end(1, &15, true), None);
        // The panel now shows another object before the interaction was seen to end.
        t.begin(2, &99);
        assert_eq!(t.end(2, &99, false), Some((1, 10)));
        t.begin(2, &99);
        assert_eq!(t.end(2, &99, false), None, "the new target is the baseline");
    }
}
