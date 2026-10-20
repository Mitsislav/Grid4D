//! Recording and editing keys of the selected objects: Record (F9), the
//! timeline's key strip (select, move, delete) and jumping between keys
//! (see `docs/reference/animation-recording.md`). The document edits are built
//! here and committed by `commands::apply`, one undo step each.

use grid4d_scene::{Document, Interpolation, NodeId, Param, Scene, SceneEdit, TangentPreset};

use crate::{AnimationSettings, CurveKey, EditorState, Selection};

/// Where edits of what is shown land: the current (whole) frame, and whether
/// changes to animated parameters become keys there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Keying {
    pub frame: f64,
    pub autokey: bool,
}

impl Keying {
    #[must_use]
    pub fn of(doc: &Document, editor: &EditorState) -> Self {
        Self {
            frame: f64::from(doc.time.frame()),
            autokey: editor.animation.autokey,
        }
    }
}

/// Fills `out` with the frames that hold a key of any selected object, sorted,
/// each once.
pub fn key_frames(scene: &Scene, selection: &Selection, out: &mut Vec<f64>) {
    out.clear();
    for &id in selection.as_slice() {
        if let Some(node) = scene.get(id) {
            for track in &node.data.tracks {
                out.extend(track.curve.keys().iter().map(|k| k.frame));
            }
        }
    }
    out.sort_by(f64::total_cmp);
    out.dedup();
}

/// The nearest whole frame with a key of a selected object after (`forward`)
/// or before `current`.
#[must_use]
pub fn adjacent_key(scene: &Scene, selection: &Selection, current: i32, forward: bool) -> Option<i32> {
    let mut frames = Vec::new();
    key_frames(scene, selection, &mut frames);
    let whole = frames.into_iter().filter_map(whole_frame);
    if forward {
        whole.filter(|&f| f > current).min()
    } else {
        whole.filter(|&f| f < current).max()
    }
}

/// `frame` rounded to a whole frame, if it is within the `i32` range.
#[must_use]
pub fn whole_frame(frame: f64) -> Option<i32> {
    let rounded = frame.round();
    #[expect(clippy::cast_possible_truncation, reason = "checked to be within the i32 range")]
    let whole = (rounded >= f64::from(i32::MIN) && rounded <= f64::from(i32::MAX)).then_some(rounded as i32);
    whole
}

/// Record Active Objects: keys at frame `at` for the enabled channels of every
/// selected object, with the values shown at the current frame.
#[must_use]
pub fn record(doc: &Document, selection: &Selection, settings: &AnimationSettings, at: f64) -> Vec<SceneEdit> {
    let current = f64::from(doc.time.frame());
    let params = Param::TRANSFORM.into_iter().filter(|&p| settings.records(p));
    edit_selected(&doc.scene, selection, |data| data.record(params.clone(), current, at))
}

/// Removes the keys at `frames` from every selected object; parameters left
/// without keys keep the values shown at the current frame.
#[must_use]
pub fn delete(doc: &Document, selection: &Selection, frames: &[f64]) -> Vec<SceneEdit> {
    let current = f64::from(doc.time.frame());
    edit_selected(&doc.scene, selection, |data| {
        data.remove_keys(|_, f| frames.contains(&f), current);
    })
}

/// Moves the keys at `frames` of every selected object by `by` frames.
#[must_use]
pub fn shift(doc: &Document, selection: &Selection, frames: &[f64], by: f64) -> Vec<SceneEdit> {
    edit_selected(&doc.scene, selection, |data| {
        data.move_keys(|f| frames.contains(&f), by)
    })
}

/// What the F-Curve editor's menu and Delete do to the selected keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurveOp {
    Delete,
    Interpolation(Interpolation),
    Tangents(TangentPreset),
}

impl CurveOp {
    /// The undo label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Delete => "Delete Keys",
            Self::Interpolation(_) => "Set Interpolation",
            Self::Tangents(_) => "Set Tangents",
        }
    }
}

/// `op` applied to `keys`: one `SetData` edit per object that changes.
/// Parameters left without keys keep the values shown at the current frame.
#[must_use]
pub fn curve_edits(doc: &Document, keys: &[CurveKey], op: CurveOp) -> Vec<SceneEdit> {
    let current = f64::from(doc.time.frame());
    let mut nodes: Vec<NodeId> = Vec::new();
    for key in keys {
        if !nodes.contains(&key.node) {
            nodes.push(key.node);
        }
    }
    nodes
        .into_iter()
        .filter_map(|id| {
            let before = &doc.scene.get(id)?.data;
            let mut data = before.clone();
            let selected = |param: Param, frame: f64| {
                keys.iter()
                    .any(|k| k.node == id && k.param == param && k.frame == frame)
            };
            match op {
                CurveOp::Delete => {
                    data.remove_keys(selected, current);
                }
                CurveOp::Interpolation(interpolation) => {
                    for key in keys.iter().filter(|k| k.node == id) {
                        if let Some(track) = data.tracks.iter_mut().find(|t| t.param == key.param) {
                            track.curve.set_interpolation(key.frame, interpolation);
                        }
                    }
                }
                CurveOp::Tangents(preset) => {
                    for key in keys.iter().filter(|k| k.node == id) {
                        if let Some(track) = data.tracks.iter_mut().find(|t| t.param == key.param) {
                            track.curve.apply_tangent_preset(key.frame, preset);
                        }
                    }
                }
            }
            (data != *before).then_some(SceneEdit::SetData { id, data })
        })
        .collect()
}

/// A `SetData` edit for every selected object that `change` actually changes.
fn edit_selected(
    scene: &Scene,
    selection: &Selection,
    mut change: impl FnMut(&mut grid4d_scene::NodeData),
) -> Vec<SceneEdit> {
    selection
        .as_slice()
        .iter()
        .filter_map(|&id: &NodeId| {
            let before = &scene.get(id)?.data;
            let mut data = before.clone();
            change(&mut data);
            (data != *before).then_some(SceneEdit::SetData { id, data })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use grid4d_scene::{Axis, Curve, Key, Node, ObjectKind, Track};

    use super::*;

    fn animated_doc() -> (Document, NodeId, NodeId) {
        let mut doc = Document::default();
        let mut a = Node::new("a", ObjectKind::Null);
        a.data.tracks.push(Track {
            param: Param::Position(Axis::X),
            curve: Curve::from_keys([Key::new(0.0, 0.0), Key::new(10.0, 100.0)]),
        });
        let mut b = Node::new("b", ObjectKind::Null);
        b.data.tracks.push(Track {
            param: Param::Scale(Axis::Y),
            curve: Curve::from_keys([Key::new(5.0, 2.0), Key::new(10.0, 3.0)]),
        });
        let (a, b) = (doc.scene.insert(a), doc.scene.insert(b));
        (doc, a, b)
    }

    #[test]
    fn key_frames_merge_the_selected_objects() {
        let (doc, a, b) = animated_doc();
        let mut selection = Selection::default();
        selection.set_all([a, b]);
        let mut frames = Vec::new();
        key_frames(&doc.scene, &selection, &mut frames);
        assert_eq!(frames, [0.0, 5.0, 10.0]);
        selection.set_single(b);
        key_frames(&doc.scene, &selection, &mut frames);
        assert_eq!(frames, [5.0, 10.0], "only the selected objects' keys");
        assert_eq!(adjacent_key(&doc.scene, &selection, 5, true), Some(10));
        assert_eq!(adjacent_key(&doc.scene, &selection, 5, false), None);
        assert_eq!(adjacent_key(&doc.scene, &selection, 7, false), Some(5));
    }

    #[test]
    fn recording_follows_the_channel_toggles() {
        let (mut doc, a, _) = animated_doc();
        doc.time.set_frame(4);
        let mut selection = Selection::default();
        selection.set_single(a);
        let settings = AnimationSettings {
            scale: false,
            rotation: false,
            ..AnimationSettings::default()
        };
        let edits = record(&doc, &selection, &settings, 20.0);
        let [SceneEdit::SetData { data, .. }] = edits.as_slice() else {
            panic!("one edit");
        };
        assert_eq!(data.tracks.len(), 3, "position X, Y, Z");
        let x = data
            .tracks
            .iter()
            .find(|t| t.param == Param::Position(Axis::X))
            .unwrap();
        assert_eq!(x.curve.keys().len(), 3);
        assert_eq!(
            x.curve.key_at(20.0).map(|k| k.value),
            doc.scene.get(a).unwrap().data.tracks[0].curve.value_at(4.0),
            "the value shown at the current frame, written at the target frame"
        );
        assert!(record(&doc, &Selection::default(), &settings, 0.0).is_empty());
    }
}
